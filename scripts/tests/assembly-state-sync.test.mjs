import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  MONGOSH_EJSON_MARKER,
} from '../lib/mongosh-json-command.mjs';
import {
  assemblyStateSyncUsage,
  parseSyncStateArgs,
  runAssemblyStateSyncCommand,
  syncAssemblyState,
  syncStateMongoUrlEnvVar,
} from '../lib/assembly-state-sync.mjs';

const activationUrl = 'http://127.0.0.1:4001/__skiff/activate-assembly';
const mongoUrl = 'mongodb://127.0.0.1:27017';
const assemblyIdentity = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const configSnapshotId = `skiff-runtime-config-snapshot-v1:${'7'.repeat(32)}`;

function healthResponse(overrides = {}) {
  return new Response(JSON.stringify({
    ok: true,
    activeAssembly: {
      profile: 'dev',
      generation: 7,
      assemblyIdentity,
      configSnapshotId,
      ...overrides,
    },
    pendingActivation: null,
    capabilityConnections: [],
    replicas: [],
    counters: {},
  }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

function stateDocument({ generation = 3, revision = 1, pending = null } = {}) {
  return {
    _id: 'dev',
    revision: { $numberInt: String(revision) },
    state: {
      schemaVersion: 'skiff-profile-activation-state-v1',
      profile: 'dev',
      committed: {
        generation: { $numberInt: String(generation) },
        assembly: { assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'b'.repeat(64)}` },
        configSnapshot: { snapshotId: `skiff-runtime-config-snapshot-v1:${'6'.repeat(32)}` },
      },
      pending,
    },
  };
}

function fakeMongosh({ readDocument = stateDocument(), writeError } = {}) {
  const calls = [];
  const mongosh = async (command, args, options) => {
    calls.push({ command, args, options });
    const evalScript = args.at(-1);
    if (evalScript.includes('findOne')) {
      return {
        stdout: `startup\n${MONGOSH_EJSON_MARKER}${JSON.stringify(readDocument)}\n`,
        stderr: '',
      };
    }
    if (writeError !== undefined) {
      throw writeError;
    }
    return { stdout: '', stderr: '' };
  };
  return { mongosh, calls };
}

test('sync rewrites the activation state document to the health active epoch', async () => {
  const fetched = [];
  const { mongosh, calls } = fakeMongosh({ readDocument: stateDocument() });
  const output = [];
  const result = await syncAssemblyState({
    artifactRoot: '/tmp/artifacts',
    profile: 'dev',
    activationUrl,
    mongoUrl,
    fetchImpl: async (url) => {
      fetched.push(url);
      return healthResponse();
    },
    mongosh,
    stdout: (line) => output.push(line),
  });

  assert.deepEqual(fetched, ['http://127.0.0.1:4001/__router/health']);
  assert.equal(calls.length, 2);

  const readCall = calls[0];
  assert.equal(readCall.command, 'mongosh');
  assert.equal(readCall.args[0], mongoUrl);
  assert.match(readCall.args.at(-1), /findOne\(\{_id: "dev"\}/);
  assert.match(readCall.args.at(-1), /getSiblingDB\("skiff-router"\)/);
  assert.match(readCall.args.at(-1), /getCollection\("activation_state"\)/);

  const writeCall = calls[1];
  assert.equal(writeCall.command, 'mongosh');
  assert.equal(writeCall.args[0], mongoUrl);
  const evalScript = writeCall.args.at(-1);
  assert.match(evalScript, /getSiblingDB\("skiff-router"\)/);
  assert.match(evalScript, /getCollection\("activation_state"\)/);
  assert.match(evalScript, /replaceOne\(\{_id: "dev"\}, /);
  assert.match(evalScript, /upsert: true/);
  assert.match(evalScript, /"_id":"dev"/);
  assert.match(evalScript, /"revision":0/);
  assert.match(evalScript, /"generation":7/);
  assert.match(evalScript, /"schemaVersion":"skiff-profile-activation-state-v1"/);
  assert.match(evalScript, new RegExp(`"assemblyIdentity":"${assemblyIdentity}"`));
  assert.match(evalScript, new RegExp(`"snapshotId":"${configSnapshotId}"`));
  assert.match(evalScript, /"pending":null/);
  assert.deepEqual(writeCall.options, { cwd: process.cwd() });

  assert.deepEqual(result.before, {
    revision: 1,
    generation: 3,
    assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'b'.repeat(64)}`,
    configSnapshotId: `skiff-runtime-config-snapshot-v1:${'6'.repeat(32)}`,
    pending: null,
  });
  assert.deepEqual(result.after, {
    revision: 0,
    generation: 7,
    assemblyIdentity,
    configSnapshotId,
    pending: null,
  });
  assert.equal(output[0], [
    'state: skiff-router.activation_state (profile dev)',
    'generation: 3 -> 7',
    `assembly: skiff-runtime-assembly-v3:sha256:${'b'.repeat(64)}`,
    `  -> ${assemblyIdentity}`,
    `configSnapshot: skiff-runtime-config-snapshot-v1:${'6'.repeat(32)}`,
    `  -> ${configSnapshotId}`,
    'revision: 1 -> 0',
  ].join('\n'));
});

test('sync writes the document when no prior document exists', async () => {
  const { mongosh, calls } = fakeMongosh({ readDocument: null });
  const result = await syncAssemblyState({
    artifactRoot: '/tmp/artifacts',
    profile: 'dev',
    activationUrl,
    mongoUrl,
    fetchImpl: async () => healthResponse(),
    mongosh,
    stdout: () => {},
  });
  assert.equal(result.before, null);
  assert.equal(calls.length, 2);
  assert.equal(result.after.generation, 7);
});

test('sync --json emits a structured result', async () => {
  const output = [];
  await syncAssemblyState({
    artifactRoot: '/tmp/artifacts',
    profile: 'dev',
    activationUrl,
    mongoUrl,
    fetchImpl: async () => healthResponse(),
    mongosh: fakeMongosh().mongosh,
    stdout: (line) => output.push(line),
    json: true,
  });
  const parsed = JSON.parse(output[0]);
  assert.equal(parsed.profile, 'dev');
  assert.equal(parsed.mongo, 'skiff-router.activation_state');
  assert.equal(parsed.before.generation, 3);
  assert.equal(parsed.after.generation, 7);
});

test('sync fails closed when health has no active assembly or is not ok', async () => {
  const cases = [
    { ok: false, activeAssembly: { profile: 'dev', generation: 7, assemblyIdentity, configSnapshotId } },
    { ok: true, activeAssembly: null },
    { ok: true },
  ];
  for (const body of cases) {
    const mongosh = fakeMongosh().mongosh;
    await assert.rejects(
      syncAssemblyState({
        artifactRoot: '/tmp/artifacts',
        profile: 'dev',
        activationUrl,
        mongoUrl,
        fetchImpl: async () => new Response(JSON.stringify(body), { status: 200 }),
        mongosh,
        stdout: () => {},
      }),
      /exact active assembly tuple/,
    );
  }
});

test('sync fails closed when health is unreachable or rejects', async () => {
  for (const fetchImpl of [
    async () => { throw new Error('connection refused'); },
    async () => new Response('boom', { status: 503 }),
  ]) {
    await assert.rejects(
      syncAssemblyState({
        artifactRoot: '/tmp/artifacts',
        profile: 'dev',
        activationUrl,
        mongoUrl,
        fetchImpl,
        mongosh: fakeMongosh().mongosh,
        stdout: () => {},
      }),
      /router health/,
    );
  }
});

test('sync fails closed on a router profile mismatch', async () => {
  await assert.rejects(
    syncAssemblyState({
      artifactRoot: '/tmp/artifacts',
      profile: 'prod',
      activationUrl,
      mongoUrl,
      fetchImpl: async () => healthResponse(),
      mongosh: fakeMongosh().mongosh,
      stdout: () => {},
    }),
    /router coordinates profile dev, not requested prod/,
  );
});

test('sync propagates mongosh write failures without writing anything else', async () => {
  const writeError = new Error('mongosh failed');
  const { mongosh, calls } = fakeMongosh({ writeError });
  await assert.rejects(
    syncAssemblyState({
      artifactRoot: '/tmp/artifacts',
      profile: 'dev',
      activationUrl,
      mongoUrl,
      fetchImpl: async () => healthResponse(),
      mongosh,
      stdout: () => {},
    }),
    /mongosh failed/,
  );
  assert.equal(calls.length, 2);
  assert.match(calls[1].args.at(-1), /replaceOne/);
});

test('sync validates artifact-root, profile, and mongo url up front', async () => {
  const base = {
    profile: 'dev',
    activationUrl,
    mongoUrl,
    fetchImpl: async () => healthResponse(),
    mongosh: fakeMongosh().mongosh,
    stdout: () => {},
  };
  await assert.rejects(
    syncAssemblyState({ ...base, artifactRoot: 'relative/artifacts' }),
    /absolute --artifact-root/,
  );
  await assert.rejects(
    syncAssemblyState({ ...base, artifactRoot: '/tmp/artifacts', profile: '..' }),
    /canonical ASCII profile token/,
  );
  await assert.rejects(
    syncAssemblyState({ ...base, artifactRoot: '/tmp/artifacts', mongoUrl: '  ' }),
    /requires a mongo URL/,
  );
});

test('parse rejects malformed sync-state arguments', () => {
  assert.deepEqual(parseSyncStateArgs([
    '--artifact-root',
    '/tmp/artifacts',
    '--profile',
    'dev',
    '--activation-url',
    activationUrl,
    '--mongo-url',
    mongoUrl,
    '--json',
  ]), {
    artifactRoot: '/tmp/artifacts',
    profile: 'dev',
    activationUrl: 'http://127.0.0.1:4001/__skiff/activate-assembly',
    mongoUrl,
    json: true,
  });
  for (const bad of [
    ['--profile', 'dev', '--activation-url', activationUrl, '--mongo-url', mongoUrl],
    ['--artifact-root', '/tmp/artifacts', '--activation-url', activationUrl, '--mongo-url', mongoUrl],
    ['/tmp/artifacts', '--profile', 'dev', '--activation-url', activationUrl, '--mongo-url', mongoUrl],
    ['--artifact-root', '/tmp/artifacts', '--profile', 'dev', '--activation-url', activationUrl, '--mongo-url', mongoUrl, '--bogus'],
    ['--artifact-root', '/tmp/artifacts', '--artifact-root', '/tmp/other', '--profile', 'dev', '--activation-url', activationUrl, '--mongo-url', mongoUrl],
    ['--artifact-root', '/tmp/artifacts', '--profile', 'dev', '--activation-url', 'not-a-url', '--mongo-url', mongoUrl],
    ['--artifact-root', '/tmp/artifacts', '--profile', 'dev', '--activation-url', 'ftp://127.0.0.1:4001/x', '--mongo-url', mongoUrl],
  ]) {
    assert.throws(
      () => parseSyncStateArgs(bad),
      /requires|unknown option|does not accept a positional root|http\(s\) URL|provided more than once/,
    );
  }
});

test('parse accepts --option=value form and an http activation origin', () => {
  const parsed = parseSyncStateArgs([
    '--artifact-root=/tmp/artifacts',
    '--profile=dev',
    '--activation-url=http://127.0.0.1:4001',
    '--mongo-url=mongodb://localhost:27017',
  ]);
  assert.equal(parsed.artifactRoot, '/tmp/artifacts');
  assert.equal(parsed.profile, 'dev');
  assert.equal(parsed.activationUrl, 'http://127.0.0.1:4001');
  assert.equal(parsed.mongoUrl, 'mongodb://localhost:27017');
  assert.equal(parsed.json, false);
});

test('runAssemblyStateSyncCommand resolves mongo url from the environment fallback', async () => {
  const previous = process.env[syncStateMongoUrlEnvVar];
  process.env[syncStateMongoUrlEnvVar] = mongoUrl;
  try {
    const result = await runAssemblyStateSyncCommand([
      '--artifact-root',
      '/tmp/artifacts',
      '--profile',
      'dev',
      '--activation-url',
      activationUrl,
      '--json',
    ], {
      mongoUrlEnv: process.env[syncStateMongoUrlEnvVar],
      fetchImpl: async () => healthResponse(),
      mongosh: fakeMongosh().mongosh,
      stdout: () => {},
    });
    assert.equal(result.after.generation, 7);
    assert.equal(result.mongo, 'skiff-router.activation_state');
  } finally {
    if (previous === undefined) {
      delete process.env[syncStateMongoUrlEnvVar];
    } else {
      process.env[syncStateMongoUrlEnvVar] = previous;
    }
  }
});

test('runAssemblyStateSyncCommand fails closed without a mongo url', async () => {
  await assert.rejects(
    runAssemblyStateSyncCommand([
      '--artifact-root',
      '/tmp/artifacts',
      '--profile',
      'dev',
      '--activation-url',
      activationUrl,
    ], { mongoUrlEnv: undefined }),
    new RegExp(`requires --mongo-url or ${syncStateMongoUrlEnvVar}`),
  );
});

test('runAssemblyStateSyncCommand prints help for -h/--help', async () => {
  for (const flag of ['-h', '--help']) {
    const output = [];
    const previousLog = console.log;
    console.log = (line) => output.push(line);
    try {
      const result = await runAssemblyStateSyncCommand([flag]);
      assert.equal(result, null);
    } finally {
      console.log = previousLog;
    }
    assert.equal(output.join('\n'), assemblyStateSyncUsage);
  }
});
