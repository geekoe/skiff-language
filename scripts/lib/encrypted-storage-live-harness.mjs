import { createHash, randomBytes, randomInt } from 'node:crypto';
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { assertPortsClosed, leaseLocalPorts } from './local-port-lease.mjs';
import { runAttachedCommand } from './command-execution.mjs';
import { createMongoshCommand } from './mongosh-json-command.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const mongoshCommand = createMongoshCommand();
export const repoRoot = resolve(scriptDir, '..', '..');

const PORT_MIN = 45000;
const PORT_MAX = 45999;
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);
const EVENT_NAME = 'service_db.encryption_keyring_loaded';
const KEYRING_FORMAT = 'skiff-service-db-keyring-v1';

export function randomRootKey() {
  return randomBytes(32).toString('base64');
}

export function keyringFingerprint(keyring) {
  const parts = [
    Buffer.from('skiff-service-db-keyring-fingerprint-v1'),
    Buffer.from(keyring.format),
    Buffer.from(keyring.activeKeyId),
  ];
  for (const keyId of Object.keys(keyring.keys).sort((left, right) =>
    Buffer.from(left).compare(Buffer.from(right)))) {
    parts.push(Buffer.from(keyId));
    parts.push(Buffer.from(keyring.keys[keyId], 'base64'));
  }
  const hash = createHash('sha256');
  for (const part of parts) {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(part.length);
    hash.update(length);
    hash.update(part);
  }
  return hash.digest('hex');
}

export function makeKeyring(activeKeyId, keys) {
  return { format: KEYRING_FORMAT, activeKeyId, keys };
}

export class EncryptedStorageLiveHarness {
  static async create() {
    const portLease = await leaseIsolatedPorts();
    const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-encrypted-storage-live-'));
    const instanceRoot = join(tempRoot, 'instance');
    const configPath = join(instanceRoot, 'config.yml');
    const fixtureRoot = join(repoRoot, 'runtime', 'encrypted-storage-live');
    const paths = {
      tempRoot,
      instanceRoot,
      configPath,
      devHome: join(instanceRoot, 'dev-home'),
      artifactRoot: join(instanceRoot, 'dev-home', 'artifacts'),
      keyring: join(instanceRoot, 'dev-home', 'secrets', 'service-db-keyring.json'),
      runtimeLog: join(instanceRoot, 'logs', 'runtime.log'),
      runtimeErrorLog: join(instanceRoot, 'logs', 'runtime.err.log'),
      routerLog: join(instanceRoot, 'logs', 'router.log'),
      routerErrorLog: join(instanceRoot, 'logs', 'router.err.log'),
      fixtureRoot,
      packageStore: join(fixtureRoot, 'package-store'),
    };
    await mkdir(instanceRoot, { recursive: true });
    await writeFile(configPath, instanceConfigText(paths, portLease.ports), 'utf8');
    return new EncryptedStorageLiveHarness(paths, portLease);
  }

  constructor(paths, portLease) {
    this.paths = paths;
    this.portLease = portLease;
    this.ports = portLease.ports;
    this.routerHttpUrl = `http://127.0.0.1:${this.ports.base}`;
    this.routerReloadUrl = `http://127.0.0.1:${this.ports.base + 1}/__skiff/reload-artifacts`;
    this.mongoUrl = `mongodb://127.0.0.1:${this.ports.mongo}/?directConnection=true&replicaSet=rs0&retryWrites=false`;
    this.instanceInitialized = false;
    this.retirementGateActive = false;
    this.currentKeyring = undefined;
    this.cleaned = false;
    this.cleanupFallbackUsed = false;
    this.cleanupFallbackGroups = [];
  }

  async initialize(keyring) {
    await this.runSkiff(['instance', 'init', this.paths.configPath]);
    this.instanceInitialized = true;
    await this.writeKeyring(keyring);
    await this.seedServiceArtifacts('default-service');
    await this.seedServiceArtifacts('mapped-service');
    try {
      await this.runSkiff(['instance', 'up', this.paths.configPath]);
    } catch (error) {
      const routerLogs = await this.readLogs([this.paths.routerLog, this.paths.routerErrorLog]);
      throw new Error(`${error.message}\nrouter startup logs:\n${routerLogs}`, { cause: error });
    }
    await this.initializeReplicaSet();
    await this.assertRuntimeKeyringEvent(keyring);
  }

  async syncService(relativeRoot) {
    await this.runSkiff([
      'instance',
      'sync',
      this.paths.configPath,
      join(this.paths.fixtureRoot, relativeRoot),
    ]);
  }

  async seedServiceArtifacts(relativeRoot) {
    await runCommand(
      'node',
      [
        'scripts/skiff-dev-sync.mjs',
        '--root',
        join(this.paths.fixtureRoot, relativeRoot),
        '--artifact-root',
        this.paths.artifactRoot,
        '--build-root',
        join(this.paths.devHome, 'build'),
        '--default-packages-dir',
        this.paths.packageStore,
        '--no-reload',
      ],
      { cwd: repoRoot },
    );
  }

  async runLiveTestRunner(testFile, config) {
    const configPath = join(this.paths.tempRoot, 'test-runner-live.json');
    await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, 'utf8');
    const databasesBefore = new Set(await this.databaseNames());
    const run = runCommand(
      'cargo',
      [
        'run',
        '--manifest-path',
        'test-runner/Cargo.toml',
        '--',
        testFile,
        '--live',
        '--allow-network',
        '--config',
        configPath,
      ],
      {
        cwd: repoRoot,
        env: {
          ...process.env,
          SKIFF_DEV_RELOAD_URL: this.routerReloadUrl,
          SKIFF_TEST_ARTIFACT_ROOT: this.paths.artifactRoot,
          SKIFF_TEST_SYNC_CLEANUP: '1',
          SKIFF_TEST_DB_CLEANUP_SETTLE_MS: '0',
        },
      },
    );
    const observation = this.observeTransientEncryptedStorage(databasesBefore);
    const [, storage] = await Promise.all([run, observation]);
    let droppedBy = 'test-runner';
    if (await this.databaseExists(storage.database)) {
      await this.dropDatabase(storage.database);
      droppedBy = 'live-harness';
    }
    if (await this.databaseExists(storage.database)) {
      throw new Error(`failed to retire transient test-runner database ${storage.database}`);
    }
    return { ...storage, dropped: true, droppedBy };
  }

  async restartRuntime(keyring, { retirementAuthorized = false } = {}) {
    if (
      this.retirementGateActive
      && this.currentKeyring !== undefined
      && removesExistingKey(this.currentKeyring, keyring)
      && !retirementAuthorized
    ) {
      throw new Error('key removal requires the rotation cohort retirement API');
    }
    await this.writeKeyring(keyring);
    await this.runSkiff(['instance', 'restart', this.paths.configPath, 'runtime']);
    await this.assertRuntimeKeyringEvent(keyring);
  }

  async writeKeyring(keyring) {
    await mkdir(dirname(this.paths.keyring), { recursive: true });
    await writeFile(this.paths.keyring, `${JSON.stringify(keyring, null, 2)}\n`, {
      mode: 0o600,
    });
    await chmod(this.paths.keyring, 0o600);
    this.currentKeyring = structuredClone(keyring);
  }

  requireRetirementGate() {
    this.retirementGateActive = true;
  }

  async readKeyring() {
    return JSON.parse(await readFile(this.paths.keyring, 'utf8'));
  }

  async request(service, path, body, { expectFailure = false, rotationToken } = {}) {
    const url = new URL(path, this.routerHttpUrl);
    url.searchParams.set('service', service);
    url.searchParams.set('version', '0.1.0');
    let lastResponse;
    for (let attempt = 0; attempt < 30; attempt += 1) {
      try {
        const response = await fetch(url, {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            'x-skiff-service': service,
            'x-skiff-version': '0.1.0',
            ...(rotationToken === undefined ? {} : { 'x-skiff-rotation-token': rotationToken }),
          },
          body: JSON.stringify(body),
        });
        const text = await response.text();
        lastResponse = { status: response.status, text };
        if (expectFailure) {
          if (response.ok) {
            throw new Error(`expected ${service}${path} to fail, got HTTP ${response.status}`);
          }
          return lastResponse;
        }
        if (response.ok) {
          return text ? JSON.parse(text) : null;
        }
        if (![404, 502, 503, 504].includes(response.status)) {
          throw new Error(`${service}${path} returned HTTP ${response.status}: ${text}`);
        }
      } catch (error) {
        if (expectFailure && lastResponse !== undefined) {
          return lastResponse;
        }
        if (attempt === 29) {
          throw error;
        }
      }
      await delay(200);
    }
    throw new Error(`${service}${path} did not become ready: ${lastResponse?.text ?? 'no response'}`);
  }

  async rawDocument(database, collection, id) {
    return this.mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).findOne({_id:${JSON.stringify(id)}})`,
    );
  }

  async rawDocuments(database, collection) {
    return this.mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).find({}).sort({_id:1}).toArray()`,
    );
  }

  async collectionNames(database) {
    return this.mongoJson(database, 'db.getCollectionNames().sort()');
  }

  async databaseNames() {
    return this.mongoJson(
      'admin',
      'db.adminCommand({listDatabases:1,nameOnly:true}).databases.map((entry)=>entry.name).sort()',
    );
  }

  async databaseExists(database) {
    return (await this.databaseNames()).includes(database);
  }

  async dropDatabase(database) {
    return this.mongoJson(database, 'db.dropDatabase()');
  }

  async replaceRawDocument(database, collection, id, document) {
    const serialized = JSON.stringify(document);
    return this.mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).replaceOne({_id:${JSON.stringify(id)}}, EJSON.parse(${JSON.stringify(serialized)}))`,
    );
  }

  async setRawFields(database, collection, id, fields) {
    const serialized = JSON.stringify(fields);
    return this.mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).updateOne({_id:${JSON.stringify(id)}}, {$set:EJSON.parse(${JSON.stringify(serialized)})})`,
    );
  }

  async countNotKeyId(database, collection, field, keyId) {
    return this.mongoJson(
      database,
      `db.getCollection(${JSON.stringify(collection)}).countDocuments({${JSON.stringify(`${field}._skiff_encrypted.keyId`)}:{$ne:${JSON.stringify(keyId)}}})`,
    );
  }

  async mongoJson(database, expression) {
    return mongoshCommand.json({
      url: `mongodb://127.0.0.1:${this.ports.mongo}/${database}?directConnection=true`,
      expression,
      cwd: repoRoot,
    });
  }

  async assertRuntimeKeyringEvent(keyring) {
    const expectedFingerprint = keyringFingerprint(keyring);
    for (let attempt = 0; attempt < 50; attempt += 1) {
      const logs = await this.runtimeLogs();
      const matching = logs
        .split(/\r?\n/)
        .filter((line) => line.includes(EVENT_NAME))
        .filter((line) => line.includes(expectedFingerprint))
        .filter((line) => line.includes(keyring.activeKeyId));
      if (matching.length > 0) {
        return { fingerprint: expectedFingerprint, line: matching.at(-1) };
      }
      await delay(100);
    }
    throw new Error(`runtime did not emit ${EVENT_NAME} for ${keyring.activeKeyId}/${expectedFingerprint}`);
  }

  async observeTransientEncryptedStorage(databasesBefore) {
    for (let attempt = 0; attempt < 3000; attempt += 1) {
      const databases = await this.databaseNames();
      for (const database of databases) {
        if (databasesBefore.has(database) || ['admin', 'config', 'local'].includes(database)) {
          continue;
        }
        const collections = await this.collectionNames(database);
        for (const collection of collections) {
          const documents = await this.rawDocuments(database, collection);
          const fields = encryptedEnvelopeFields(documents);
          if (fields.length > 0) {
            return {
              storageServiceId: storageServiceIdFromDatabase(database),
              database,
              collection,
              fields,
              keyIds: encryptedEnvelopeKeyIds(documents, fields),
              rawSnapshot: JSON.stringify(documents),
            };
          }
        }
      }
      await delay(50);
    }
    throw new Error('did not observe transient test-runner encrypted storage');
  }

  async cleanup({ forceFallbackForTest = false } = {}) {
    if (this.cleaned) {
      return;
    }
    if (this.instanceInitialized) {
      let downError;
      try {
        if (forceFallbackForTest) {
          throw new Error('simulated instance down failure for cleanup fallback coverage');
        }
        await this.runSkiff(['instance', 'down', this.paths.configPath]);
      } catch (error) {
        downError = error;
      }
      if (downError !== undefined) {
        try {
          await this.stopOwnedProcessGroups();
          this.cleanupFallbackUsed = true;
        } catch (fallbackError) {
          throw new Error(
            `instance cleanup failed; preserving ${this.paths.tempRoot}: ${downError.message}; fallback: ${fallbackError.message}`,
            { cause: fallbackError },
          );
        }
      }
      try {
        await assertPortsClosed([
          this.ports.base,
          this.ports.base + 1,
          this.ports.base + 2,
          this.ports.mongo,
        ]);
      } catch (error) {
        throw new Error(
          `instance cleanup left listeners; preserving ${this.paths.tempRoot}: ${error.message}`,
          { cause: error },
        );
      }
    }
    await this.portLease.release();
    await rm(this.paths.tempRoot, { recursive: true, force: true });
    this.cleaned = true;
  }

  async stopOwnedProcessGroups() {
    let entries;
    try {
      entries = await readdir(join(this.paths.instanceRoot, 'pids'));
    } catch (error) {
      if (error.code === 'ENOENT') {
        return;
      }
      throw error;
    }
    const groups = [];
    for (const entry of entries) {
      if (!entry.endsWith('.pid')) {
        continue;
      }
      const raw = await readFile(join(this.paths.instanceRoot, 'pids', entry), 'utf8');
      const metadata = JSON.parse(raw);
      if (
        metadata.configPath !== this.paths.configPath
        || metadata.instanceRoot !== this.paths.instanceRoot
        || !Number.isInteger(metadata.pgid)
        || metadata.pgid <= 1
      ) {
        throw new Error(`refusing to stop unowned process metadata ${entry}`);
      }
      groups.push(metadata.pgid);
    }
    this.cleanupFallbackGroups = [...groups];
    for (const pgid of groups) {
      signalProcessGroup(pgid, 'SIGTERM');
    }
    await waitForProcessGroups(groups, 40);
    for (const pgid of groups.filter(processGroupAlive)) {
      signalProcessGroup(pgid, 'SIGKILL');
    }
    await waitForProcessGroups(groups, 20);
    const survivors = groups.filter(processGroupAlive);
    if (survivors.length > 0) {
      throw new Error(`owned process groups did not stop: ${survivors.join(', ')}`);
    }
  }

  async runSkiff(args) {
    await runCommand('node', ['scripts/skiff.mjs', ...args], { cwd: repoRoot });
  }

  async initializeReplicaSet() {
    const initiate = `try { rs.status(); } catch (error) { rs.initiate({_id:'rs0',members:[{_id:0,host:'127.0.0.1:${this.ports.mongo}'}]}); }`;
    await mongoshCommand.run(
      [`mongodb://127.0.0.1:${this.ports.mongo}/admin?directConnection=true`, '--quiet', '--eval', initiate],
      { cwd: repoRoot },
    );
    for (let attempt = 0; attempt < 60; attempt += 1) {
      try {
        const writable = await this.mongoJson('admin', 'db.hello().isWritablePrimary === true');
        if (writable) {
          return;
        }
      } catch {
        // Replica initialization briefly closes connections.
      }
      await delay(250);
    }
    throw new Error('managed Mongo replica set did not become PRIMARY');
  }

  async runtimeLogs() {
    return this.readLogs([this.paths.runtimeLog, this.paths.runtimeErrorLog]);
  }

  async readLogs(paths) {
    const parts = [];
    for (const path of paths) {
      try {
        parts.push(await readFile(path, 'utf8'));
      } catch (error) {
        if (error.code !== 'ENOENT') {
          throw error;
        }
      }
    }
    return parts.join('\n');
  }
}

async function leaseIsolatedPorts() {
  for (let attempt = 0; attempt < 500; attempt += 1) {
    const base = 45000 + randomInt(0, 400);
    const mongo = 45500 + randomInt(0, 500);
    const candidates = [base, base + 1, base + 2, mongo];
    if (new Set(candidates).size !== candidates.length || candidates.some(isForbiddenPort)) {
      continue;
    }
    try {
      const lease = await leaseLocalPorts(candidates);
      return {
        ports: { base, mongo },
        release: () => lease.release(),
      };
    } catch {
      // Try another disjoint port set.
    }
  }
  throw new Error(`no isolated ports available in ${PORT_MIN}-${PORT_MAX}`);
}

function isForbiddenPort(port) {
  return port < PORT_MIN || port > PORT_MAX || FORBIDDEN_PORTS.has(port);
}

function instanceConfigText(paths, ports) {
  return [
    'devHome: dev-home',
    `cargoTargetDir: ${JSON.stringify(join(repoRoot, 'build', 'cargo-target'))}`,
    'packageDirs:',
    `  - ${JSON.stringify(paths.packageStore)}`,
    'ports:',
    `  base: ${ports.base}`,
    `  mongo: ${ports.mongo}`,
    'components:',
    '  telemetry: disabled',
    '  mongo: managed',
    '  watch: disabled',
    'telemetry:',
    '  memory: true',
    'mongo:',
    '  binary: mongod',
    '  dbPath: service-db',
    'watch:',
    '  config: watch.json',
    '',
  ].join('\n');
}

function runCommand(command, args, options) {
  return runAttachedCommand(command, args, options);
}

function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}

function removesExistingKey(currentKeyring, nextKeyring) {
  return Object.keys(currentKeyring.keys).some((keyId) => nextKeyring.keys[keyId] === undefined);
}

function encryptedEnvelopeFields(documents) {
  const fields = new Set();
  for (const document of documents) {
    for (const [field, value] of Object.entries(document)) {
      if (
        field !== '_id'
        && value !== null
        && typeof value === 'object'
        && value._skiff_encrypted !== undefined
      ) {
        fields.add(field);
      }
    }
  }
  return [...fields].sort();
}

function encryptedEnvelopeKeyIds(documents, fields) {
  const keyIds = new Set();
  for (const document of documents) {
    for (const field of fields) {
      const keyId = document[field]?._skiff_encrypted?.keyId;
      if (keyId !== undefined) {
        keyIds.add(keyId);
      }
    }
  }
  return [...keyIds].sort();
}

function storageServiceIdFromDatabase(database) {
  return database.replaceAll('~~', '/').replaceAll('~', '.');
}

function signalProcessGroup(pgid, signal) {
  try {
    process.kill(-pgid, signal);
  } catch (error) {
    if (error.code !== 'ESRCH') {
      throw error;
    }
  }
}

function processGroupAlive(pgid) {
  try {
    process.kill(-pgid, 0);
    return true;
  } catch (error) {
    return error.code !== 'ESRCH';
  }
}

async function waitForProcessGroups(groups, attempts) {
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (!groups.some(processGroupAlive)) {
      return;
    }
    await delay(100);
  }
}
