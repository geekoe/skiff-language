import { createHash, randomBytes } from 'node:crypto';
import {
  chmod,
  mkdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { dirname } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { assertPortsClosed } from './local-port-lease.mjs';
import {
  captureCheckedCommand,
  runAttachedCommand,
} from './command-execution.mjs';
import {
  ENCRYPTED_STORAGE_TARGET_ENVIRONMENT as TARGET_ENVIRONMENT,
  encryptedStorageBuildArgs,
  encryptedStorageIngressRequest,
  encryptedStorageProductionAssembly,
  encryptedStorageTestRunnerArgs,
  repoRoot,
  runEncryptedStorageTestLifecycle,
} from './encrypted-storage-live-contract.mjs';
import {
  createEncryptedStorageLiveInstanceResources,
  stopEncryptedStorageLiveOwnedProcessGroups,
} from './encrypted-storage-live-instance-resources.mjs';
import {
  createEncryptedStorageLiveMongoProbe,
} from './encrypted-storage-live-mongo-probe.mjs';
import { isolatedInstanceOperations } from './isolated-test-runtime-instance.mjs';
import { requestAssemblyActivation } from './package-service-authoring.mjs';
import {
  validatePackageServiceActivationReceipt,
} from './package-service-ecosystem-smoke-oracle.mjs';

export {
  encryptedStorageBuildArgs,
  encryptedStorageIngressRequest,
  encryptedStorageProductionAssembly,
  encryptedStorageTestRunnerArgs,
  repoRoot,
  runEncryptedStorageTestLifecycle,
};

const EVENT_NAME = 'service_db.encryption_keyring_loaded';
const KEYRING_FORMAT = 'skiff-service-db-keyring-v1';
const mongoProbes = new WeakMap();
const cleanupFallbacks = new WeakSet();

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
    const { paths, portLease } =
      await createEncryptedStorageLiveInstanceResources({
        repoRoot,
        environment: TARGET_ENVIRONMENT,
      });
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
    mongoProbes.set(this, createEncryptedStorageLiveMongoProbe({
      mongoPort: this.ports.mongo,
      cwd: repoRoot,
    }));
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
      runTest: ({
        baseAssembly,
        baseConfigSnapshot,
        expectedGeneration,
      }) => runCommand(
        'cargo',
        encryptedStorageTestRunnerArgs({
          testFile,
          artifactRoot: this.paths.artifactRoot,
          baseAssembly,
          baseConfigSnapshot,
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
      assembly: { assemblyIdentity: assembly.assemblyIdentity },
      configSnapshot: { snapshotId: assembly.configSnapshotId },
    });
    validatePackageServiceActivationReceipt(activation, {
      environment: TARGET_ENVIRONMENT,
      assemblyIdentity: assembly.assemblyIdentity,
      configSnapshotId: assembly.configSnapshotId,
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
    return mongoProbes.get(this).rawDocument(database, collection, id);
  }

  async rawDocuments(database, collection) {
    return mongoProbes.get(this).rawDocuments(database, collection);
  }

  async collectionNames(database) {
    return mongoProbes.get(this).collectionNames(database);
  }

  async databaseNames() {
    return mongoProbes.get(this).databaseNames();
  }

  async databaseExists(database) {
    return mongoProbes.get(this).databaseExists(database);
  }

  async dropDatabase(database) {
    return mongoProbes.get(this).dropDatabase(database);
  }

  async replaceRawDocument(database, collection, id, document) {
    return mongoProbes.get(this).replaceRawDocument(
      database,
      collection,
      id,
      document,
    );
  }

  async setRawFields(database, collection, id, fields) {
    return mongoProbes.get(this).setRawFields(
      database,
      collection,
      id,
      fields,
    );
  }

  async countNotKeyId(database, collection, field, keyId) {
    return mongoProbes.get(this).countNotKeyId(
      database,
      collection,
      field,
      keyId,
    );
  }

  async mongoJson(database, expression) {
    return mongoProbes.get(this).mongoJson(database, expression);
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
    return mongoProbes.get(this).observeTransientEncryptedStorage(databasesBefore);
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
        cleanupFallbacks.add(this);
        try {
          await this.stopOwnedProcessGroups();
        } catch (fallbackError) {
          throw new Error(
            `instance cleanup failed; preserving ${this.paths.tempRoot}: ${downError.message}; fallback: ${fallbackError.message}`,
            { cause: fallbackError },
          );
        } finally {
          cleanupFallbacks.delete(this);
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
    const recordsFallbackUsage = cleanupFallbacks.has(this);
    try {
      await stopEncryptedStorageLiveOwnedProcessGroups({
        instanceRoot: this.paths.instanceRoot,
        configPath: this.paths.configPath,
        onValidated: (groups) => {
          if (groups !== undefined) {
            this.cleanupFallbackGroups = [...groups];
          }
          if (recordsFallbackUsage) {
            this.cleanupFallbackUsed = true;
          }
        },
      });
    } catch (error) {
      if (recordsFallbackUsage) {
        this.cleanupFallbackUsed = false;
      }
      throw error;
    }
  }

  async runSkiff(args) {
    await runCommand('node', ['scripts/skiff.mjs', ...args], { cwd: repoRoot });
  }

  async initializeReplicaSet() {
    return mongoProbes.get(this).initializeReplicaSet();
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

function runCommand(command, args, options) {
  return runAttachedCommand(command, args, options);
}

function removesExistingKey(currentKeyring, nextKeyring) {
  return Object.keys(currentKeyring.keys).some((keyId) => nextKeyring.keys[keyId] === undefined);
}
