import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
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
  isolatedRuntimeHealthReady,
  isolatedTestInstanceConfigText,
  isolatedTestRunnerEnvironment,
} from '../lib/isolated-test-runtime-instance.mjs';
import { readInstanceConfig } from '../lib/local-instance-config.mjs';
import { leaseConsecutiveLocalPorts } from '../lib/local-port-lease.mjs';
import { runOwnedCommand } from '../lib/owned-command.mjs';
import {
  captureIsolatedTestConfig,
  claimIsolatedTestWorkspace,
} from '../lib/isolated-test-runtime-workspace.mjs';

import './isolated-test-runtime-workspace-cases.mjs';

test('isolated instance config and runner env stay inside dynamic temp boundaries', () => {
  const devHome = '/tmp/skiff-test-runtime/instance/dev-home';
  const config = isolatedTestInstanceConfigText({
    devHome,
    cargoTarget: '/checkout/build/cargo-target',
    basePort: 46042,
    mongoPort: 46045,
  });
  assert.match(config, /devHome: "\/tmp\/skiff-test-runtime\/instance\/dev-home"/);
  assert.match(config, /cargoTargetDir: "\/checkout\/build\/cargo-target"/);
  assert.match(config, /base: 46042/);
  assert.match(config, /mongo: 46045/);
  assert.match(config, /telemetry: disabled/);
  assert.match(config, /mongo: managed/);
  assert.match(config, /watch: disabled/);
  assert.doesNotMatch(config, /400[0-7]/);

  const environment = isolatedTestRunnerEnvironment({
    baseEnv: {
      PATH: '/bin',
      CARGO_TARGET_DIR: 'hostile-relative-target',
      SKIFF_DEV_RELOAD_URL: 'http://127.0.0.1:4001/stable',
      SKIFF_TEST_ARTIFACT_ROOT: '/tmp/retired-artifact-root',
      SKIFF_TEST_PLATFORM_SOURCE_ROOT: '/tmp/hostile-platform-root',
    },
    skiffRoot: '/checkout',
    cargoTarget: '/checkout/build/cargo-target',
    devHome,
    controlPort: 46043,
    routerHttpPort: 46042,
  });
  assert.equal(environment.SKIFF_DEV_HOME, devHome);
  assert.equal(environment.SKIFF_DEV_RELOAD_URL, undefined);
  assert.equal(environment.SKIFF_TEST_ARTIFACT_ROOT, undefined);
  assert.equal(environment.CARGO_TARGET_DIR, '/checkout/build/cargo-target');
  assert.equal(environment.SKIFF_TEST_RUNTIME_ARTIFACT_ROOT, `${devHome}/artifacts`);
  assert.equal(environment.SKIFF_TEST_INGRESS_URL, 'http://127.0.0.1:46042');
  assert.equal(environment.SKIFF_TEST_PLATFORM_SOURCE_ROOT, '/checkout');
});

test('isolated instance derives its ecosystem store CLI inside its owned dev-home', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-isolated-store-cli-'));
  const configPath = join(root, 'instance', 'config.yml');
  const devHome = join(root, 'instance', 'dev-home');
  try {
    await mkdir(join(root, 'instance'), { recursive: true });
    await writeFile(configPath, isolatedTestInstanceConfigText({
      devHome,
      cargoTarget: join(root, 'checkout', 'build', 'cargo-target'),
      basePort: 46042,
      mongoPort: 46045,
    }));
    const config = await readInstanceConfig({
      configPath,
      repoRoot: join(root, 'checkout'),
    });
    assert.equal(config.ports.mongo, 46045);
    assert.equal(config.components.mongo, 'managed');
    assert.equal(config.paths.serviceDbPath, join(devHome, 'service-db'));
    assert.equal(
      config.paths.ecosystemStoreCli,
      join(
        devHome,
        'bin',
        process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler',
      ),
    );
    assert.equal(config.paths.ecosystemStoreCli.startsWith(`${devHome}/`), true);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('bootstrap comes from current checkout and writes only canonical generation zero', () => {
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

test('default owner shutdown invokes current checkout instance down command', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-owner-shutdown-test-'));
  const scriptsRoot = join(root, 'scripts');
  const capturePath = join(root, 'capture.json');
  const configPath = join(root, 'instance', 'config.yml');
  try {
    let ownershipReceipt = await claimIsolatedTestWorkspace(root);
    await mkdir(scriptsRoot, { recursive: true });
    await mkdir(join(root, 'instance'), { recursive: true });
    await writeFile(configPath, 'profile: "skiff-test"\n');
    ownershipReceipt = await captureIsolatedTestConfig(ownershipReceipt, configPath);
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

    await operations.stopOwnedInstance(ownershipReceipt);

    assert.deepEqual(JSON.parse(await readFile(capturePath, 'utf8')), ['down', configPath]);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('default config creation is exclusive and preserves a foreign destination', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-config-no-clobber-test-'));
  const configPath = join(root, 'instance', 'config.yml');
  const foreignConfig = 'foreign: true\n';
  try {
    const ownershipReceipt = await claimIsolatedTestWorkspace(root);
    await mkdir(join(root, 'instance'));
    await writeFile(configPath, foreignConfig, 'utf8');
    const operations = isolatedInstanceOperations({ skiffRoot: root, baseEnv: process.env });
    await assert.rejects(
      operations.writeConfig(
        configPath,
        'profile: "skiff-test"\n',
        ownershipReceipt,
      ),
      { code: 'EEXIST' },
    );
    assert.equal(await readFile(configPath, 'utf8'), foreignConfig);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('readiness requires one exact connected replica and its own capability handshake', () => {
  const assemblyIdentity = `skiff-runtime-assembly-v3:sha256:${'1'.repeat(64)}`;
  const configSnapshotId =
    `skiff-runtime-config-snapshot-v1:${'2'.repeat(32)}`;
  const bootstrap = {
    profile: 'skiff-test',
    bootstrap: {
      generation: 0,
      assembly: { assemblyIdentity },
      configSnapshot: { snapshotId: configSnapshotId },
    },
  };
  const readyHealth = {
    activeAssembly: {
      profile: 'skiff-test',
      generation: 0,
      assemblyIdentity,
      configSnapshotId,
    },
    capabilityConnections: [{ runtimeId: 'runtime-1', connected: true }],
    replicas: [{
      replicaId: 'runtime-1',
      profile: 'skiff-test',
      connected: true,
      state: 'healthy',
      generation: 0,
      assemblyIdentity,
      configSnapshotId,
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
    ['active generation differs', (health) => { health.activeAssembly.generation = 1; }],
    ['active generation is missing', (health) => { delete health.activeAssembly.generation; }],
    ['active assembly differs', (health) => { health.activeAssembly.assemblyIdentity = 'other'; }],
    ['active assembly is missing', (health) => { delete health.activeAssembly.assemblyIdentity; }],
    ['active config snapshot differs', (health) => { health.activeAssembly.configSnapshotId = 'other'; }],
    ['active config snapshot is missing', (health) => { delete health.activeAssembly.configSnapshotId; }],
    ['replica id differs', (health) => { health.replicas[0].replicaId = 'runtime-2'; }],
    ['replica id is missing', (health) => { delete health.replicas[0].replicaId; }],
    ['replica profile differs', (health) => { health.replicas[0].profile = 'other'; }],
    ['replica profile is missing', (health) => { delete health.replicas[0].profile; }],
    ['replica connected is false', (health) => { health.replicas[0].connected = false; }],
    ['replica connected is missing', (health) => { delete health.replicas[0].connected; }],
    ['replica state is not healthy', (health) => { health.replicas[0].state = 'draining'; }],
    ['replica state is missing', (health) => { delete health.replicas[0].state; }],
    ['replica generation differs', (health) => { health.replicas[0].generation = 1; }],
    ['replica generation is missing', (health) => { delete health.replicas[0].generation; }],
    ['replica assembly differs', (health) => { health.replicas[0].assemblyIdentity = 'other'; }],
    ['replica assembly is missing', (health) => { delete health.replicas[0].assemblyIdentity; }],
    ['replica config snapshot differs', (health) => { health.replicas[0].configSnapshotId = 'other'; }],
    ['replica config snapshot is missing', (health) => { delete health.replicas[0].configSnapshotId; }],
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
  splitRuntimeHealth.replicas.push({
    ...structuredClone(readyHealth.replicas[0]),
    replicaId: 'runtime-2',
    profile: 'other',
  });
  assert.equal(
    isolatedRuntimeHealthReady(splitRuntimeHealth, bootstrap),
    false,
    'exact replica and connected capability belong to different runtimes',
  );

  const receiptMutations = [
    ['bootstrap profile is missing', (receipt) => { delete receipt.profile; }],
    ['bootstrap generation is missing', (receipt) => { delete receipt.bootstrap.generation; }],
    ['bootstrap assembly is missing', (receipt) => { delete receipt.bootstrap.assembly; }],
    ['bootstrap config snapshot is missing', (receipt) => { delete receipt.bootstrap.configSnapshot; }],
  ];
  for (const [scenario, mutate] of receiptMutations) {
    const receipt = structuredClone(bootstrap);
    mutate(receipt);
    assert.equal(isolatedRuntimeHealthReady(readyHealth, receipt), false, scenario);
  }
});

test('one absolute checkout and Cargo target flow through config, bootstrap, supervisor, and runner', async () => {
  const observed = {};
  const relativeSkiffRoot = 'relative-skiff-checkout';
  const relativeCargoTarget = 'relative-cargo-target';
  const expectedSkiffRoot = resolve(relativeSkiffRoot);
  const expectedCargoTarget = resolve(relativeCargoTarget);
  const { dependencies } = lifecycleDouble({
    writeConfig: async (_configPath, config) => { observed.config = config; },
    seedBootstrap: async (input) => {
      observed.bootstrap = input;
      observed.bootstrapReceipt = { profile: 'skiff-test', bootstrap: {} };
      return observed.bootstrapReceipt;
    },
    seedActivationState: async (input) => {
      observed.activationState = input;
    },
    spawnSupervisor: (input) => {
      observed.supervisor = input;
      return { pid: 1000 };
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
      assert.match(observed.supervisor.startupGate, /activation-seeded\.ready$/);
    },
    runTest: async (environment) => { observed.runnerEnv = environment; },
  });

  assert.equal(isAbsolute(expectedCargoTarget), true);
  assert.notEqual(expectedCargoTarget, resolve(expectedSkiffRoot, relativeCargoTarget));
  assert.equal(
    observed.config.includes(`cargoTargetDir: ${JSON.stringify(expectedCargoTarget)}`),
    true,
  );
  assert.match(observed.config, /mongo: 46003/);
  assert.doesNotMatch(observed.config, /27017/);
  assert.equal(observed.bootstrap.skiffRoot, expectedSkiffRoot);
  assert.equal(observed.bootstrap.env.CARGO_TARGET_DIR, expectedCargoTarget);
  assert.equal(observed.bootstrap.env.SKIFF_TEST_PLATFORM_SOURCE_ROOT, expectedSkiffRoot);
  assert.strictEqual(observed.validatedBootstrapReceipt, observed.bootstrapReceipt);
  assert.equal(
    observed.activationState.artifactRoot,
    '/tmp/isolated-runtime-double/instance/dev-home/artifacts',
  );
  assert.equal(observed.activationState.profile, 'skiff-test');
  assert.strictEqual(
    observed.activationState.bootstrap,
    observed.bootstrapReceipt,
  );
  assert.equal(observed.supervisor.skiffRoot, expectedSkiffRoot);
  assert.strictEqual(observed.supervisor.env, observed.bootstrap.env);
  assert.strictEqual(observed.runnerEnv, observed.bootstrap.env);
});

test('success and test failure both run owner shutdown, status, ports, lease, and temp cleanup', async () => {
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
        assert.equal(environment.SKIFF_TEST_ACTIVATION_URL.includes(':46001/'), true);
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
      'lease', 'temp', 'workspace-claim', 'source-artifacts', 'config', 'config-owner',
      'spawn', 'mongo-started', 'mongo-primary', 'bootstrap', 'activation-state',
      'startup-gate', 'ready', 'test',
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

test('false supervisor stop remains a cleanup failure while later owners still settle', async () => {
  const { actions, dependencies } = lifecycleDouble({
    stopSupervisor: async () => {
      actions.push('stop-supervisor');
      return { stopped: false };
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
    /supervisor reported stopped:false/,
  );
  assert.deepEqual(actions.slice(-5), [
    'stop-supervisor', 'instance-down', 'instance-status', 'ports-closed', 'lease-release',
  ]);
  assert.equal(actions.includes('temp-remove'), false);
});

test('write and bootstrap startup failures preserve dependency order and cleanup ownership', async () => {
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
      failureAt === 'config'
        ? /config failed.*preserve workspace with uncaptured config/s
        : /bootstrap failed/,
    );
    assert.equal(actions.includes('instance-down'), failureAt === 'bootstrap');
    assert.equal(actions.includes('instance-status'), failureAt === 'bootstrap');
    assert.equal(actions.includes('lease-release'), true);
    assert.equal(actions.includes('temp-remove'), failureAt === 'bootstrap');
  }
});

test('config capture failure preserves the workspace and still releases ports and lease', async () => {
  const { actions, dependencies } = lifecycleDouble({
    captureConfigOwnership: async () => {
      actions.push('config-owner');
      throw new Error('config identity changed before capture');
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
    /config identity changed before capture.*preserve workspace with uncaptured config/s,
  );
  assert.equal(actions.includes('instance-down'), false);
  assert.equal(actions.includes('instance-status'), false);
  assert.equal(actions.includes('ports-closed'), true);
  assert.equal(actions.includes('lease-release'), true);
  assert.equal(actions.includes('temp-remove'), false);
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

test('every ordered startup stage fails precisely and completes owned cleanup', async () => {
  const scenarios = [
    ['Mongo spawn', 'waitMongoStarted', 'mongo spawn exploded'],
    ['Mongo primary election', 'waitMongoPrimary', 'primary election exploded'],
    ['activation seed', 'seedActivationState', 'activation seed exploded'],
    ['Router/Runtime readiness', 'waitReady', 'isolated Router startup failed'],
  ];
  for (const [diagnostic, operation, detail] of scenarios) {
    const { actions, dependencies } = lifecycleDouble({
      [operation]: async () => {
        actions.push(operation === 'spawnSupervisor' ? 'spawn' : {
          waitMongoStarted: 'mongo-started',
          waitMongoPrimary: 'mongo-primary',
          seedActivationState: 'activation-state',
          waitReady: 'ready',
        }[operation]);
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
    assert.deepEqual(actions.slice(-6), [
      'stop-supervisor', 'instance-down', 'instance-status', 'ports-closed',
      'lease-release', 'temp-remove',
    ]);
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
    writeConfig: async () => { actions.push('config'); },
    captureConfigOwnership: async (receipt, configPath) => {
      actions.push('config-owner');
      return {
        ...receipt,
        config: { path: configPath, identity: { dev: '1', ino: '4' } },
      };
    },
    seedBootstrap: async () => { actions.push('bootstrap'); },
    spawnSupervisor: () => {
      actions.push('spawn');
      return { pid: 1000 };
    },
    waitMongoStarted: async () => { actions.push('mongo-started'); },
    waitMongoPrimary: async () => { actions.push('mongo-primary'); },
    seedActivationState: async () => { actions.push('activation-state'); },
    releaseStartupGate: async () => { actions.push('startup-gate'); },
    waitReady: async () => { actions.push('ready'); },
    stopSupervisor: async () => { actions.push('stop-supervisor'); },
    stopOwnedInstance: async () => { actions.push('instance-down'); },
    verifyInstanceStopped: async () => { actions.push('instance-status'); },
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
