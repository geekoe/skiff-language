import { spawn } from 'node:child_process';
import { createHash, randomBytes, randomInt } from 'node:crypto';
import { createConnection, createServer } from 'node:net';
import {
  chmod,
  mkdir,
  mkdtemp,
  open,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
export const repoRoot = resolve(scriptDir, '..', '..');

const PORT_LEASE_DIR = join(tmpdir(), 'skiff-encrypted-storage-live-port-leases');
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
    this.writerBarrier = false;
    this.cleaned = false;
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
    await runCommand(
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
  }

  async restartRuntime(keyring) {
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
  }

  beginWriterBarrier() {
    this.writerBarrier = true;
  }

  async request(service, path, body, { expectFailure = false, rotation = false } = {}) {
    if (this.writerBarrier && isBusinessWritePath(path) && !rotation) {
      throw new Error(`writer barrier blocked ${service}${path}`);
    }
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
    const marker = '__SKIFF_ENCRYPTED_LIVE_EJSON__';
    const code = `const value=(${expression}); print(${JSON.stringify(marker)}+EJSON.stringify(value,{relaxed:false}));`;
    const result = await runCommandCapture(
      'mongosh',
      [
        `mongodb://127.0.0.1:${this.ports.mongo}/${database}?directConnection=true`,
        '--quiet',
        '--eval',
        code,
      ],
      { cwd: repoRoot },
    );
    const line = result.stdout.split(/\r?\n/).find((candidate) => candidate.startsWith(marker));
    if (line === undefined) {
      throw new Error(`mongosh result did not contain EJSON marker: ${result.stdout}${result.stderr}`);
    }
    return JSON.parse(line.slice(marker.length));
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

  async cleanup() {
    if (this.cleaned) {
      return;
    }
    this.cleaned = true;
    let cleanupError;
    if (this.instanceInitialized) {
      try {
        await this.runSkiff(['instance', 'down', this.paths.configPath]);
        await assertPortsClosed([
          this.ports.base,
          this.ports.base + 1,
          this.ports.base + 2,
          this.ports.mongo,
        ]);
      } catch (error) {
        cleanupError = error;
      }
    }
    await this.portLease.release();
    await rm(this.paths.tempRoot, { recursive: true, force: true });
    if (cleanupError !== undefined) {
      throw cleanupError;
    }
  }

  async runSkiff(args) {
    await runCommand('node', ['scripts/skiff.mjs', ...args], { cwd: repoRoot });
  }

  async initializeReplicaSet() {
    const initiate = `try { rs.status(); } catch (error) { rs.initiate({_id:'rs0',members:[{_id:0,host:'127.0.0.1:${this.ports.mongo}'}]}); }`;
    await runCommandCapture(
      'mongosh',
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
  await mkdir(PORT_LEASE_DIR, { recursive: true });
  for (let attempt = 0; attempt < 500; attempt += 1) {
    const base = 45000 + randomInt(0, 400);
    const mongo = 45500 + randomInt(0, 500);
    const candidates = [base, base + 1, base + 2, mongo];
    if (new Set(candidates).size !== candidates.length || candidates.some(isForbiddenPort)) {
      continue;
    }
    const handles = [];
    try {
      for (const port of candidates) {
        const handle = await open(join(PORT_LEASE_DIR, `${port}.lock`), 'wx');
        handles.push({ port, handle });
        await assertPortAvailable(port);
      }
      return {
        ports: { base, mongo },
        async release() {
          for (const { port, handle } of handles) {
            await handle.close();
            await rm(join(PORT_LEASE_DIR, `${port}.lock`), { force: true });
          }
        },
      };
    } catch {
      for (const { port, handle } of handles) {
        await handle.close();
        await rm(join(PORT_LEASE_DIR, `${port}.lock`), { force: true });
      }
    }
  }
  throw new Error(`no isolated ports available in ${PORT_MIN}-${PORT_MAX}`);
}

function isForbiddenPort(port) {
  return port < PORT_MIN || port > PORT_MAX || FORBIDDEN_PORTS.has(port);
}

function isBusinessWritePath(path) {
  return !path.endsWith('/read') && !path.endsWith('/project') && !path.endsWith('/scan');
}

function assertPortAvailable(port) {
  return new Promise((resolvePromise, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => {
      server.close(resolvePromise);
    });
  });
}

async function assertPortsClosed(ports) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const openPorts = [];
    for (const port of ports) {
      if (await canConnect(port)) {
        openPorts.push(port);
      }
    }
    if (openPorts.length === 0) {
      return;
    }
    await delay(100);
  }
  throw new Error(`managed instance left a listener on one of: ${ports.join(', ')}`);
}

function canConnect(port) {
  return new Promise((resolvePromise) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.setTimeout(100);
    socket.once('connect', () => {
      socket.destroy();
      resolvePromise(true);
    });
    socket.once('timeout', () => {
      socket.destroy();
      resolvePromise(false);
    });
    socket.once('error', () => resolvePromise(false));
  });
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
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { ...options, stdio: 'inherit' });
    child.once('error', reject);
    child.once('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise();
      } else {
        reject(new Error(`${command} ${args.join(' ')} exited with ${signal ?? code}`));
      }
    });
  });
}

function runCommandCapture(command, args, options) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.once('error', reject);
    child.once('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise({ stdout, stderr });
      } else {
        reject(new Error(`${command} ${args.join(' ')} exited with ${signal ?? code}: ${stderr || stdout}`));
      }
    });
  });
}

function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}
