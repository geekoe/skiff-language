#!/usr/bin/env node

import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import {
  EncryptedStorageLiveHarness,
  keyringFingerprint,
  makeKeyring,
  randomRootKey,
} from './lib/encrypted-storage-live-harness.mjs';

const DEFAULT_SERVICE = 'example.com/encrypted-live-default';
const MAPPED_SERVICE = 'example.com/encrypted-live-mapped';
const DEFAULT_DATABASE = storageDatabaseName(DEFAULT_SERVICE);
const MAPPED_DATABASE = storageDatabaseName(MAPPED_SERVICE);
const DEFAULT_COLLECTION = 'Credential';
const ARCHIVE_COLLECTION = 'CredentialArchive';
const MAPPED_COLLECTION = 'mapped_package_secret';
const DEFAULT_BASE = '/encrypted-live/default';
const MAPPED_BASE = '/encrypted-live/mapped';

let liveHarness;
let signalCleanupStarted = false;

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => {
    if (signalCleanupStarted) {
      return;
    }
    signalCleanupStarted = true;
    void (async () => {
      try {
        await liveHarness?.cleanup();
      } finally {
        process.exit(signal === 'SIGINT' ? 130 : 143);
      }
    })();
  });
}

async function run() {
  liveHarness = await EncryptedStorageLiveHarness.create();
  const roots = {
    old: randomRootKey(),
    wrongOld: randomRootKey(),
    next: randomRootKey(),
    future: randomRootKey(),
  };
  const oldOnly = makeKeyring('old-v1', { 'old-v1': roots.old });
  const wrongOld = makeKeyring('old-v1', { 'old-v1': roots.wrongOld });
  const nextOnly = makeKeyring('next-v2', { 'next-v2': roots.next });
  const oldAndNext = makeKeyring('next-v2', {
    'old-v1': roots.old,
    'next-v2': roots.next,
  });
  const futureCohort = makeKeyring('future-v3', {
    'next-v2': roots.next,
    'future-v3': roots.future,
  });

  console.log(`isolated instance root: ${liveHarness.paths.tempRoot}`);
  console.log(`isolated ports: router=${liveHarness.ports.base}/${liveHarness.ports.base + 1} mongo=${liveHarness.ports.mongo}`);
  assertPortsInAllowedRange(liveHarness.ports);

  await liveHarness.initialize(oldOnly);
  const testRunnerStorage = await runOuterRuntimeLiveTest(oldOnly);

  const state = await exerciseOperationMatrix();
  state.plaintexts.push(testRunnerStorage.plaintextSentinel);
  await assertPhysicalStorage(state, 'old-v1');
  await assertCrossContextCopyFails(state);

  await liveHarness.restartRuntime(oldOnly);
  await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main);
  await assertBusinessRead(MAPPED_SERVICE, `${MAPPED_BASE}/read`, state.mappedOne);

  await liveHarness.restartRuntime(wrongOld);
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main.id, state.plaintexts);
  await liveHarness.restartRuntime(oldOnly);
  await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main);

  await liveHarness.restartRuntime(nextOnly);
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main.id, state.plaintexts);

  await liveHarness.restartRuntime(oldAndNext);
  await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main);
  await assertBusinessRead(MAPPED_SERVICE, `${MAPPED_BASE}/read`, state.mappedOne);
  const nextRow = {
    id: 'credential-next-key',
    apiKey: 'sk-live-next-api',
    refreshToken: 'sk-live-next-refresh',
    label: 'next-key',
    sequence: 6,
  };
  await callDefault('/insert', nextRow);
  state.defaultRows.set(nextRow.id, nextRow);
  state.plaintexts.push(nextRow.apiKey, nextRow.refreshToken);
  const nextRaw = await rawDefault(nextRow.id);
  assertEnvelope(nextRaw.apiKey, 'next-v2', 'new writes must use active key');
  assertEnvelope(nextRaw.refreshToken, 'next-v2', 'all encrypted fields must use active key');

  const oldBackupDocument = await rawDefault(state.main.id);
  const inventory = rotationInventory(testRunnerStorage);
  const checkpointPath = join(liveHarness.paths.tempRoot, 'rotation-checkpoints.json');
  const targetFingerprint = keyringFingerprint(oldAndNext);
  const barrierToken = randomRootKey();
  const firstAttempt = await RotationCohort.create({
    harness: liveHarness,
    expectedInventory: inventory,
    inventory,
    targetFingerprint,
    checkpointPath,
    barrierToken,
  });
  await firstAttempt.beginWriteBarrier();
  await assertDirectFetchWriterBarrier(state);
  const keyringBeforeRefusals = await liveHarness.readKeyring();
  await assertRejects(
    () => firstAttempt.retireOldKey(inventoryWithMissingService(inventory), nextOnly),
    'missing service must block old-key retirement',
  );
  assertKeyringUnchanged(keyringBeforeRefusals, await liveHarness.readKeyring());
  await assertRejects(
    () => firstAttempt.retireOldKey(inventoryWithMissingField(inventory), nextOnly),
    'missing field must block old-key retirement',
  );
  assertKeyringUnchanged(keyringBeforeRefusals, await liveHarness.readKeyring());
  await assertRejects(
    () => firstAttempt.retireOldKey(inventory, nextOnly),
    'incomplete checkpoints must block old-key retirement',
  );
  assertKeyringUnchanged(keyringBeforeRefusals, await liveHarness.readKeyring());

  const crashEntry = inventory[0];
  let crashed = false;
  try {
    await firstAttempt.migrateCollection(crashEntry, { crashAfterFirstBatchBeforeCheckpoint: true });
  } catch (error) {
    crashed = error instanceof SimulatedCheckpointCrash;
    if (!crashed) {
      throw error;
    }
  }
  assert(crashed, 'rotation must simulate a write-success/checkpoint-before-write crash');
  assert(firstAttempt.checkpoint(crashEntry) === undefined, 'crashed batch must not advance checkpoint');
  const afterCrashedWrite = await rawDefault(state.main.id);

  const resumed = await RotationCohort.create({
    harness: liveHarness,
    expectedInventory: inventory,
    inventory,
    targetFingerprint,
    checkpointPath,
    barrierToken,
  });
  await resumed.beginWriteBarrier();
  await resumed.migrateCollection(crashEntry);
  const afterResume = await rawDefault(state.main.id);
  assert(
    envelopeCiphertext(afterCrashedWrite.apiKey) !== envelopeCiphertext(afterResume.apiKey),
    'replayed crash batch must safely rewrite with a fresh nonce',
  );
  for (const entry of inventory.slice(1)) {
    await resumed.migrateCollection(entry);
  }

  for (const entry of inventory) {
    for (const field of entry.fields) {
      const count = await liveHarness.countNotKeyId(
        entry.database,
        entry.collection,
        field,
        'next-v2',
      );
      assert(numberFromEjson(count) === 0, `${entry.storageServiceId}/${entry.collection}.${field} still has non-active envelopes`);
    }
  }

  const future = await RotationCohort.create({
    harness: liveHarness,
    expectedInventory: inventory,
    inventory,
    targetFingerprint: keyringFingerprint(futureCohort),
    checkpointPath,
    barrierToken,
  });
  for (const entry of inventory) {
    assert(future.checkpoint(entry) === undefined, 'a new target fingerprint must not reuse the previous cursor');
  }

  await resumed.retireOldKey(inventory, nextOnly);
  await assertAllBusinessRowsReadable(state);
  const onlineDocument = await rawDefault(state.main.id);

  await liveHarness.replaceRawDocument(
    DEFAULT_DATABASE,
    DEFAULT_COLLECTION,
    state.main.id,
    oldBackupDocument,
  );
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main.id, state.plaintexts);
  await liveHarness.restartRuntime(oldAndNext);
  await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main);
  await liveHarness.replaceRawDocument(
    DEFAULT_DATABASE,
    DEFAULT_COLLECTION,
    state.main.id,
    onlineDocument,
  );
  await resumed.retireOldKey(inventory, nextOnly);
  await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main);

  const logs = await liveHarness.readLogs([
    liveHarness.paths.runtimeLog,
    liveHarness.paths.runtimeErrorLog,
    liveHarness.paths.routerLog,
    liveHarness.paths.routerErrorLog,
  ]);
  for (const secret of [...state.plaintexts, barrierToken, ...Object.values(roots)]) {
    assert(!logs.includes(secret), 'runtime logs must not contain plaintext or root key material');
  }
  for (const eventLine of logs.split(/\r?\n/).filter((line) => line.includes('service_db.encryption_keyring_loaded'))) {
    assert(!eventLine.includes(liveHarness.paths.keyring), 'structured keyring event must not contain keyring path');
  }

  console.log('PASS operation matrix: insert/read/projection/insert-many/replace/upsert/key-update');
  console.log('PASS raw Mongo: plaintext absent, nonce independent, default+mapped collection AAD enforced');
  console.log('PASS keyring lifecycle: restart, wrong-root/delete fail closed, old-read/new-write, fingerprint events');
  console.log('PASS rotation cohort: barrier, tuple checkpoints, crash replay, inventory refusal, raw $ne=0, offline recovery');
  await liveHarness.cleanup({ forceFallbackForTest: true });
  assert(liveHarness.cleanupFallbackUsed, 'cleanup fallback path must be exercised');
  assert(liveHarness.cleanupFallbackGroups.length >= 3, 'cleanup fallback did not own all managed process groups');
  console.log(`PASS cleanup fallback: stopped PGIDs ${liveHarness.cleanupFallbackGroups.join(', ')} and removed isolated root`);
}

async function runOuterRuntimeLiveTest(keyring) {
  const plaintextSentinel = 'sk-live-test-runner-secret';
  const testFile = join(
    liveHarness.paths.fixtureRoot,
    'default-service',
    'internal',
    'encrypted.live.test.skiff',
  );
  const storage = await liveHarness.runLiveTestRunner(testFile, {
    encryptedLive: { testRunnerSecret: plaintextSentinel },
    serviceDb: { mongoUrl: liveHarness.mongoUrl },
  });
  assert(storage.fields.includes('secret'), 'test-runner encrypted field was not dynamically discovered');
  assert(
    storage.keyIds.length === 1 && storage.keyIds[0] === keyring.activeKeyId,
    'test-runner transient storage did not use the host runtime active key',
  );
  assert(!storage.rawSnapshot.includes(plaintextSentinel), 'test-runner raw Mongo snapshot leaked plaintext');
  assert(!(await liveHarness.databaseExists(storage.database)), 'retired test-runner database must be dropped');
  await liveHarness.assertRuntimeKeyringEvent(keyring);
  return { ...storage, plaintextSentinel };
}

async function exerciseOperationMatrix() {
  const plaintexts = [];
  const defaultRows = new Map();
  const archiveRows = new Map();
  const mappedRows = new Map();
  const main = {
    id: 'credential-main',
    apiKey: 'sk-live-insert-api',
    refreshToken: 'sk-live-insert-refresh',
    label: 'inserted',
    sequence: 1,
  };
  await callDefault('/insert', main);
  defaultRows.set(main.id, main);
  plaintexts.push(main.apiKey, main.refreshToken);
  await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, main);
  const projected = await callDefault('/project', { id: main.id });
  assert(
    JSON.stringify(Object.keys(projected).sort()) === JSON.stringify(['apiKey', 'id', 'refreshToken']),
    `encrypted projection returned the wrong shape: ${JSON.stringify(projected)}`,
  );
  assert(projected.id === main.id, 'encrypted projection must materialize primary key');
  assert(projected.apiKey === main.apiKey, 'encrypted projection must decrypt apiKey');
  assert(projected.refreshToken === main.refreshToken, 'encrypted projection must decrypt refreshToken');

  const many = {
    prefix: 'credential-many',
    apiKey: 'sk-live-many-api',
    refreshToken: 'sk-live-many-refresh',
    startSequence: 2,
  };
  await callDefault('/insert-many', many);
  for (const suffix of ['a', 'b']) {
    const row = {
      id: `${many.prefix}-${suffix}`,
      apiKey: many.apiKey,
      refreshToken: many.refreshToken,
      label: `many-${suffix}`,
      sequence: many.startSequence + (suffix === 'a' ? 0 : 1),
    };
    defaultRows.set(row.id, row);
  }
  plaintexts.push(many.apiKey, many.refreshToken);

  Object.assign(main, {
    apiKey: 'sk-live-replace-key-api',
    refreshToken: 'sk-live-replace-key-refresh',
    label: 'replace-key',
  });
  await callDefault('/replace-key', main);
  plaintexts.push(main.apiKey, main.refreshToken);

  const queryRow = {
    id: 'credential-query',
    apiKey: 'sk-live-query-insert-api',
    refreshToken: 'sk-live-query-insert-refresh',
    label: 'query-match',
    sequence: 4,
  };
  await callDefault('/insert', queryRow);
  plaintexts.push(queryRow.apiKey, queryRow.refreshToken);
  const queryReplacement = {
    matchLabel: queryRow.label,
    id: queryRow.id,
    apiKey: 'sk-live-query-replace-api',
    refreshToken: 'sk-live-query-replace-refresh',
    label: 'query-replaced',
    sequence: queryRow.sequence,
  };
  await callDefault('/replace-query', queryReplacement);
  Object.assign(queryRow, queryReplacement);
  delete queryRow.matchLabel;
  defaultRows.set(queryRow.id, queryRow);
  plaintexts.push(queryRow.apiKey, queryRow.refreshToken);

  const upsert = {
    id: 'credential-upsert',
    apiKey: 'sk-live-upsert-insert-api',
    refreshToken: 'sk-live-upsert-insert-refresh',
    label: 'upsert-insert',
    sequence: 5,
  };
  await callDefault('/upsert', upsert);
  plaintexts.push(upsert.apiKey, upsert.refreshToken);
  Object.assign(upsert, {
    apiKey: 'sk-live-upsert-change-api',
    refreshToken: 'sk-live-upsert-change-refresh',
    label: 'upsert-change',
  });
  await callDefault('/upsert', upsert);
  defaultRows.set(upsert.id, upsert);
  plaintexts.push(upsert.apiKey, upsert.refreshToken);

  Object.assign(main, {
    apiKey: 'sk-live-update-api',
    refreshToken: 'sk-live-update-refresh',
    label: 'updated',
  });
  await callDefault('/update', main);
  plaintexts.push(main.apiKey, main.refreshToken);

  const identityDate = await callDefault('/identity-date', {
    id: 'identity-date-smoke',
    note: 'identity-plain-smoke',
    recordedAt: '2026-07-10T12:34:56.000Z',
  });
  assert(identityDate.note === 'identity-plain-smoke', 'identity field smoke failed');
  assert(identityDate.recordedAt === '2026-07-10T12:34:56.000Z', 'Date projection smoke failed');

  const mappedOne = { id: 'mapped-one', token: 'sk-live-mapped-one', label: 'mapped-one', sequence: 1 };
  const mappedTwo = { id: 'mapped-two', token: 'sk-live-mapped-two', label: 'mapped-two', sequence: 2 };
  await callMapped('/insert', mappedOne);
  await callMapped('/insert', mappedTwo);
  mappedRows.set(mappedOne.id, mappedOne);
  mappedRows.set(mappedTwo.id, mappedTwo);
  plaintexts.push(mappedOne.token, mappedTwo.token);

  const archiveOne = {
    id: main.id,
    apiKey: main.apiKey,
    label: 'archive-main',
    sequence: 1,
  };
  await callDefault('/archive-insert', archiveOne);
  archiveRows.set(archiveOne.id, archiveOne);
  plaintexts.push(archiveOne.apiKey);

  const defaultCursorRows = await callDefault('/scan', { lastId: 4 });
  assert(
    defaultCursorRows.length === 1 && defaultCursorRows[0].sequence === 5,
    'default scan must apply its non-zero lastId cursor',
  );
  const mappedCursorRows = await callMapped('/scan', { lastId: 1 });
  assert(
    mappedCursorRows.length === 1 && mappedCursorRows[0].sequence === 2,
    'mapped scan must apply its non-zero lastId cursor',
  );

  return { plaintexts, defaultRows, archiveRows, mappedRows, main, mappedOne, archiveOne };
}

async function assertPhysicalStorage(state, keyId) {
  const defaultCollections = await liveHarness.collectionNames(DEFAULT_DATABASE);
  const mappedCollections = await liveHarness.collectionNames(MAPPED_DATABASE);
  assert(defaultCollections.includes(DEFAULT_COLLECTION), 'default physical collection missing');
  assert(defaultCollections.includes(ARCHIVE_COLLECTION), 'second default physical collection missing');
  assert(mappedCollections.includes(MAPPED_COLLECTION), 'mapped final physical collection missing');
  assert(!mappedCollections.includes('package_secret'), 'unmapped package collection must not be used');

  const documents = await liveHarness.rawDocuments(DEFAULT_DATABASE, DEFAULT_COLLECTION);
  const archiveDocuments = await liveHarness.rawDocuments(DEFAULT_DATABASE, ARCHIVE_COLLECTION);
  const mappedDocuments = await liveHarness.rawDocuments(MAPPED_DATABASE, MAPPED_COLLECTION);
  const raw = JSON.stringify([documents, archiveDocuments, mappedDocuments]);
  for (const plaintext of state.plaintexts) {
    assert(!raw.includes(plaintext), `raw Mongo contains plaintext sentinel ${plaintext}`);
  }
  for (const document of documents) {
    assertEnvelope(document.apiKey, keyId, 'default apiKey envelope');
    assertEnvelope(document.refreshToken, keyId, 'default refreshToken envelope');
  }
  for (const document of mappedDocuments) {
    assertEnvelope(document.token, keyId, 'mapped package token envelope');
  }
  for (const document of archiveDocuments) {
    assertEnvelope(document.apiKey, keyId, 'archive apiKey envelope');
  }
  const first = documents.find((document) => document._id === 'credential-many-a');
  const second = documents.find((document) => document._id === 'credential-many-b');
  assert(envelopeNonce(first.apiKey) !== envelopeNonce(second.apiKey), 'insert-many rows must use independent nonces');
  assert(envelopeCiphertext(first.apiKey) !== envelopeCiphertext(second.apiKey), 'insert-many rows must use independent ciphertext');
}

async function assertCrossContextCopyFails(state) {
  let source = await rawDefault(state.main.id);

  await liveHarness.setRawFields(DEFAULT_DATABASE, DEFAULT_COLLECTION, state.main.id, {
    refreshToken: source.apiKey,
  });
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main.id, state.plaintexts);
  await callDefault('/update', state.main);

  source = await rawDefault(state.main.id);
  await liveHarness.setRawFields(DEFAULT_DATABASE, ARCHIVE_COLLECTION, state.archiveOne.id, {
    apiKey: source.apiKey,
  });
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/archive-read`, state.archiveOne.id, state.plaintexts);
  await callDefault('/archive-restore', state.archiveOne);

  source = await rawDefault(state.main.id);
  const target = await rawDefault('credential-many-a');
  await liveHarness.setRawFields(DEFAULT_DATABASE, DEFAULT_COLLECTION, target._id, {
    apiKey: source.apiKey,
  });
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, target._id, state.plaintexts);
  await callDefault('/update', state.defaultRows.get(target._id));

  const mapped = await liveHarness.rawDocument(MAPPED_DATABASE, MAPPED_COLLECTION, state.mappedOne.id);
  await liveHarness.setRawFields(DEFAULT_DATABASE, DEFAULT_COLLECTION, state.main.id, {
    apiKey: mapped.token,
  });
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main.id, state.plaintexts);
  await callDefault('/update', state.main);
}

function rotationInventory(testRunnerStorage) {
  return [
    {
      storageServiceId: DEFAULT_SERVICE,
      database: DEFAULT_DATABASE,
      collection: DEFAULT_COLLECTION,
      fields: ['apiKey', 'refreshToken'],
      scanPath: `${DEFAULT_BASE}/scan`,
      rewritePath: `${DEFAULT_BASE}/rewrite`,
      service: DEFAULT_SERVICE,
      barrierPath: `${DEFAULT_BASE}/barrier`,
      barrierStatusPath: `${DEFAULT_BASE}/barrier-status`,
    },
    {
      storageServiceId: DEFAULT_SERVICE,
      database: DEFAULT_DATABASE,
      collection: ARCHIVE_COLLECTION,
      fields: ['apiKey'],
      scanPath: `${DEFAULT_BASE}/archive-scan`,
      rewritePath: `${DEFAULT_BASE}/archive-rewrite`,
      service: DEFAULT_SERVICE,
      barrierPath: `${DEFAULT_BASE}/barrier`,
      barrierStatusPath: `${DEFAULT_BASE}/barrier-status`,
    },
    {
      storageServiceId: MAPPED_SERVICE,
      database: MAPPED_DATABASE,
      collection: MAPPED_COLLECTION,
      fields: ['token'],
      scanPath: `${MAPPED_BASE}/scan`,
      rewritePath: `${MAPPED_BASE}/rewrite`,
      service: MAPPED_SERVICE,
      barrierPath: `${MAPPED_BASE}/barrier`,
      barrierStatusPath: `${MAPPED_BASE}/barrier-status`,
    },
    {
      storageServiceId: testRunnerStorage.storageServiceId,
      database: testRunnerStorage.database,
      collection: testRunnerStorage.collection,
      fields: [...testRunnerStorage.fields],
      retired: true,
    },
  ];
}

class RotationCohort {
  static async create(input) {
    let checkpoints = new Map();
    try {
      const stored = JSON.parse(await readFile(input.checkpointPath, 'utf8'));
      checkpoints = new Map(stored);
    } catch (error) {
      if (error.code !== 'ENOENT') {
        throw error;
      }
    }
    return new RotationCohort(input, checkpoints);
  }

  constructor(input, checkpoints) {
    Object.assign(this, input);
    this.checkpoints = checkpoints;
    this.writeBarrier = false;
  }

  async beginWriteBarrier() {
    for (const endpoint of barrierEndpoints(this.expectedInventory)) {
      const result = await this.harness.request(endpoint.service, endpoint.barrierPath, {
        token: this.barrierToken,
      });
      assert(result.active === true, `failed to activate writer barrier for ${endpoint.service}`);
    }
    this.writeBarrier = true;
    this.harness.requireRetirementGate();
    await this.assertRealBarrierActive();
  }

  checkpoint(entry) {
    return this.checkpoints.get(this.checkpointKey(entry));
  }

  assertRetirementInventory(candidate) {
    assert(this.writeBarrier, 'cohort retirement requires an active full-writer barrier');
    const expected = canonicalInventory(this.expectedInventory);
    const actual = canonicalInventory(candidate);
    assert(actual === expected, 'cohort inventory is incomplete; refusing old-key retirement');
  }

  async assertRealBarrierActive() {
    assert(this.writeBarrier, 'cohort requires an active full-writer barrier');
    for (const endpoint of barrierEndpoints(this.expectedInventory)) {
      const status = await this.harness.request(
        endpoint.service,
        endpoint.barrierStatusPath,
        {},
      );
      assert(status.active === true, `writer barrier is not active for ${endpoint.service}`);
    }
  }

  async migrateCollection(entry, { crashAfterFirstBatchBeforeCheckpoint = false } = {}) {
    assert(this.writeBarrier, 'rotation migration requires an active full-writer barrier');
    if (entry.retired) {
      assert(!(await this.harness.databaseExists(entry.database)), `${entry.database} must remain retired`);
      for (const field of entry.fields) {
        const count = await this.harness.countNotKeyId(
          entry.database,
          entry.collection,
          field,
          'next-v2',
        );
        assert(numberFromEjson(count) === 0, `retired ${entry.collection}.${field} is not empty`);
      }
      await this.saveCheckpoint(entry, { lastId: null, complete: true });
      return;
    }
    const cursor = this.checkpoint(entry)?.lastId ?? 0;
    const rows = await this.harness.request(entry.service, entry.scanPath, {
      lastId: cursor,
    });
    assert(
      rows.every((row) => row.sequence > cursor),
      `${entry.collection} scan did not honor lastId ${cursor}`,
    );
    const batchSize = 2;
    for (let offset = 0; offset < rows.length; offset += batchSize) {
      const batch = rows.slice(offset, offset + batchSize);
      for (const row of batch) {
        await this.harness.request(entry.service, entry.rewritePath, row, {
          rotationToken: this.barrierToken,
        });
      }
      if (crashAfterFirstBatchBeforeCheckpoint && offset === 0) {
        throw new SimulatedCheckpointCrash();
      }
      await this.saveCheckpoint(entry, { lastId: batch.at(-1).sequence, complete: false });
    }
    await this.saveCheckpoint(entry, {
      lastId: rows.at(-1)?.sequence ?? cursor,
      complete: true,
    });
  }

  async retireOldKey(candidateInventory, nextKeyring) {
    this.assertRetirementInventory(candidateInventory);
    await this.assertRealBarrierActive();
    for (const entry of this.expectedInventory) {
      const checkpoint = this.checkpoint(entry);
      assert(checkpoint?.complete === true, `checkpoint is incomplete for ${entry.storageServiceId}/${entry.collection}`);
      if (entry.retired) {
        assert(!(await this.harness.databaseExists(entry.database)), `${entry.database} unexpectedly returned`);
      }
      for (const field of entry.fields) {
        const count = await this.harness.countNotKeyId(
          entry.database,
          entry.collection,
          field,
          nextKeyring.activeKeyId,
        );
        assert(
          numberFromEjson(count) === 0,
          `${entry.storageServiceId}/${entry.collection}.${field} blocks old-key retirement`,
        );
      }
    }
    await this.harness.restartRuntime(nextKeyring, { retirementAuthorized: true });
  }

  async saveCheckpoint(entry, value) {
    this.checkpoints.set(this.checkpointKey(entry), value);
    await writeFile(this.checkpointPath, `${JSON.stringify([...this.checkpoints], null, 2)}\n`, 'utf8');
  }

  checkpointKey(entry) {
    return JSON.stringify([
      this.targetFingerprint,
      entry.storageServiceId,
      entry.collection,
    ]);
  }
}

class SimulatedCheckpointCrash extends Error {}

async function assertAllBusinessRowsReadable(state) {
  for (const row of state.defaultRows.values()) {
    await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, row);
  }
  for (const row of state.mappedRows.values()) {
    await assertBusinessRead(MAPPED_SERVICE, `${MAPPED_BASE}/read`, row);
  }
  for (const row of state.archiveRows.values()) {
    await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/archive-read`, row);
  }
}

async function assertBusinessRead(service, path, expected) {
  const actual = await liveHarness.request(service, path, { id: expected.id });
  for (const [key, value] of Object.entries(expected)) {
    assert(actual[key] === value, `${service}${path} field ${key} mismatch`);
  }
}

async function assertReadFailsClosed(service, path, id, plaintexts) {
  const response = await liveHarness.request(service, path, { id }, { expectFailure: true });
  assert(response.status >= 400, `${service}${path} must return an error`);
  for (const plaintext of plaintexts) {
    assert(!response.text.includes(plaintext), 'fail-closed response leaked plaintext');
  }
}

async function assertDirectFetchWriterBarrier(state) {
  const blockedSentinel = 'blocked-writer-secret';
  state.plaintexts.push(blockedSentinel);
  const cases = [
    {
      service: DEFAULT_SERVICE,
      path: `${DEFAULT_BASE}/update`,
      body: state.main,
    },
    {
      service: MAPPED_SERVICE,
      path: `${MAPPED_BASE}/insert`,
      body: { id: 'blocked-writer', token: blockedSentinel, label: 'blocked', sequence: 99 },
    },
  ];
  for (const probe of cases) {
    const url = new URL(probe.path, liveHarness.routerHttpUrl);
    url.searchParams.set('service', probe.service);
    url.searchParams.set('version', '0.1.0');
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-skiff-service': probe.service,
        'x-skiff-version': '0.1.0',
      },
      body: JSON.stringify(probe.body),
    });
    assert(response.status === 423, `direct writer bypass was not blocked for ${probe.service}`);
  }
}

function callDefault(path, body) {
  return liveHarness.request(DEFAULT_SERVICE, `${DEFAULT_BASE}${path}`, body);
}

function callMapped(path, body) {
  return liveHarness.request(MAPPED_SERVICE, `${MAPPED_BASE}${path}`, body);
}

function rawDefault(id) {
  return liveHarness.rawDocument(DEFAULT_DATABASE, DEFAULT_COLLECTION, id);
}

function assertEnvelope(value, keyId, message) {
  assert(value !== null && typeof value === 'object', `${message}: envelope missing`);
  const envelope = value._skiff_encrypted;
  assert(envelope !== undefined, `${message}: reserved envelope missing`);
  assert(envelope.keyId === keyId, `${message}: expected key ${keyId}, got ${envelope.keyId}`);
  assert(envelopeNonce(value).length > 0, `${message}: nonce missing`);
  assert(envelopeCiphertext(value).length > 0, `${message}: ciphertext missing`);
}

function envelopeNonce(value) {
  return value._skiff_encrypted.nonce.$binary.base64;
}

function envelopeCiphertext(value) {
  return value._skiff_encrypted.ciphertext.$binary.base64;
}

function numberFromEjson(value) {
  if (typeof value === 'number') {
    return value;
  }
  return Number(value.$numberInt ?? value.$numberLong);
}

function inventoryWithMissingField(inventory) {
  return inventory.map((entry, index) => ({
    ...entry,
    fields: index === 0 ? entry.fields.slice(0, 1) : [...entry.fields],
  }));
}

function inventoryWithMissingService(inventory) {
  return inventory.filter((entry) => entry.storageServiceId !== MAPPED_SERVICE);
}

function canonicalInventory(inventory) {
  return JSON.stringify(
    inventory
      .map((entry) => ({
        storageServiceId: entry.storageServiceId,
        collection: entry.collection,
        fields: [...entry.fields].sort(),
        retired: entry.retired === true,
      }))
      .sort((left, right) => `${left.storageServiceId}/${left.collection}`.localeCompare(`${right.storageServiceId}/${right.collection}`)),
  );
}

function barrierEndpoints(inventory) {
  const endpoints = new Map();
  for (const entry of inventory) {
    if (entry.retired || entry.barrierPath === undefined) {
      continue;
    }
    endpoints.set(`${entry.service}:${entry.barrierPath}`, {
      service: entry.service,
      barrierPath: entry.barrierPath,
      barrierStatusPath: entry.barrierStatusPath,
    });
  }
  return [...endpoints.values()];
}

function assertKeyringUnchanged(before, after) {
  assert(
    keyringFingerprint(before) === keyringFingerprint(after),
    'rejected retirement attempt changed the runtime keyring',
  );
}

function storageDatabaseName(serviceId) {
  return serviceId.replaceAll('.', '~').replaceAll('/', '~~');
}

function assertPortsInAllowedRange(ports) {
  for (const port of [ports.base, ports.base + 1, ports.base + 2, ports.mongo]) {
    assert(port >= 45000 && port <= 45999, `port ${port} escaped isolated range`);
    assert(!(port >= 44000 && port <= 44999), `port ${port} overlaps browser worktree range`);
    assert(!(port >= 4000 && port <= 4007), `port ${port} overlaps stable workspace range`);
    assert(port !== 27017, 'managed live test must not use stable Mongo');
  }
}

async function assertRejects(callback, message) {
  let rejected = false;
  try {
    await callback();
  } catch {
    rejected = true;
  }
  assert(rejected, message);
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

try {
  await run();
} catch (error) {
  console.error(`db encrypted storage live test failed: ${error?.stack || error}`);
  process.exitCode = 1;
} finally {
  await liveHarness?.cleanup();
}
