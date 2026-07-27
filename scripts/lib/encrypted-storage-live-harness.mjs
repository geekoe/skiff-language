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
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { assertPortsClosed, leaseLocalPorts } from './local-port-lease.mjs';
import {
  captureCheckedCommand,
  runAttachedCommand,
} from './command-execution.mjs';
import { isolatedInstanceOperations } from './isolated-test-runtime-instance.mjs';
import { createMongoshCommand } from './mongosh-json-command.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import {
  validatePackageServiceActivationReceipt,
} from './package-service-ecosystem-smoke-oracle.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const mongoshCommand = createMongoshCommand();
export const repoRoot = resolve(scriptDir, '..', '..');
const TARGET_ENVIRONMENT = 'dev';
const RUNTIME_ASSEMBLY_IDENTITY =
  /^skiff-runtime-assembly-v2:sha256:[0-9a-f]{64}$/;
const REQUIRED_PACKAGE_COORDINATES = new Set([
  'example.com/encrypted-live-default@0.1.0',
  'example.com/encrypted-live-mapped@0.1.0',
  'example.com/encrypted-live-store@1.0.0',
]);
const REQUIRED_SERVICE_IDS = new Set([
  'example.com/encrypted-live-default',
  'example.com/encrypted-live-mapped',
]);

export function encryptedStorageTestRunnerArgs({
  testFile,
  artifactRoot,
  baseAssembly,
  activationUrl,
  ingressUrl,
  environment,
  expectedGeneration,
}) {
  requiredAbsolutePath(testFile, 'encrypted-storage test file');
  requiredAbsolutePath(artifactRoot, 'encrypted-storage artifact root');
  if (!RUNTIME_ASSEMBLY_IDENTITY.test(baseAssembly ?? '')) {
    throw new Error('encrypted-storage base assembly must be canonical');
  }
  requiredActivationUrl(activationUrl);
  requiredIngressUrl(ingressUrl);
  if (environment !== TARGET_ENVIRONMENT) {
    throw new Error(`encrypted-storage target environment must be ${TARGET_ENVIRONMENT}`);
  }
  requiredGeneration(expectedGeneration, 'encrypted-storage expected generation');
  return [
    'run',
    '--locked',
    '--quiet',
    '--manifest-path',
    'test-runner/Cargo.toml',
    '--bin',
    'skiff-test-runner',
    '--',
    testFile,
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    repoRoot,
    '--base-assembly',
    baseAssembly,
    '--live',
    '--activation-url',
    activationUrl,
    '--ingress-url',
    ingressUrl,
    '--environment',
    environment,
    '--expected-generation',
    String(expectedGeneration),
    '--deny-skips',
    '--require-tests',
  ];
}

export function encryptedStorageBuildArgs({
  fixtureRoot,
  artifactRoot,
}) {
  requiredAbsolutePath(fixtureRoot, 'encrypted-storage fixture root');
  requiredAbsolutePath(artifactRoot, 'encrypted-storage artifact root');
  return [
    'scripts/skiff-dev-sync.mjs',
    '--root',
    join(
      fixtureRoot,
      'package-store',
      'example~com~~encrypted-live-store',
      '1.0.0',
    ),
    '--root',
    join(fixtureRoot, 'default-service'),
    '--root',
    join(fixtureRoot, 'mapped-service'),
    '--artifact-root',
    artifactRoot,
    '--environment',
    TARGET_ENVIRONMENT,
    '--build-only',
    '--json',
  ];
}

export function encryptedStorageProductionAssembly(receipt) {
  if (!isPlainObject(receipt?.runtimeAssemblyReceipt)) {
    throw new Error('runtime assembly receipt is missing');
  }
  const { runtimeAssemblyReceipt } = receipt;
  if (runtimeAssemblyReceipt.environment !== TARGET_ENVIRONMENT) {
    throw new Error(`runtime assembly receipt environment must be ${TARGET_ENVIRONMENT}`);
  }
  const assembly = runtimeAssemblyReceipt.assembly;
  if (!isPlainObject(assembly) || assembly.assemblyIdentity === undefined) {
    throw new Error('assembly identity is missing');
  }
  if (
    Object.keys(assembly).length !== 1
    || !RUNTIME_ASSEMBLY_IDENTITY.test(assembly.assemblyIdentity)
  ) {
    throw new Error('assembly identity is not canonical');
  }
  const packageCoordinates = exactStringSet(
    receipt.packageArtifactReceipts,
    (entry) => {
      const artifact = entry?.artifact;
      return typeof artifact?.packageId === 'string'
        && typeof artifact?.packageVersion === 'string'
        ? `${artifact.packageId}@${artifact.packageVersion}`
        : undefined;
    },
  );
  if (!setsEqual(packageCoordinates, REQUIRED_PACKAGE_COORDINATES)) {
    throw new Error('required package roots are incomplete');
  }
  const serviceIds = exactStringSet(
    receipt.serviceDeploymentReceipts,
    (entry) => entry?.deployment?.serviceId,
  );
  if (!setsEqual(serviceIds, REQUIRED_SERVICE_IDS)) {
    throw new Error('required service roots are incomplete');
  }
  return Object.freeze({ assemblyIdentity: assembly.assemblyIdentity });
}

export function encryptedStorageIngressRequest({
  ingressUrl,
  path,
  body,
  rotationToken,
}) {
  const ingress = requiredIngressUrl(ingressUrl);
  if (
    typeof path !== 'string'
    || !path.startsWith('/')
    || path.startsWith('//')
    || path.includes('?')
    || path.includes('#')
  ) {
    throw new Error('encrypted-storage ingress path must come from the manifest');
  }
  const url = new URL(path, ingress);
  return {
    url,
    options: {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(rotationToken === undefined
          ? {}
          : { 'x-skiff-rotation-token': rotationToken }),
      },
      body: JSON.stringify(body),
    },
  };
}

export async function runEncryptedStorageTestLifecycle({
  activationState,
  runTest,
  observeStorage,
  cleanupStorage,
  restoreProductionAssembly,
  readCommittedGeneration,
}) {
  assertActivationState(activationState);
  for (const [name, operation] of Object.entries({
    runTest,
    observeStorage,
    cleanupStorage,
    restoreProductionAssembly,
  })) {
    if (typeof operation !== 'function') {
      throw new Error(`encrypted-storage lifecycle requires ${name}`);
    }
  }
  const expectedGeneration = activationState.currentGeneration;
  const baseAssembly = activationState.productionAssembly.assemblyIdentity;
  const [testOutcome, observationOutcome] = await Promise.allSettled([
    Promise.resolve().then(() => runTest({ baseAssembly, expectedGeneration })),
    Promise.resolve().then(() => observeStorage()),
  ]);
  const failures = [];
  let testActivationCommitted = false;
  if (testOutcome.status === 'fulfilled') {
    activationState.currentGeneration += 1;
    testActivationCommitted = true;
  } else {
    failures.push(contextualError('test runner failed', testOutcome.reason));
    if (typeof readCommittedGeneration === 'function') {
      try {
        const committedGeneration = await readCommittedGeneration();
        requiredGeneration(
          committedGeneration,
          'encrypted-storage observed committed generation',
        );
        if (committedGeneration === expectedGeneration + 1) {
          activationState.currentGeneration = committedGeneration;
          testActivationCommitted = true;
        } else if (committedGeneration !== expectedGeneration) {
          failures.push(new Error(
            `encrypted-storage test generation is indeterminate: expected ${expectedGeneration} or ${expectedGeneration + 1}, observed ${committedGeneration}`,
          ));
        }
      } catch (error) {
        failures.push(contextualError(
          'failed to determine whether test activation committed',
          error,
        ));
      }
    }
  }

  let storage;
  if (observationOutcome.status === 'fulfilled') {
    try {
      storage = await cleanupStorage(observationOutcome.value);
    } catch (error) {
      failures.push(contextualError('transient storage cleanup failed', error));
    }
  } else {
    failures.push(contextualError(
      'transient storage observation failed',
      observationOutcome.reason,
    ));
  }

  if (testActivationCommitted) {
    try {
      await restoreProductionAssembly({
        assembly: activationState.productionAssembly,
        expectedGeneration: activationState.currentGeneration,
      });
      activationState.currentGeneration += 1;
    } catch (error) {
      failures.push(contextualError('production restore failed', error));
    }
  }
  if (failures.length > 0) {
    throw new AggregateError(
      failures,
      `encrypted-storage live test lifecycle failed: ${failures.map((error) => error.message).join('; ')}`,
    );
  }
  return { storage, currentGeneration: activationState.currentGeneration };
}

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
    };
    await mkdir(instanceRoot, { recursive: true });
    await writeFile(configPath, instanceConfigText(portLease.ports), 'utf8');
    return new EncryptedStorageLiveHarness(paths, portLease);
  }

  constructor(paths, portLease) {
    this.paths = paths;
    this.portLease = portLease;
    this.ports = portLease.ports;
    this.routerHttpUrl = `http://127.0.0.1:${this.ports.base}`;
    this.activationUrl =
      `http://127.0.0.1:${this.ports.base + 1}/__skiff/activate-assembly`;
    this.controlHealthUrl =
      `http://127.0.0.1:${this.ports.base + 1}/__router/health`;
    this.mongoUrl = `mongodb://127.0.0.1:${this.ports.mongo}/?directConnection=true&replicaSet=rs0&retryWrites=false`;
    this.activationState = {
      currentGeneration: 0,
      productionAssembly: undefined,
    };
    this.instanceOperations = isolatedInstanceOperations({
      skiffRoot: repoRoot,
      baseEnv: process.env,
    });
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
    this.activationState.productionAssembly = await this.buildProductionAssembly();
    try {
      await this.runSkiff([
        'instance',
        'restart',
        this.paths.configPath,
        'mongo',
      ]);
      await this.initializeReplicaSet();
      await this.instanceOperations.seedActivationState({
        mongoPort: this.ports.mongo,
        bootstrap: {
          environment: TARGET_ENVIRONMENT,
          bootstrap: {
            generation: this.activationState.currentGeneration,
            assembly: this.activationState.productionAssembly,
          },
        },
        signal: new AbortController().signal,
      });
      await this.runSkiff(['instance', 'up', this.paths.configPath]);
      await this.assertProductionAssemblyReady();
    } catch (error) {
      const routerLogs = await this.readLogs([this.paths.routerLog, this.paths.routerErrorLog]);
      throw new Error(
        `${error.message}\nrouter startup logs:\n${routerLogs}`,
        { cause: error },
      );
    }
    await this.assertRuntimeKeyringEvent(keyring);
  }

  async buildProductionAssembly() {
    const { stdout } = await captureCheckedCommand(
      'node',
      encryptedStorageBuildArgs({
        fixtureRoot: this.paths.fixtureRoot,
        artifactRoot: this.paths.artifactRoot,
      }),
      { cwd: repoRoot },
    );
    let receipt;
    try {
      receipt = JSON.parse(stdout);
    } catch (error) {
      throw new Error(`encrypted-storage build returned invalid JSON: ${error.message}`);
    }
    return encryptedStorageProductionAssembly(receipt);
  }

  async runLiveTestRunner(testFile) {
    const databasesBefore = new Set(await this.databaseNames());
    const result = await runEncryptedStorageTestLifecycle({
      activationState: this.activationState,
      runTest: ({ baseAssembly, expectedGeneration }) => runCommand(
        'cargo',
        encryptedStorageTestRunnerArgs({
          testFile,
          artifactRoot: this.paths.artifactRoot,
          baseAssembly,
          activationUrl: this.activationUrl,
          ingressUrl: this.routerHttpUrl,
          environment: TARGET_ENVIRONMENT,
          expectedGeneration,
        }),
        { cwd: repoRoot },
      ),
      observeStorage: () => this.observeTransientEncryptedStorage(databasesBefore),
      cleanupStorage: async (storage) => {
        let droppedBy = 'test-runner';
        if (await this.databaseExists(storage.database)) {
          await this.dropDatabase(storage.database);
          droppedBy = 'live-harness';
        }
        if (await this.databaseExists(storage.database)) {
          throw new Error(
            `failed to retire transient test-runner database ${storage.database}`,
          );
        }
        return { ...storage, dropped: true, droppedBy };
      },
      readCommittedGeneration: () => this.readCommittedGeneration(),
      restoreProductionAssembly: (input) => this.restoreProductionAssembly(input),
    });
    return result.storage;
  }

  async readCommittedGeneration() {
    const response = await fetch(this.controlHealthUrl);
    const text = await response.text();
    if (!response.ok) {
      throw new Error(
        `router health returned HTTP ${response.status}${text ? `: ${text}` : ''}`,
      );
    }
    let health;
    try {
      health = JSON.parse(text);
    } catch (error) {
      throw new Error(`router health returned invalid JSON: ${error.message}`);
    }
    const generation = health?.activeAssembly?.generation;
    if (
      health?.ok !== true
      || health?.pendingActivation !== null
      || health?.activeAssembly?.environment !== TARGET_ENVIRONMENT
      || !Number.isSafeInteger(generation)
      || generation < 0
    ) {
      throw new Error('router health did not expose one committed dev generation');
    }
    return generation;
  }

  async assertProductionAssemblyReady() {
    for (let attempt = 0; attempt < 1200; attempt += 1) {
      try {
        const response = await fetch(this.controlHealthUrl);
        if (response.ok) {
          const health = await response.json();
          const active = health?.activeAssembly;
          const matchingReplica = (health?.replicas ?? []).some((replica) =>
            replica?.connected === true
            && replica?.state === 'healthy'
            && replica?.environment === TARGET_ENVIRONMENT
            && replica?.generation === this.activationState.currentGeneration
            && replica?.assemblyIdentity
              === this.activationState.productionAssembly.assemblyIdentity);
          if (
            health?.ok === true
            && health?.pendingActivation === null
            && active?.environment === TARGET_ENVIRONMENT
            && active?.generation === this.activationState.currentGeneration
            && active?.assemblyIdentity
              === this.activationState.productionAssembly.assemblyIdentity
            && matchingReplica
          ) {
            return;
          }
        }
      } catch {
        // Router and Runtime may still be converging on the seeded generation.
      }
      await delay(100);
    }
    throw new Error('production RuntimeAssembly did not become ready at generation 0');
  }

  async restoreProductionAssembly({ assembly, expectedGeneration }) {
    const activation = await requestAssemblyActivation({
      activationUrl: this.activationUrl,
      expectedGeneration,
      environment: TARGET_ENVIRONMENT,
      assembly,
    });
    validatePackageServiceActivationReceipt(activation, {
      environment: TARGET_ENVIRONMENT,
      assemblyIdentity: assembly.assemblyIdentity,
      expectedGeneration,
    });
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
    const request = encryptedStorageIngressRequest({
      ingressUrl: this.routerHttpUrl,
      path,
      body,
      rotationToken,
    });
    let lastResponse;
    for (let attempt = 0; attempt < 30; attempt += 1) {
      try {
        const response = await fetch(request.url, request.options);
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

function instanceConfigText(ports) {
  return [
    `environment: ${TARGET_ENVIRONMENT}`,
    'devHome: dev-home',
    `cargoTargetDir: ${JSON.stringify(join(repoRoot, 'build', 'cargo-target'))}`,
    'ports:',
    `  base: ${ports.base}`,
    `  mongo: ${ports.mongo}`,
    'http:',
    '  maxRequestBytes: 67108864',
    '  maxResponseBytes: 8388608',
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

function requiredAbsolutePath(value, label) {
  if (typeof value !== 'string' || !isAbsolute(value)) {
    throw new Error(`${label} must be an absolute path`);
  }
}

function requiredUrl(value, label) {
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${label} must be an absolute URL`);
  }
  if (parsed.protocol !== 'http:' || parsed.username || parsed.password) {
    throw new Error(`${label} must be an unauthenticated http URL`);
  }
  return parsed;
}

function requiredActivationUrl(value) {
  const parsed = requiredUrl(value, 'encrypted-storage activation URL');
  if (
    parsed.pathname !== '/__skiff/activate-assembly'
    || parsed.search
    || parsed.hash
  ) {
    throw new Error(
      'encrypted-storage activation URL must point exactly to /__skiff/activate-assembly',
    );
  }
  return parsed;
}

function requiredIngressUrl(value) {
  const parsed = requiredUrl(value, 'encrypted-storage ingress URL');
  if (parsed.pathname !== '/' || parsed.search || parsed.hash) {
    throw new Error('encrypted-storage ingress URL must be an origin');
  }
  return parsed;
}

function requiredGeneration(value, label) {
  if (
    !Number.isSafeInteger(value)
    || Object.is(value, -0)
    || value < 0
    || value > Number.MAX_SAFE_INTEGER - 2
  ) {
    throw new Error(`${label} must be a non-negative safe generation`);
  }
}

function exactStringSet(values, select) {
  if (!Array.isArray(values)) {
    return undefined;
  }
  const result = new Set();
  for (const value of values) {
    const selected = select(value);
    if (
      typeof selected !== 'string'
      || selected.length === 0
      || result.has(selected)
    ) {
      return undefined;
    }
    result.add(selected);
  }
  return result;
}

function setsEqual(left, right) {
  return left instanceof Set
    && left.size === right.size
    && [...left].every((value) => right.has(value));
}

function assertActivationState(state) {
  if (!isPlainObject(state)) {
    throw new Error('encrypted-storage lifecycle requires caller-owned activation state');
  }
  requiredGeneration(
    state.currentGeneration,
    'encrypted-storage current generation',
  );
  const assembly = state.productionAssembly;
  if (
    !isPlainObject(assembly)
    || Object.keys(assembly).length !== 1
    || !RUNTIME_ASSEMBLY_IDENTITY.test(assembly.assemblyIdentity ?? '')
  ) {
    throw new Error(
      'encrypted-storage lifecycle requires a canonical production assembly',
    );
  }
}

function contextualError(label, error) {
  const cause = error instanceof Error ? error : new Error(String(error));
  return new Error(`${label}: ${cause.message}`, { cause });
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
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
