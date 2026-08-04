import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import { stackStatus } from '../lib/stack-status.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');
const REMOTE_ROUTER_YML = [
  'profile: prod',
  'host: 127.0.0.1',
  'artifactsPath: /srv/skiff/artifacts',
  'serviceDb:',
  '  mongoUrl: mongodb://127.0.0.1:27017',
  '',
].join('\n');

test('status cross-checks remote router.yml, health profile, and runtime connected', async (t) => {
  const { configDir, shell, calls } = await statusFixture(t, {
    health: {
      activeAssembly: {
        profile: 'prod',
        generation: 0,
        assemblyIdentity: 'assembly',
        configSnapshotId: 'snapshot',
      },
      replicas: [{
        replicaId: 'runtime-a',
        connected: true,
        state: 'healthy',
        profile: 'prod',
        generation: 0,
        assemblyIdentity: 'assembly',
        configSnapshotId: 'snapshot',
      }],
    },
  });
  const result = await stackStatus({ configDir, skiffRoot, shell });

  assert.equal(result.profile, 'prod');
  assert.equal(result.generation, 0);
  assert.equal(result.remoteRouterProfile, 'prod');
  assert.equal(result.activeProfile, 'prod');
  assert.equal(result.runtimeConnected, true);
  assert.equal(result.replicaId, 'runtime-a');
  assert.equal(calls.length, 2);
  assert.match(calls[0].command, /cat \/srv\/skiff\/config\/router\.yml/);
  assert.match(calls[1].command, /curl -fsS http:\/\/127\.0\.0\.1:4001\/__router\/health/);
});

test('status fails closed when the remote router.yml profile differs', async (t) => {
  const { configDir, shell, calls } = await statusFixture(t, {
    remoteRouter: REMOTE_ROUTER_YML.replace('profile: prod', 'profile: other'),
  });
  await assert.rejects(
    stackStatus({ configDir, skiffRoot, shell }),
    /config\.yml\.profile="prod" but remote router\.yml\.profile="other"/,
  );
  assert.equal(calls.length, 1);
});

test('status fails closed when health activeAssembly.profile differs', async (t) => {
  const { configDir, shell, calls } = await statusFixture(t, {
    health: {
      activeAssembly: { profile: 'other', generation: 0 },
      replicas: [],
    },
  });
  await assert.rejects(
    stackStatus({ configDir, skiffRoot, shell }),
    /health activeAssembly\.profile="other"/,
  );
  assert.equal(calls.length, 2);
});

test('status fails closed when no healthy runtime replica is connected', async (t) => {
  const { configDir, shell } = await statusFixture(t, {
    health: {
      activeAssembly: { profile: 'prod', generation: 0 },
      replicas: [{
        replicaId: 'runtime-b',
        connected: false,
        state: 'starting',
        profile: 'prod',
        generation: 0,
      }],
    },
  });
  await assert.rejects(
    stackStatus({ configDir, skiffRoot, shell }),
    /runtime is not connected and healthy/,
  );
});

async function statusFixture(t, {
  remoteRouter = REMOTE_ROUTER_YML,
  health,
} = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-stack-status-test-'));
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
    '  host: status.test',
    '  remoteSkiff: /srv/skiff',
    '  nodeBin: /opt/node/bin',
    'verify:',
    '  httpPort: 4000',
    '  controlPort: 4001',
    '  telemetryPort: 4002',
    '  healthPath: /__router/health',
    '',
  ].join('\n'));
  await writeFile(join(configDir, 'router.yml'), REMOTE_ROUTER_YML);
  await writeFile(
    join(configDir, 'runtime.yml'),
    'router: ws://127.0.0.1:4001/runtime\nruntime-home: /srv/skiff/runtime-home\n',
  );
  await writeFile(
    join(configDir, 'telemetry.yml'),
    'telemetry:\n  host: 127.0.0.1\n  port: 4002\n',
  );

  const calls = [];
  const responses = [remoteRouter, JSON.stringify(health)];
  const shell = {
    remoteCapture: async (host, command) => {
      calls.push({ host, command });
      const response = responses.shift();
      if (response === undefined) {
        throw new Error(`unexpected remote capture: ${command}`);
      }
      return response;
    },
  };
  return { configDir, shell, calls, root };
}
