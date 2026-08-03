#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { join } from 'node:path';

import {
  createRotationInventory,
  inventoryWithMissingField,
  inventoryWithMissingService,
  numberFromEjson,
  ROTATION_PAGE_SIZE,
  RotationCohort,
  SimulatedCheckpointCrash,
} from './lib/encrypted-storage-rotation-cohort.mjs';
import {
  encryptedStorageIngressRequest,
  EncryptedStorageLiveHarness,
  keyringFingerprint,
  makeKeyring,
  randomRootKey,
} from './lib/encrypted-storage-live-harness.mjs';

const usage =
  'usage: node scripts/check-db-encrypted-storage-live.mjs [--help]';
const STORAGE_ENVIRONMENT = 'dev';
const DEFAULT_SERVICE = 'example.com/encrypted-live-default';
const MAPPED_SERVICE = 'example.com/encrypted-live-mapped';
const MAPPED_PACKAGE = 'example.com/encrypted-live-store';
const DEFAULT_DATABASE = storageDatabaseName(STORAGE_ENVIRONMENT, DEFAULT_SERVICE);
const MAPPED_DATABASE = storageDatabaseName(STORAGE_ENVIRONMENT, MAPPED_SERVICE);
const DEFAULT_COLLECTION = storageCollectionName(DEFAULT_SERVICE, 'Credential');
const ARCHIVE_COLLECTION = storageCollectionName(DEFAULT_SERVICE, 'CredentialArchive');
const MAPPED_COLLECTION = storageCollectionName(MAPPED_PACKAGE, 'package_secret');
const MAPPED_SERVICE_COLLECTION = storageCollectionName(MAPPED_SERVICE, 'Credential');
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

async function run(rawArgs) {
  if (rawArgs.length === 1 && ['-h', '--help'].includes(rawArgs[0])) {
    console.log(usage);
    return;
  }
  if (rawArgs.length > 0) {
    throw new Error(`unknown option ${rawArgs[0]}\n${usage}`);
  }
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
  const inventory = createRotationInventory({
    defaultService: DEFAULT_SERVICE,
    mappedService: MAPPED_SERVICE,
    defaultDatabase: DEFAULT_DATABASE,
    mappedDatabase: MAPPED_DATABASE,
    defaultCollection: DEFAULT_COLLECTION,
    archiveCollection: ARCHIVE_COLLECTION,
    mappedCollection: MAPPED_COLLECTION,
    mappedServiceCollection: MAPPED_SERVICE_COLLECTION,
    defaultBase: DEFAULT_BASE,
    mappedBase: MAPPED_BASE,
    retiredStorage: testRunnerStorage,
  });
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
    () => firstAttempt.retireOldKey(inventoryWithMissingService(inventory, MAPPED_SERVICE), nextOnly),
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
  assert(crashed, 'rotation must simulate a committed batch transaction before checkpoint persistence');
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
  const resumedCursors = resumed.scanCursors(crashEntry);
  assert(resumedCursors.length >= 2, 'default rotation must cross at least two scan pages');
  assert(resumedCursors.some((cursor) => cursor !== ''), 'default rotation never issued a non-empty lastId scan');
  const batchRewrites = resumed.batchRewrites(crashEntry);
  assert(batchRewrites.length > 1, 'default rotation must issue more than one batch rewrite request');
  assert(
    batchRewrites.every((batch) => batch.rowCount > 1),
    'rotation must send multiple rows in each full batch rewrite request',
  );
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
  console.log('PASS raw Mongo: plaintext absent, nonce independent, field/collection/record/service AAD enforced');
  console.log('PASS keyring lifecycle: restart, wrong-root/delete fail closed, old-read/new-write, fingerprint events');
  console.log('PASS rotation cohort: string-id pages, transactional batches, barrier, tuple checkpoints, crash replay, inventory refusal, raw $ne=0, offline recovery');
  await liveHarness.cleanup({ forceFallbackForTest: true });
  assert(liveHarness.cleanupFallbackUsed, 'cleanup fallback path must be exercised');
  assert(liveHarness.cleanupFallbackGroups.length >= 3, 'cleanup fallback did not own all managed process groups');
  console.log(`PASS cleanup fallback: stopped PGIDs ${liveHarness.cleanupFallbackGroups.join(', ')} and removed isolated root`);
}

async function runOuterRuntimeLiveTest(keyring) {
  const plaintextSentinel = 'encrypted-live-test-runner-secret';
  const testFile = join(
    liveHarness.paths.fixtureRoot,
    'default-service',
    'internal',
    'encrypted.live.test.skiff',
  );
  const storage = await liveHarness.runLiveTestRunner(testFile);
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

  const paginationRows = Array.from({ length: 105 }, (_, index) => ({
    id: `credential-page-${String(index).padStart(3, '0')}`,
    apiKey: 'sk-live-page-api',
    refreshToken: 'sk-live-page-refresh',
    label: `page-${index}`,
    sequence: index % 2,
  }));
  await callDefault('/insert-bulk', { rows: paginationRows });
  plaintexts.push(paginationRows[0].apiKey, paginationRows[0].refreshToken);

  const serviceProbe = {
    id: main.id,
    apiKey: main.apiKey,
    label: 'mapped-service-context',
    sequence: 1,
  };
  await callMapped('/service-probe-insert', serviceProbe);
  plaintexts.push(serviceProbe.apiKey);

  const defaultCursor = 'credential-page-103';
  const defaultCursorRows = await callDefault('/scan', { lastId: defaultCursor });
  assert(
    defaultCursorRows[0].id === 'credential-page-104'
      && defaultCursorRows.every((row) => row.id > defaultCursor),
    'default scan must apply its non-empty string primary-key cursor',
  );
  const mappedCursorRows = await callMapped('/scan', { lastId: 'mapped-one' });
  assert(
    mappedCursorRows.length === 1 && mappedCursorRows[0].id === 'mapped-two',
    'mapped scan must apply its non-empty string primary-key cursor',
  );

  return {
    plaintexts,
    defaultRows,
    archiveRows,
    mappedRows,
    paginationRows,
    main,
    mappedOne,
    archiveOne,
    serviceProbe,
  };
}

async function assertPhysicalStorage(state, keyId) {
  const defaultCollections = await liveHarness.collectionNames(DEFAULT_DATABASE);
  const mappedCollections = await liveHarness.collectionNames(MAPPED_DATABASE);
  assert(defaultCollections.includes(DEFAULT_COLLECTION), 'default physical collection missing');
  assert(defaultCollections.includes(ARCHIVE_COLLECTION), 'second default physical collection missing');
  assert(mappedCollections.includes(MAPPED_COLLECTION), 'mapped final physical collection missing');
  assert(
    mappedCollections.includes(MAPPED_SERVICE_COLLECTION),
    'mapped service-owned Credential collection missing',
  );
  assert(!mappedCollections.includes('package_secret'), 'unmapped package collection must not be used');

  const documents = await liveHarness.rawDocuments(DEFAULT_DATABASE, DEFAULT_COLLECTION);
  const archiveDocuments = await liveHarness.rawDocuments(DEFAULT_DATABASE, ARCHIVE_COLLECTION);
  const mappedDocuments = await liveHarness.rawDocuments(MAPPED_DATABASE, MAPPED_COLLECTION);
  const mappedServiceDocuments = await liveHarness.rawDocuments(
    MAPPED_DATABASE,
    MAPPED_SERVICE_COLLECTION,
  );
  const raw = JSON.stringify([documents, archiveDocuments, mappedDocuments, mappedServiceDocuments]);
  assert(documents.length > ROTATION_PAGE_SIZE, 'default rotation fixture must span more than one scan page');
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
  for (const document of mappedServiceDocuments) {
    assertEnvelope(document.apiKey, keyId, 'mapped service-owned apiKey envelope');
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

  source = await rawDefault(state.main.id);
  const mappedServiceTarget = await liveHarness.rawDocument(
    MAPPED_DATABASE,
    MAPPED_SERVICE_COLLECTION,
    state.serviceProbe.id,
  );
  assert(source._id === mappedServiceTarget._id, 'service-only AAD probe must keep record id unchanged');
  assert(
    state.main.apiKey === state.serviceProbe.apiKey,
    'service-only AAD probe must keep the encrypted plaintext unchanged',
  );
  await liveHarness.setRawFields(MAPPED_DATABASE, MAPPED_SERVICE_COLLECTION, state.serviceProbe.id, {
    apiKey: source.apiKey,
  });
  await assertReadFailsClosed(
    MAPPED_SERVICE,
    `${MAPPED_BASE}/service-probe-read`,
    state.serviceProbe.id,
    state.plaintexts,
  );
  await callMapped('/service-probe-restore', state.serviceProbe);

  const mapped = await liveHarness.rawDocument(MAPPED_DATABASE, MAPPED_COLLECTION, state.mappedOne.id);
  await liveHarness.setRawFields(DEFAULT_DATABASE, DEFAULT_COLLECTION, state.main.id, {
    apiKey: mapped.token,
  });
  await assertReadFailsClosed(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.main.id, state.plaintexts);
  await callDefault('/update', state.main);
}

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
  await assertBusinessRead(
    MAPPED_SERVICE,
    `${MAPPED_BASE}/service-probe-read`,
    state.serviceProbe,
  );
  for (const index of [0, 99, 104]) {
    await assertBusinessRead(DEFAULT_SERVICE, `${DEFAULT_BASE}/read`, state.paginationRows[index]);
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
    const request = encryptedStorageIngressRequest({
      ingressUrl: liveHarness.routerHttpUrl,
      path: probe.path,
      body: probe.body,
    });
    const response = await fetch(request.url, request.options);
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

function assertKeyringUnchanged(before, after) {
  assert(
    keyringFingerprint(before) === keyringFingerprint(after),
    'rejected retirement attempt changed the runtime keyring',
  );
}

function storageDatabaseName(environment, serviceId) {
  return `skiff_${storageIdentityDigest(
    'skiff-service-db-storage-identity-v1',
    environment,
    serviceId,
  )}`;
}

function storageCollectionName(packageId, declaredCollectionIdentity) {
  const readable = String(declaredCollectionIdentity)
    .replace(/[^A-Za-z0-9_.-]/g, '_')
    .slice(0, 32);
  const digest = storageIdentityDigest(
    'skiff-package-collection-storage-identity-v2',
    packageId,
    declaredCollectionIdentity,
  ).slice(0, 12);
  return `${readable}_${digest}`;
}

function storageIdentityDigest(...parts) {
  const hash = createHash('sha256');
  for (const part of parts) {
    const value = Buffer.from(part, 'utf8');
    const length = Buffer.alloc(8);
    length.writeBigUInt64BE(BigInt(value.length));
    hash.update(length);
    hash.update(value);
  }
  return hash.digest('base64url');
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
  await run(process.argv.slice(2));
} catch (error) {
  console.error(`db encrypted storage live test failed: ${error?.stack || error}`);
  process.exitCode = 1;
} finally {
  await liveHarness?.cleanup();
}
