import assert from 'node:assert/strict';
import test from 'node:test';

import {
  createEncryptedStorageLiveInstanceResources,
  createEncryptedStorageLiveOwnedProcessGroupStopper,
  encryptedStorageLiveInstanceConfigText,
  isEncryptedStorageLivePortForbidden,
} from '../lib/encrypted-storage-live-instance-resources.mjs';

test('instance resources reject forbidden ports and write the canonical config', async () => {
  for (const port of [27017, 4000, 4004, 4007, 44000, 44999, 46000]) {
    assert.equal(isEncryptedStorageLivePortForbidden(port), true, port);
  }
  for (const port of [45000, 45555, 45999]) {
    assert.equal(isEncryptedStorageLivePortForbidden(port), false, port);
  }

  const events = [];
  const randomValues = [20, 30];
  const resources = await createEncryptedStorageLiveInstanceResources({
    repoRoot: '/repo/skiff',
    environment: 'dev',
    temporaryDirectory: '/tmp/tests',
    randomPort: () => randomValues.shift(),
    leasePorts: async (ports) => {
      events.push(['lease', ports]);
      return {
        async release() {
          events.push(['release']);
        },
      };
    },
    makeTempDirectory: async (prefix) => {
      events.push(['temp', prefix]);
      return '/tmp/tests/run-a';
    },
    makeDirectory: async (path, options) => {
      events.push(['mkdir', path, options]);
    },
    writeTextFile: async (path, text, encoding) => {
      events.push(['write', path, text, encoding]);
    },
  });

  assert.deepEqual(resources.portLease.ports, { base: 45020, mongo: 45530 });
  assert.deepEqual(resources.paths, {
    tempRoot: '/tmp/tests/run-a',
    instanceRoot: '/tmp/tests/run-a/instance',
    configPath: '/tmp/tests/run-a/instance/config.yml',
    devHome: '/tmp/tests/run-a/instance/dev-home',
    artifactRoot: '/tmp/tests/run-a/instance/dev-home/artifacts',
    keyring:
      '/tmp/tests/run-a/instance/dev-home/secrets/service-db-keyring.json',
    runtimeLog: '/tmp/tests/run-a/instance/logs/runtime.log',
    runtimeErrorLog: '/tmp/tests/run-a/instance/logs/runtime.err.log',
    routerLog: '/tmp/tests/run-a/instance/logs/router.log',
    routerErrorLog: '/tmp/tests/run-a/instance/logs/router.err.log',
    fixtureRoot: '/repo/skiff/runtime/encrypted-storage-live',
  });
  assert.deepEqual(events.slice(0, 3), [
    ['lease', [45020, 45021, 45022, 45530]],
    ['temp', '/tmp/tests/skiff-encrypted-storage-live-'],
    ['mkdir', '/tmp/tests/run-a/instance', { recursive: true }],
  ]);
  assert.deepEqual(events[3], [
    'write',
    '/tmp/tests/run-a/instance/config.yml',
    encryptedStorageLiveInstanceConfigText({
      repoRoot: '/repo/skiff',
      environment: 'dev',
      ports: { base: 45020, mongo: 45530 },
    }),
    'utf8',
  ]);
  assert.equal(events[3][2], [
    'environment: dev',
    'devHome: dev-home',
    'cargoTargetDir: "/repo/skiff/build/cargo-target"',
    'ports:',
    '  base: 45020',
    '  mongo: 45530',
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
  ].join('\n'));
  await resources.portLease.release();
  assert.deepEqual(events.at(-1), ['release']);
});

test('owned process group discovery rejects mismatched metadata before signaling', async () => {
  let readCount = 0;
  let validated = false;
  let signaled = false;
  const stopOwnedProcessGroups =
    createEncryptedStorageLiveOwnedProcessGroupStopper({
      readDirectory: async () => ['router.pid'],
      readTextFile: async () => {
        readCount += 1;
        return JSON.stringify({
          configPath: '/tmp/other/config.yml',
          instanceRoot: '/tmp/owned/instance',
          pgid: 9001,
        });
      },
      killProcess: () => {
        signaled = true;
      },
      wait: async () => undefined,
    });
  await assert.rejects(
    stopOwnedProcessGroups({
      instanceRoot: '/tmp/owned/instance',
      configPath: '/tmp/owned/instance/config.yml',
      onValidated: () => {
        validated = true;
      },
    }),
    /refusing to stop unowned process metadata router\.pid/,
  );
  assert.equal(readCount, 1);
  assert.equal(validated, false);
  assert.equal(signaled, false);
});

test('owned process groups receive TERM, bounded waits, and survivor-only KILL', async () => {
  const events = [];
  const alive = new Set([101, 202]);
  const stopOwnedProcessGroups =
    createEncryptedStorageLiveOwnedProcessGroupStopper({
      readDirectory: async () => ['router.pid', 'runtime.pid'],
      readTextFile: async (path) => JSON.stringify({
        configPath: '/tmp/owned/instance/config.yml',
        instanceRoot: '/tmp/owned/instance',
        pgid: path.endsWith('router.pid') ? 101 : 202,
      }),
      killProcess(pid, signal) {
        const pgid = -pid;
        if (signal === 0) {
          events.push(['probe', pgid]);
          if (!alive.has(pgid)) {
            const error = new Error('not found');
            error.code = 'ESRCH';
            throw error;
          }
          return;
        }
        events.push(['signal', pgid, signal]);
        if (signal === 'SIGTERM' && pgid === 101) {
          alive.delete(pgid);
        }
        if (signal === 'SIGKILL') {
          alive.delete(pgid);
        }
      },
      wait: async (milliseconds) => {
        events.push(['wait', milliseconds]);
      },
    });
  await stopOwnedProcessGroups({
    instanceRoot: '/tmp/owned/instance',
    configPath: '/tmp/owned/instance/config.yml',
    onValidated: (groups) => {
      events.push(['validated', groups]);
    },
  });

  const signals = events.filter(([kind]) => kind === 'signal');
  assert.deepEqual(events[0], ['validated', [101, 202]]);
  assert.deepEqual(signals, [
    ['signal', 101, 'SIGTERM'],
    ['signal', 202, 'SIGTERM'],
    ['signal', 202, 'SIGKILL'],
  ]);
  assert.equal(
    events.filter(([kind]) => kind === 'wait').length,
    40,
  );
  const firstWait = events.findIndex(([kind]) => kind === 'wait');
  const kill = events.findIndex(
    (event) => event[0] === 'signal' && event[2] === 'SIGKILL',
  );
  assert(firstWait > events.findIndex((event) => event[2] === 'SIGTERM'));
  assert(kill > firstWait);
  assert.equal(alive.size, 0);
});
