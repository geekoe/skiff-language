import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
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
  isolatedTestInstanceRuntimeFiles,
  isolatedRuntimeHealthReady,
  isolatedTestRunnerEnvironment,
} from '../lib/isolated-test-runtime-instance.mjs';
import { leaseConsecutiveLocalPorts } from '../lib/local-port-lease.mjs';
import { runOwnedCommand } from '../lib/owned-command.mjs';
import { claimIsolatedTestWorkspace } from '../lib/isolated-test-runtime-workspace.mjs';

import './isolated-test-runtime-workspace-cases.mjs';

test('bootstrap comes from current checkout and seeds only the canonical baseline records', () => {
  const args = bootstrapCanonicalArgs({
    skiffRoot: '/checkout/skiff',
    artifactRoot: '/tmp/isolated/dev-home/artifacts',
    profile: 'isolated-test',
  });
  assert.deepEqual(args, [
    'run',
    '--quiet',
    '--locked',
    '--manifest-path',
    '/checkout/skiff/test-runner/Cargo.toml',
    '--bin',
    'skiff-package-service-smoke-fixture',
    '--',
    '--seed-committed',
    '/checkout/skiff/test-runner/fixtures/isolated-test-bootstrap',
    '--artifact-root',
    '/tmp/isolated/dev-home/artifacts',
    '--platform-source-root',
    '/checkout/skiff',
    '--profile',
    'isolated-test',
  ]);
  assert.equal(args.some((value) => value.includes('.skiff-instance')), false);
  assert.equal(args.some((value) => value.includes('reload')), false);
});

test('isolated instance initialization provisions runnable configs and dirs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-isolated-init-test-'));
  const devHome = join(root, 'instance', 'dev-home');
  try {
    const ownershipReceipt = await claimIsolatedTestWorkspace(root);
    const operations = isolatedInstanceOperations({
      skiffRoot: root,
      baseEnv: process.env,
    });
    await operations.initializeInstance({
      profile: 'skiff-test',
      devHome,
      basePort: 46042,
      mongoPort: 46045,
      ownershipReceipt,
    });

    const files = isolatedTestInstanceRuntimeFiles({
      profile: 'skiff-test',
      devHome,
      basePort: 46042,
      mongoPort: 46045,
    });
    const routerConfig = await readFile(files.routerConfigPath, 'utf8');
    const runtimeConfig = await readFile(files.runtimeConfigPath, 'utf8');
    assert.match(routerConfig, /^profile: skiff-test$/m);
    assert.match(routerConfig, /^serviceDb:$/m);
    assert.match(routerConfig, /mongoUrl: "mongodb:\/\/127\.0\.0\.1:46045\//);
    assert.match(runtimeConfig, /router: "ws:\/\/127\.0\.0\.1:46043\/runtime"/);
    assert.match(runtimeConfig, /service-db-keyring\.json/);
    assert.equal((await stat(files.keyringPath)).isFile(), true);
    for (const dir of ['mongo-data', 'logs', 'secrets', 'runtime-home', 'artifacts']) {
      assert.equal((await stat(join(devHome, dir))).isDirectory(), true, dir);
    }
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('readiness requires one connected replica and its own capability handshake', () => {
  const bootstrap = {
    profile: 'skiff-test',
    bootstrap: {
      assembly: { assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'1'.repeat(64)}` },
    },
  };
  const readyHealth = {
    activeAssembly: {
      profile: 'skiff-test',
      releaseCount: 1,
      buildIds: ['deployment-a'],
    },
    capabilityConnections: [{ runtimeId: 'runtime-1', connected: true }],
    replicas: [{
      replicaId: 'runtime-1',
      profile: 'skiff-test',
      connected: true,
      state: 'healthy',
    }],
  };
  assert.equal(
    isolatedRuntimeHealthReady({
      activeAssembly: readyHealth.activeAssembly,
      capabilityConnections: [],
      replicas: [],
    }, bootstrap),
    false,
  );
  assert.equal(isolatedRuntimeHealthReady(readyHealth, bootstrap), true);

  const healthMutations = [
    ['active profile differs', (health) => { health.activeAssembly.profile = 'other'; }],
    ['active profile is missing', (health) => { delete health.activeAssembly.profile; }],
    ['replica id differs', (health) => { health.replicas[0].replicaId = 'runtime-2'; }],
    ['replica id is missing', (health) => { delete health.replicas[0].replicaId; }],
    ['replica connected is false', (health) => { health.replicas[0].connected = false; }],
    ['replica connected is missing', (health) => { delete health.replicas[0].connected; }],
    ['replica state is not healthy', (health) => { health.replicas[0].state = 'draining'; }],
    ['replica state is missing', (health) => { delete health.replicas[0].state; }],
    ['capability runtime differs', (health) => { health.capabilityConnections[0].runtimeId = 'runtime-2'; }],
    ['capability runtime is missing', (health) => { delete health.capabilityConnections[0].runtimeId; }],
    ['capability connected is false', (health) => { health.capabilityConnections[0].connected = false; }],
    ['capability connected is missing', (health) => { delete health.capabilityConnections[0].connected; }],
  ];
  for (const [scenario, mutate] of healthMutations) {
    const health = structuredClone(readyHealth);
    mutate(health);
    assert.equal(isolatedRuntimeHealthReady(health, bootstrap), false, scenario);
  }

  const splitRuntimeHealth = structuredClone(readyHealth);
  splitRuntimeHealth.capabilityConnections[0].runtimeId = 'runtime-2';
  assert.equal(
    isolatedRuntimeHealthReady(splitRuntimeHealth, bootstrap),
    false,
    'exact replica and connected capability belong to different runtimes',
  );

  const receiptMutations = [
    ['bootstrap profile is missing', (receipt) => { delete receipt.profile; }],
    ['bootstrap payload is missing', (receipt) => { delete receipt.bootstrap; }],
  ];
  for (const [scenario, mutate] of receiptMutations) {
    const receipt = structuredClone(bootstrap);
    mutate(receipt);
    assert.equal(isolatedRuntimeHealthReady(readyHealth, receipt), false, scenario);
  }
});

test('one absolute checkout and Cargo target flow through spawns, bootstrap, and runner', async () => {
  const observed = {};
  const relativeSkiffRoot = 'relative-skiff-checkout';
  const relativeCargoTarget = 'relative-cargo-target';
  const expectedSkiffRoot = resolve(relativeSkiffRoot);
  const expectedCargoTarget = resolve(relativeCargoTarget);
  const { dependencies } = lifecycleDouble({
    spawnMongo: (input) => {
      observed.mongo = input;
      return { pid: 1000 };
    },
    seedBootstrap: async (input) => {
      observed.bootstrap = input;
      observed.bootstrapReceipt = { profile: 'skiff-test', bootstrap: {} };
      return observed.bootstrapReceipt;
    },
    spawnRouter: (input) => {
      observed.router = input;
      return { pid: 1001 };
    },
    spawnRuntime: (input) => {
      observed.runtime = input;
      return { pid: 1002 };
    },
  });

  await runInIsolatedTestRuntime({
    skiffRoot: relativeSkiffRoot,
    baseEnv: {
      PATH: '/bin',
      CARGO_TARGET_DIR: relativeCargoTarget,
      SKIFF_TEST_PLATFORM_SOURCE_ROOT: '/tmp/hostile-platform-root',
    },
    signalTarget: new EventEmitter(),
    dependencies,
    validateBootstrapReceipt: (receipt) => {
      observed.validatedBootstrapReceipt = receipt;
      assert.equal(observed.bootstrap.skiffRoot, expectedSkiffRoot);
      assert.equal(observed.bootstrap.env.SKIFF_TEST_PLATFORM_SOURCE_ROOT, expectedSkiffRoot);
    },
    runTest: async (environment) => { observed.runnerEnv = environment; },
  });

  assert.equal(isAbsolute(expectedCargoTarget), true);
  assert.notEqual(expectedCargoTarget, resolve(expectedSkiffRoot, relativeCargoTarget));
  assert.equal(observed.mongo.mongoBinary, 'mongod');
  assert.equal(observed.mongo.mongoPort, 46003);
  assert.equal(
    observed.mongo.mongoDataDir,
    '/tmp/isolated-runtime-double/instance/dev-home/mongo-data',
  );
  assert.equal(observed.mongo.cwd, '/tmp/isolated-runtime-double/instance');
  assert.equal(
    observed.router.routerBinary,
    join(expectedSkiffRoot, 'build', 'bin', 'skiff-router'),
  );
  assert.equal(observed.router.routerConfigPath, '/tmp/isolated-runtime-double/instance/dev-home/router.yml');
  assert.equal(
    observed.runtime.runtimeBinary,
    join(expectedSkiffRoot, 'build', 'bin', 'runtime'),
  );
  assert.equal(observed.runtime.runtimeConfigPath, '/tmp/isolated-runtime-double/instance/dev-home/runtime.yml');
  assert.equal(observed.bootstrap.skiffRoot, expectedSkiffRoot);
  assert.equal(observed.bootstrap.env.CARGO_TARGET_DIR, expectedCargoTarget);
  assert.equal(observed.bootstrap.env.SKIFF_TEST_PLATFORM_SOURCE_ROOT, expectedSkiffRoot);
  assert.strictEqual(observed.validatedBootstrapReceipt, observed.bootstrapReceipt);
  assert.strictEqual(observed.mongo.env, observed.bootstrap.env);
  assert.strictEqual(observed.router.env, observed.bootstrap.env);
  assert.strictEqual(observed.runtime.env, observed.bootstrap.env);
  assert.strictEqual(observed.runnerEnv, observed.bootstrap.env);
});

test('success and test failure both run stack stop, ports, lease, and temp cleanup', async () => {
  for (const failing of [false, true]) {
    const { actions, dependencies } = lifecycleDouble();
    const operation = runInIsolatedTestRuntime({
      skiffRoot: '/checkout/skiff',
      baseEnv: { PATH: '/bin' },
      signalTarget: new EventEmitter(),
      dependencies,
      runTest: async (environment, _signal, stack) => {
        actions.push('test');
        assert.equal(environment.SKIFF_DEV_RELOAD_URL, undefined);
        assert.equal(environment.SKIFF_TEST_ACTIVATION_URL, undefined);
        assert.equal(environment.SKIFF_TEST_EXPECTED_GENERATION, undefined);
        assert.equal(stack.sourceArtifactRoot, '/tmp/isolated-runtime-double/source-artifacts');
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
      'lease', 'temp', 'workspace-claim', 'source-artifacts', 'instance-init',
      'mongo-spawn', 'mongo-primary', 'bootstrap', 'router-spawn', 'runtime-spawn',
      'ready', 'test',
      'stop-stack', 'ports-closed', 'lease-release', 'temp-remove',
    ]);
  }
});

test('test and cleanup errors are both retained and evidence workspace is preserved', async () => {
  const { actions, dependencies } = lifecycleDouble({
    stopProcesses: async () => {
      actions.push('stop-stack');
      throw new Error('stack stop failed');
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
      assert.match(error.message, /stack stop failed|isolated runtime cleanup failed/);
      assert.match(error.message, /preserving isolated runtime workspace \/tmp\/isolated-runtime-double/);
      return true;
    },
  );
  assert.equal(actions.includes('stop-stack'), true);
  assert.equal(actions.includes('ports-closed'), true);
  assert.equal(actions.includes('lease-release'), true);
  assert.equal(actions.includes('temp-remove'), false);
});

test('stack stop failure remains a cleanup failure while later owners still settle', async () => {
  const { actions, dependencies } = lifecycleDouble({
    stopProcesses: async () => {
      actions.push('stop-stack');
      throw new Error('stack stop failed');
    },
  });
  await assert.rejects(
    runInIsolatedTestRuntime({
      skiffRoot: '/checkout/skiff',
      baseEnv: {},
      signalTarget: new EventEmitter(),
      dependencies,
      runTest: async () => { actions.push('test'); },
    }),
    /stack stop failed/,
  );
  assert.deepEqual(actions.slice(-3), ['stop-stack', 'ports-closed', 'lease-release']);
  assert.equal(actions.includes('temp-remove'), false);
});

test('startup failures preserve stage order and complete stack cleanup', async () => {
  for (const failureAt of ['mongo-spawn', 'bootstrap']) {
    const { actions, dependencies } = lifecycleDouble({
      ...(failureAt === 'mongo-spawn'
        ? {
            spawnMongo: async () => {
              actions.push('mongo-spawn');
              throw new Error('mongo spawn failed');
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
      failureAt === 'mongo-spawn'
        ? /isolated Mongo spawn failed: mongo spawn failed/
        : /isolated bootstrap seed failed: bootstrap failed/,
    );
    assert.equal(actions.includes('test'), false);
    assert.equal(actions.includes('stop-stack'), failureAt === 'bootstrap');
    assert.equal(actions.includes('ports-closed'), true);
    assert.equal(actions.includes('lease-release'), true);
    assert.equal(actions.includes('temp-remove'), true);
  }
});

test('workspace claim failure preserves the workspace and still releases ports and lease', async () => {
  const { actions, dependencies } = lifecycleDouble({
    claimWorkspace: async () => {
      actions.push('workspace-claim');
      throw new Error('workspace claim failed');
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
    (error) => {
      assert.match(error.message, /workspace claim failed/);
      assert.match(error.message, /isolated runtime cleanup failed|preserve unowned temp workspace/);
      assert.match(error.message, /preserving isolated runtime workspace \/tmp\/isolated-runtime-double/);
      return true;
    },
  );
  assert.equal(actions.includes('workspace-claim'), true);
  assert.equal(actions.includes('stop-stack'), false);
  assert.equal(actions.includes('ports-closed'), true);
  assert.equal(actions.includes('lease-release'), true);
  assert.equal(actions.includes('temp-remove'), false);
});

test('partial runtime startup failure still stops spawned stack and verifies ports', async () => {
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
    /isolated Router\/Runtime readiness failed: runtime startup failed/,
  );
  assert.deepEqual(actions.slice(-4), [
    'stop-stack', 'ports-closed', 'lease-release', 'temp-remove',
  ]);
});

test('every ordered startup stage fails precisely and completes owned cleanup', async () => {
  const scenarios = [
    ['Mongo spawn', 'spawnMongo', 'mongo-spawn', 'mongo spawn exploded'],
    ['Mongo primary election', 'waitMongoPrimary', 'mongo-primary', 'primary election exploded'],
    ['bootstrap seed', 'seedBootstrap', 'bootstrap', 'bootstrap seed exploded'],
    ['Router spawn', 'spawnRouter', 'router-spawn', 'router spawn exploded'],
    ['Runtime spawn', 'spawnRuntime', 'runtime-spawn', 'runtime spawn exploded'],
    ['Router/Runtime readiness', 'waitReady', 'ready', 'isolated Router startup failed'],
  ];
  for (const [diagnostic, operation, action, detail] of scenarios) {
    const { actions, dependencies } = lifecycleDouble({
      [operation]: async () => {
        actions.push(action);
        throw new Error(detail);
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
      new RegExp(`isolated ${diagnostic} failed: ${detail}`),
    );
    assert.equal(actions.includes('ports-closed'), true);
    assert.equal(actions.includes('lease-release'), true);
    if (operation === 'spawnMongo') {
      assert.deepEqual(actions.slice(-3), [
        'ports-closed', 'lease-release', 'temp-remove',
      ]);
    } else {
      assert.deepEqual(actions.slice(-4), [
        'stop-stack', 'ports-closed', 'lease-release', 'temp-remove',
      ]);
    }
  }
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
  assert.deepEqual(actions.slice(-4), [
    'stop-stack', 'ports-closed', 'lease-release', 'temp-remove',
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

test('live bypass and reserved port leases preserve a foreign token replacement', async () => {
  assert.equal(shouldUseIsolatedTestRuntime(true), false);
  assert.equal(shouldUseIsolatedTestRuntime(false), true);
  const leaseDir = await mkdtemp(join(tmpdir(), 'skiff-port-lease-audit-'));
  const first = await leaseConsecutiveLocalPorts({
    rangeStart: isolatedTestRuntimeConstants.portMin,
    rangeEnd: isolatedTestRuntimeConstants.portMax,
    count: 4,
    leaseDir,
  });
  const second = await leaseConsecutiveLocalPorts({
    rangeStart: isolatedTestRuntimeConstants.portMin,
    rangeEnd: isolatedTestRuntimeConstants.portMax,
    count: 4,
    leaseDir,
  });
  try {
    assert.equal(first.ports.every((port) => port >= 46000 && port <= 46999), true);
    assert.equal(second.ports.every((port) => port >= 46000 && port <= 46999), true);
    assert.equal(first.ports.some((port) => second.ports.includes(port)), false);
    const forbidden = [27017, ...range(4000, 4007), ...range(44000, 45999)];
    assert.equal(first.ports.some((port) => forbidden.includes(port)), false);
    assert.equal(second.ports.some((port) => forbidden.includes(port)), false);
    const replacedLeasePath = join(leaseDir, `${first.ports[0]}.lock`);
    const foreignLease = `${JSON.stringify({
      schemaVersion: 'skiff-local-port-lease-v1',
      pid: process.pid,
      token: 'foreign-token',
      ports: [first.ports[0]],
    })}\n`;
    await rm(replacedLeasePath);
    await writeFile(replacedLeasePath, foreignLease, 'utf8');
    await first.release();
    assert.equal(await readFile(replacedLeasePath, 'utf8'), foreignLease);
    await rm(replacedLeasePath);
  } finally {
    await first.release();
    await second.release();
    await rm(leaseDir, { force: true, recursive: true });
  }
});

function lifecycleDouble(overrides = {}) {
  const actions = [];
  const workspaceReceipt = {
    schemaVersion: 'isolated-runtime-double-v1',
    nonce: '0'.repeat(32),
    root: { path: '/tmp/isolated-runtime-double', identity: { dev: '1', ino: '2' } },
    marker: {
      path: '/tmp/isolated-runtime-double/.skiff-isolated-workspace-owner.json',
      identity: { dev: '1', ino: '3' },
    },
  };
  const dependencies = {
    leasePorts: async () => {
      actions.push('lease');
      return {
        ports: [46000, 46001, 46002, 46003],
        release: async () => { actions.push('lease-release'); },
      };
    },
    makeTempRoot: async () => {
      actions.push('temp');
      return '/tmp/isolated-runtime-double';
    },
    claimWorkspace: async () => {
      actions.push('workspace-claim');
      return workspaceReceipt;
    },
    createSourceArtifactRoot: async () => { actions.push('source-artifacts'); },
    initializeInstance: async () => { actions.push('instance-init'); },
    seedBootstrap: async () => { actions.push('bootstrap'); },
    spawnMongo: () => {
      actions.push('mongo-spawn');
      return { pid: 1000 };
    },
    waitMongoPrimary: async () => { actions.push('mongo-primary'); },
    spawnRouter: () => {
      actions.push('router-spawn');
      return { pid: 1001 };
    },
    spawnRuntime: () => {
      actions.push('runtime-spawn');
      return { pid: 1002 };
    },
    waitReady: async () => { actions.push('ready'); },
    stopProcesses: async () => { actions.push('stop-stack'); },
    assertPortsClosed: async () => { actions.push('ports-closed'); },
    removeOwnedWorkspace: async () => { actions.push('temp-remove'); },
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
