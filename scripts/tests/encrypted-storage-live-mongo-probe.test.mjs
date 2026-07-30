import assert from 'node:assert/strict';
import test from 'node:test';

import {
  createEncryptedStorageLiveMongoProbe,
} from '../lib/encrypted-storage-live-mongo-probe.mjs';

test('mongo probe preserves URL, expression, cwd, and read/write results', async () => {
  const calls = [];
  const canned = [
    { _id: 'credential-main', secret: 'ciphertext' },
    { acknowledged: true, modifiedCount: 1 },
    { acknowledged: true, modifiedCount: 1 },
    3,
  ];
  const probe = createEncryptedStorageLiveMongoProbe({
    mongoPort: 45777,
    cwd: '/repo/skiff',
    command: {
      async json(input) {
        calls.push(input);
        return canned.shift();
      },
      async run() {
        throw new Error('replica command was not expected');
      },
    },
  });

  assert.deepEqual(
    await probe.rawDocument('service-db', 'credentials', 'credential-main'),
    { _id: 'credential-main', secret: 'ciphertext' },
  );
  assert.deepEqual(
    await probe.replaceRawDocument(
      'service-db',
      'credentials',
      'credential-main',
      { _id: 'credential-main', count: { $numberLong: '2' } },
    ),
    { acknowledged: true, modifiedCount: 1 },
  );
  assert.deepEqual(
    await probe.setRawFields(
      'service-db',
      'credentials',
      'credential-main',
      { secret: { $binary: { base64: 'AA==', subType: '00' } } },
    ),
    { acknowledged: true, modifiedCount: 1 },
  );
  assert.equal(
    await probe.countNotKeyId(
      'service-db',
      'credentials',
      'secret',
      'key-next',
    ),
    3,
  );

  assert.deepEqual(
    calls.map(({ url, cwd }) => ({ url, cwd })),
    Array.from({ length: 4 }, () => ({
      url: 'mongodb://127.0.0.1:45777/service-db?directConnection=true',
      cwd: '/repo/skiff',
    })),
  );
  assert.equal(
    calls[0].expression,
    'db.getCollection("credentials").findOne({_id:"credential-main"})',
  );
  assert.equal(
    calls[1].expression,
    'db.getCollection("credentials").replaceOne({_id:"credential-main"}, EJSON.parse("{\\"_id\\":\\"credential-main\\",\\"count\\":{\\"$numberLong\\":\\"2\\"}}"))',
  );
  assert.equal(
    calls[2].expression,
    'db.getCollection("credentials").updateOne({_id:"credential-main"}, {$set:EJSON.parse("{\\"secret\\":{\\"$binary\\":{\\"base64\\":\\"AA==\\",\\"subType\\":\\"00\\"}}}")})',
  );
  assert.equal(
    calls[3].expression,
    'db.getCollection("credentials").countDocuments({"secret._skiff_encrypted.keyId":{$ne:"key-next"}})',
  );
});

test('mongo probe initializes the replica set and decodes transient storage', async () => {
  const calls = [];
  const documents = [
    {
      _id: 'credential-main',
      alpha: { _skiff_encrypted: { keyId: 'key-next' } },
      beta: { _skiff_encrypted: { keyId: 'key-old' } },
    },
    {
      _id: 'credential-secondary',
      alpha: { _skiff_encrypted: { keyId: 'key-old' } },
    },
  ];
  const probe = createEncryptedStorageLiveMongoProbe({
    mongoPort: 45888,
    cwd: '/repo/skiff',
    wait: async () => {
      throw new Error('successful canned responses must not wait');
    },
    command: {
      async run(args, options) {
        calls.push({ kind: 'run', args, options });
        return { stdout: '', stderr: '' };
      },
      async json(input) {
        calls.push({ kind: 'json', ...input });
        if (input.expression === 'db.hello().isWritablePrimary === true') {
          return true;
        }
        if (input.expression.includes('listDatabases')) {
          return ['admin', 'example~com~~encrypted-live'];
        }
        if (input.expression === 'db.getCollectionNames().sort()') {
          return ['credentials'];
        }
        if (input.expression.includes('.find({}).sort({_id:1}).toArray()')) {
          return documents;
        }
        throw new Error(`unexpected expression: ${input.expression}`);
      },
    },
  });

  await probe.initializeReplicaSet();
  assert.deepEqual(calls[0].args.slice(0, 3), [
    'mongodb://127.0.0.1:45888/admin?directConnection=true',
    '--quiet',
    '--eval',
  ]);
  assert.match(calls[0].args[3], /127\.0\.0\.1:45888/);
  assert.deepEqual(calls[0].options, { cwd: '/repo/skiff' });

  const storage = await probe.observeTransientEncryptedStorage(new Set());
  assert.deepEqual(storage, {
    storageServiceId: 'example.com/encrypted-live',
    database: 'example~com~~encrypted-live',
    collection: 'credentials',
    fields: ['alpha', 'beta'],
    keyIds: ['key-next', 'key-old'],
    rawSnapshot: JSON.stringify(documents),
  });
  for (const call of calls.filter(({ kind }) => kind === 'json')) {
    assert.equal(call.cwd, '/repo/skiff');
    assert.match(call.url, /^mongodb:\/\/127\.0\.0\.1:45888\//);
  }
});
