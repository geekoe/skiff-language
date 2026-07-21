import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { test } from 'node:test';

import {
  isolatedTestRuntimeConstants,
  runInIsolatedTestRuntime,
  shouldUseIsolatedTestRuntime,
} from '../lib/isolated-test-runtime.mjs';
import {
  bootstrapCanonicalArgs,
  isolatedInstanceOperations,
  isolatedRuntimeHealthReady,
  isolatedTestInstanceConfigText,
  isolatedTestRunnerEnvironment,
} from '../lib/isolated-test-runtime-instance.mjs';
import { leaseConsecutiveLocalPorts } from '../lib/local-port-lease.mjs';
import { runOwnedCommand } from '../lib/owned-command.mjs';

test('isolated instance config and runner env stay inside dynamic temp boundaries', () => {
  const devHome = '/tmp/skiff-test-runtime/instance/dev-home';
  const config = isolatedTestInstanceConfigText({
    devHome,
    cargoTarget: '/checkout/build/cargo-target',
    basePort: 46042,
  });
  assert.match(config, /devHome: "\/tmp\/skiff-test-runtime\/instance\/dev-home"/);
  assert.match(config, /cargoTargetDir: "\/checkout\/build\/cargo-target"/);
  assert.match(config, /base: 46042/);
  assert.match(config, /mongo: 27017/);
  assert.match(config, /telemetry: disabled/);
  assert.match(config, /mongo: disabled/);
  assert.match(config, /watch: disabled/);
  assert.doesNotMatch(config, /400[0-7]/);

  const environment = isolatedTestRunnerEnvironment({
    baseEnv: { PATH: '/bin', SKIFF_DEV_RELOAD_URL: 'http://127.0.0.1:4001/stable' },
    devHome,
    controlPort: 46043,
    routerHttpPort: 46042,
  });
  assert.equal(environment.SKIFF_DEV_HOME, devHome);
  assert.equal(environment.SKIFF_DEV_RELOAD_URL, undefined);
  assert.equal(environment.SKIFF_TEST_ARTIFACT_ROOT, `${devHome}/artifacts`);
  assert.equal(environment.SKIFF_TEST_INGRESS_URL, 'http://127.0.0.1:46042');
});

test('bootstrap comes from current checkout and writes only canonical generation zero', () => {
  const args = bootstrapCanonicalArgs({
    skiffRoot: '/checkout/skiff',
    artifactRoot: '/tmp/isolated/dev-home/artifacts',
    environment: 'isolated-test',
  });
  assert.deepEqual(args, [
    'run',
    '--quiet',
    '--manifest-path',
    '/checkout/skiff/test-runner/Cargo.toml',
    '--bin',
    'skiff-package-service-smoke-fixture',
    '--',
    '--bootstrap-only',
    '--artifact-root',
    '/tmp/isolated/dev-home/artifacts',
    '--environment',
    'isolated-test',
  ]);
  assert.equal(args.some((value) => value.includes('.skiff-instance')), false);
  assert.equal(args.some((value) => value.includes('reload')), false);
});

test('default owner shutdown invokes current checkout instance down command', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-owner-shutdown-test-'));
  const scriptsRoot = join(root, 'scripts');
  const capturePath = join(root, 'capture.json');
  const configPath = join(root, 'instance', 'config.yml');
  try {
    await mkdir(scriptsRoot, { recursive: true });
    await writeFile(join(scriptsRoot, 'skiff-instance.mjs'), [
      "import { writeFile } from 'node:fs/promises';",
      "await writeFile(process.env.SKIFF_OWNER_SHUTDOWN_CAPTURE, JSON.stringify(process.argv.slice(2)));",
    ].join('\n'));
    const operations = isolatedInstanceOperations({
      skiffRoot: root,
      baseEnv: {
        ...process.env,
        SKIFF_OWNER_SHUTDOWN_CAPTURE: capturePath,
      },
    });

    await operations.stopOwnedInstance(configPath);

    assert.deepEqual(JSON.parse(await readFile(capturePath, 'utf8')), ['down', configPath]);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('readiness requires exact empty-assembly registration and a separate capability handshake', () => {
  const assemblyIdentity = `skiff-runtime-assembly-v1:sha256:${'1'.repeat(64)}`;
  const bootstrap = {
    environment: 'skiff-test',
    bootstrap: { generation: 0, assembly: { assemblyIdentity } },
  };
  const activeAssembly = {
    environment: 'skiff-test',
    generation: 0,
    assemblyIdentity,
  };
  assert.equal(
    isolatedRuntimeHealthReady({
      activeAssembly,
      capabilityConnections: [],
      replicas: [],
    }, bootstrap),
    false,
  );
  assert.equal(
    isolatedRuntimeHealthReady({
      activeAssembly,
      capabilityConnections: [{ runtimeId: 'runtime-1', connected: true }],
      replicas: [{
        replicaId: 'runtime-1',
        connected: true,
        state: 'healthy',
        generation: 0,
        assemblyIdentity,
      }],
    }, bootstrap),
    true,
  );
});

test('success and test failure both run owner shutdown, status, ports, lease, and temp cleanup', async () => {
  for (const failing of [false, true]) {
    const { actions, dependencies } = lifecycleDouble();
    const operation = runInIsolatedTestRuntime({
      skiffRoot: '/checkout/skiff',
      baseEnv: { PATH: '/bin' },
      signalTarget: new EventEmitter(),
      dependencies,
      runTest: async (environment) => {
        actions.push('test');
        assert.equal(environment.SKIFF_DEV_RELOAD_URL, undefined);
        assert.equal(environment.SKIFF_TEST_ACTIVATION_URL.includes(':46001/'), true);
        if (failing) {
          throw new Error('test failed');
        }
        return 'passed';
      },
    });
    if (failing) {
      await assert.rejects(operation, /test failed/);
    } else {
      assert.equal(await operation, 'passed');
    }
    assert.deepEqual(actions, [
      'lease', 'temp', 'config', 'bootstrap', 'spawn', 'ready', 'test',
      'stop-supervisor', 'instance-down', 'instance-status', 'ports-closed',
      'lease-release', 'temp-remove',
    ]);
  }
});

test('test and cleanup errors are both retained and evidence workspace is preserved', async () => {
  const { actions, dependencies } = lifecycleDouble({
    stopOwnedInstance: async () => {
      actions.push('instance-down');
      throw new Error('down failed');
    },
  });
  await assert.rejects(
    runInIsolatedTestRuntime({
      skiffRoot: '/checkout/skiff',
      baseEnv: {},
      signalTarget: new EventEmitter(),
      dependencies,
      runTest: async () => {
        actions.push('test');
        throw new Error('test failed');
      },
    }),
    (error) => {
      assert.match(error.message, /test failed/);
      assert.match(error.message, /down failed|isolated runtime cleanup failed/);
      assert.match(error.message, /preserving isolated runtime workspace \/tmp\/isolated-runtime-double/);
      return true;
    },
  );
  assert.equal(actions.includes('instance-status'), true);
  assert.equal(actions.includes('ports-closed'), true);
  assert.equal(actions.includes('lease-release'), true);
  assert.equal(actions.includes('temp-remove'), false);
});

test('write and bootstrap startup failures do not run instance ownership commands', async () => {
  for (const failureAt of ['config', 'bootstrap']) {
    const { actions, dependencies } = lifecycleDouble({
      ...(failureAt === 'config'
        ? {
            writeConfig: async () => {
              actions.push('config');
              throw new Error('config failed');
            },
          }
        : {
            seedBootstrap: async () => {
              actions.push('bootstrap');
              throw new Error('bootstrap failed');
            },
          }),
    });
    await assert.rejects(
      runInIsolatedTestRuntime({
        skiffRoot: '/checkout/skiff',
        baseEnv: {},
        signalTarget: new EventEmitter(),
        dependencies,
        runTest: async () => assert.fail('test runner must not start'),
      }),
      new RegExp(`${failureAt} failed`),
    );
    assert.equal(actions.includes('instance-down'), false);
    assert.equal(actions.includes('instance-status'), false);
    assert.equal(actions.includes('lease-release'), true);
    assert.equal(actions.includes('temp-remove'), true);
  }
});

test('partial supervisor startup failure still runs owner down plus status and port verification', async () => {
  const { actions, dependencies } = lifecycleDouble({
    waitReady: async () => {
      actions.push('ready');
      throw new Error('runtime startup failed');
    },
  });
  await assert.rejects(
    runInIsolatedTestRuntime({
      skiffRoot: '/checkout/skiff',
      baseEnv: {},
      signalTarget: new EventEmitter(),
      dependencies,
      runTest: async () => assert.fail('test runner must not start'),
    }),
    /runtime startup failed/,
  );
  assert.deepEqual(actions.slice(-6), [
    'stop-supervisor', 'instance-down', 'instance-status', 'ports-closed',
    'lease-release', 'temp-remove',
  ]);
});

test('SIGTERM aborts the test and still completes owned cleanup', async () => {
  const signals = new EventEmitter();
  const { actions, dependencies } = lifecycleDouble();
  await assert.rejects(
    runInIsolatedTestRuntime({
      skiffRoot: '/checkout/skiff',
      baseEnv: {},
      signalTarget: signals,
      dependencies,
      runTest: (_environment, signal) => new Promise((resolvePromise, reject) => {
        signal.addEventListener('abort', () => reject(signal.reason), { once: true });
        queueMicrotask(() => signals.emit('SIGTERM'));
      }),
    }),
    /SIGTERM/,
  );
  assert.deepEqual(actions.slice(-6), [
    'stop-supervisor', 'instance-down', 'instance-status', 'ports-closed',
    'lease-release', 'temp-remove',
  ]);
});

test('aborted owned command waits for its process group before returning', {
  skip: process.platform === 'win32',
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-owned-command-test-'));
  const pidPath = join(root, 'pids.json');
  const grandchildSource = [
    "process.on('SIGTERM', () => setTimeout(() => process.exit(0), 200));",
    'setInterval(() => {}, 1000);',
  ].join('');
  const parentSource = [
    "const { spawn } = require('node:child_process');",
    "const { writeFileSync } = require('node:fs');",
    `const child = spawn(process.execPath, ['-e', ${JSON.stringify(grandchildSource)}], { stdio: 'ignore' });`,
    "writeFileSync(process.argv.at(-1), JSON.stringify([process.pid, child.pid]));",
    "process.on('SIGTERM', () => setTimeout(() => process.exit(0), 200));",
    'setInterval(() => {}, 1000);',
  ].join('');
  const abortController = new AbortController();
  try {
    const operation = runOwnedCommand(process.execPath, ['-e', parentSource, pidPath], {
      signal: abortController.signal,
      stdio: 'ignore',
    });
    const pids = await waitForPidFile(pidPath);
    const startedAt = Date.now();
    abortController.abort(new Error('controlled interruption'));
    await assert.rejects(operation, /controlled interruption/);
    assert.ok(Date.now() - startedAt >= 150, 'abort must wait for graceful group shutdown');
    assert.equal(pids.every((pid) => !processAlive(pid)), true);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('live mode bypasses automatic isolated runtime and leases stay in reserved range', async () => {
  assert.equal(shouldUseIsolatedTestRuntime(true), false);
  assert.equal(shouldUseIsolatedTestRuntime(false), true);
  const first = await leaseConsecutiveLocalPorts({
    rangeStart: isolatedTestRuntimeConstants.portMin,
    rangeEnd: isolatedTestRuntimeConstants.portMax,
    count: 3,
  });
  const second = await leaseConsecutiveLocalPorts({
    rangeStart: isolatedTestRuntimeConstants.portMin,
    rangeEnd: isolatedTestRuntimeConstants.portMax,
    count: 3,
  });
  try {
    assert.equal(first.ports.every((port) => port >= 46000 && port <= 46999), true);
    assert.equal(second.ports.every((port) => port >= 46000 && port <= 46999), true);
    assert.equal(first.ports.some((port) => second.ports.includes(port)), false);
    const forbidden = [27017, ...range(4000, 4007), ...range(44000, 45999)];
    assert.equal(first.ports.some((port) => forbidden.includes(port)), false);
    assert.equal(second.ports.some((port) => forbidden.includes(port)), false);
  } finally {
    await first.release();
    await second.release();
  }
});

function lifecycleDouble(overrides = {}) {
  const actions = [];
  const dependencies = {
    leasePorts: async () => {
      actions.push('lease');
      return {
        ports: [46000, 46001, 46002],
        release: async () => { actions.push('lease-release'); },
      };
    },
    makeTempRoot: async () => {
      actions.push('temp');
      return '/tmp/isolated-runtime-double';
    },
    writeConfig: async () => { actions.push('config'); },
    seedBootstrap: async () => { actions.push('bootstrap'); },
    spawnSupervisor: () => {
      actions.push('spawn');
      return { pid: 1000 };
    },
    waitReady: async () => { actions.push('ready'); },
    stopSupervisor: async () => { actions.push('stop-supervisor'); },
    stopOwnedInstance: async () => { actions.push('instance-down'); },
    verifyInstanceStopped: async () => { actions.push('instance-status'); },
    assertPortsClosed: async () => { actions.push('ports-closed'); },
    removeTempRoot: async () => { actions.push('temp-remove'); },
    ...overrides,
  };
  return { actions, dependencies };
}

function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}

async function waitForPidFile(path) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      return JSON.parse(await readFile(path, 'utf8'));
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
    }
    await delay(20);
  }
  throw new Error(`owned command did not write ${path}`);
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code !== 'ESRCH';
  }
}
