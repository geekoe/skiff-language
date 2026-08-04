import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';

import { initStack } from '../lib/stack-init.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

test('init authors empty assembly + profile snapshot + std records and seeds Mongo generation 0', async (t) => {
  const { configDir, shell, calls, authoring, authoringState } = await initFixture(t);
  const result = await initStack({ configDir, skiffRoot, shell, authoring });

  assert.equal(authoringState.assemblyCall.profile, 'prod');
  assert.deepEqual(authoringState.assemblyCall.rootDeployments, []);
  assert.equal(authoringState.assemblyCall.action, 'build');
  assert.equal(authoringState.snapshotCall.profile, 'prod');
  assert.deepEqual(authoringState.snapshotCall.sources, []);
  assert.equal(typeof authoringState.stdCall.artifactRoot, 'string');

  const artifactsRsync = calls.find((call) => (
    call.op === 'rsync'
    && call.destination === 'init.test:/srv/skiff/artifacts/'
  ));
  assert.ok(artifactsRsync, 'artifacts must be materialized to the remote artifact root');
  assert.deepEqual(artifactsRsync.extra, ['--delete']);

  const mongo = calls.find((call) => call.op === 'ssh' && call.command.includes('mongosh'));
  assert.ok(mongo, 'Mongo state must be seeded over ssh');
  assert.match(mongo.command, /getSiblingDB\("skiff-router"\)/);
  assert.match(mongo.command, /getCollection\("activation_state"\)/);
  assert.match(mongo.command, /insertOne/);
  assert.match(mongo.command, /"_id":"prod"/);
  assert.match(mongo.command, /"generation":0/);
  assert.match(mongo.command, /"schemaVersion":"skiff-profile-activation-state-v1"/);
  assert.match(
    mongo.command,
    new RegExp(`"assemblyIdentity":"skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}"`),
  );
  assert.match(mongo.command, /"snapshotId":"skiff-runtime-config-snapshot-v1:snapshot-1"/);

  const pm2Calls = calls.filter((call) => call.op === 'ssh' && call.command.includes('pm2'));
  assert.equal(pm2Calls.length, 3);
  assert.match(pm2Calls[0].command, /pm2 delete skiff-router \|\| true/);
  assert.match(pm2Calls[1].command, /pm2 startOrReload ecosystem\.config\.cjs --only skiff-router --update-env/);
  assert.match(pm2Calls[2].command, /pm2 save/);

  assert.equal(result.profile, 'prod');
  assert.equal(result.generation, 0);
  assert.equal(result.configSnapshotId, 'skiff-runtime-config-snapshot-v1:snapshot-1');
});

test('init fails closed before authoring when router.yml has no serviceDb.mongoUrl', async (t) => {
  const { configDir, shell, calls, authoring, authoringState } = await initFixture(t, {
    router: 'profile: prod\nhost: 127.0.0.1\n',
  });
  await assert.rejects(
    initStack({ configDir, skiffRoot, shell, authoring }),
    /router\.yml serviceDb\.mongoUrl is required/,
  );
  assert.equal(calls.length, 0);
  assert.equal(authoringState.assemblyCall, undefined);
});

test('init fails closed when the actor routing projection record is missing', async (t) => {
  const { configDir, shell, calls, authoring, authoringState } = await initFixture(t);
  authoringState.writeProjection = false;
  await assert.rejects(
    initStack({ configDir, skiffRoot, shell, authoring }),
    /records\/actor-routing\/current\.json/,
  );
  assert.equal(calls.filter((call) => call.op === 'ssh').length, 0);
});

async function initFixture(t, {
  router = undefined,
} = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-init-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const configDir = join(root, 'configDir');
  await mkdir(configDir, { recursive: true });
  await writeFile(join(configDir, 'build.yml'), [
    'target: x86_64-unknown-linux-gnu',
    'zigDir: /cache/zig',
    'buildRoot: build/runtime-stack',
    'cargoTargetDir: build/cargo-target',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'config.yml'), [
    'profile: prod',
    'remote:',
    '  host: init.test',
    '  remoteSkiff: /srv/skiff',
    '  nodeBin: /opt/node/bin',
    'verify:',
    '  httpPort: 4000',
    '  controlPort: 4001',
    '  telemetryPort: 4002',
    '  healthPath: /__router/health',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'router.yml'), router ?? [
    'profile: prod',
    'host: 127.0.0.1',
    'artifactsPath: /srv/skiff/artifacts',
    'serviceDb:',
    '  mongoUrl: mongodb://127.0.0.1:27017',
    '',
  ].join('\n'));
  await writeFile(
    join(configDir, 'runtime.yml'),
    'router: ws://127.0.0.1:4001/runtime\nruntime-home: /srv/skiff/runtime-home\n',
  );
  await writeFile(
    join(configDir, 'telemetry.yml'),
    'telemetry:\n  host: 127.0.0.1\n  port: 4002\n',
  );

  const calls = [];
  const shell = {
    remoteRun: async (host, command) => {
      calls.push({ op: 'ssh', host, command });
    },
    rsync: async (source, destination, extra = []) => {
      calls.push({ op: 'rsync', source, destination, extra });
    },
  };
  const authoringState = {
    assemblyCall: undefined,
    snapshotCall: undefined,
    stdCall: undefined,
    writeProjection: true,
  };
  const authoring = {
    runCompilerAuthoring: async ({ artifactRoot, ...args }) => {
      authoringState.assemblyCall = { ...args, artifactRoot };
      if (authoringState.writeProjection) {
        const projectionPath = join(artifactRoot, 'records', 'actor-routing', 'current.json');
        await mkdir(dirname(projectionPath), { recursive: true });
        await writeFile(
          projectionPath,
          '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}',
        );
      }
      return {
        runtimeAssemblyReceipt: {
          assembly: { assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}` },
          recordPath: 'records/runtime-assemblies/assembly.json',
        },
      };
    },
    runConfigSnapshotAuthoring: async (args) => {
      authoringState.snapshotCall = args;
      return {
        runtimeConfigSnapshotReceipt: {
          snapshot: { snapshotId: 'skiff-runtime-config-snapshot-v1:snapshot-1' },
        },
      };
    },
    runStdSeedAuthoring: async (args) => {
      authoringState.stdCall = args;
      return {
        package: { artifact: { packageId: 'skiff.run/std' } },
        pointer: { artifact: { packageId: 'skiff.run/std' } },
        pointerPath: 'records/packages/skiff.run/std/1.0.0/current.json',
      };
    },
  };
  return { configDir, shell, calls, authoring, authoringState, root };
}
