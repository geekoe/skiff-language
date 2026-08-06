import {
  spawn as spawnSupervisorChild,
} from 'node:child_process';
import { access, mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { runOwnedCommand } from './owned-command.mjs';
import { captureCheckedCommand } from './command-execution.mjs';
import { assertIsolatedTestWorkspaceOwned } from './isolated-test-runtime-workspace.mjs';
import { renderRouterConfig, renderRuntimeConfig } from './runtime-stack-config.mjs';
import { ensureLocalServiceDbKeyring } from './service-db-keyring.mjs';

const START_TIMEOUT_MS = 120_000;
const STOP_TIMEOUT_MS = 20_000;

export function isolatedTestInstanceYml({
  devHome,
  basePort,
  mongoPort,
  profile = 'skiff-test',
  routerBinary,
  runtimeBinary,
  mongoBinary = 'mongod',
}) {
  if (!Number.isSafeInteger(mongoPort) || mongoPort <= 0) {
    throw new Error('isolated test instance mongoPort must be a positive integer');
  }
  const httpPort = basePort;
  const controlPort = basePort + 1;
  const pidDir = join(devHome, 'pids');
  const logDir = join(devHome, 'logs');
  return [
    'schemaVersion: skiff-instance-v1',
    `profile: ${JSON.stringify(profile)}`,
    `devHome: ${JSON.stringify(devHome)}`,
    `artifactRoot: ${JSON.stringify(join(devHome, 'artifacts'))}`,
    `pidDir: ${JSON.stringify(pidDir)}`,
    `logDir: ${JSON.stringify(logDir)}`,
    `mongoDbPath: ${JSON.stringify(join(devHome, 'mongo-data'))}`,
    'processes:',
    '  - name: mongo',
    `    command: ${JSON.stringify(mongoBinary)}`,
    '    args:',
    `      - ${JSON.stringify('--dbpath')}`,
    `      - ${JSON.stringify(join(devHome, 'mongo-data'))}`,
    '      - --port',
    `      - ${JSON.stringify(String(mongoPort))}`,
    '      - --replSet',
    '      - rs0',
    '      - --bind_ip',
    '      - 127.0.0.1',
    `    cwd: ${JSON.stringify(devHome)}`,
    `    ports: [${mongoPort}]`,
    '    healthUrl: null',
    '  - name: router',
    `    command: ${JSON.stringify(routerBinary)}`,
    '    args:',
    `      - ${JSON.stringify(join(devHome, 'router.yml'))}`,
    `    cwd: ${JSON.stringify(devHome)}`,
    `    ports: [${httpPort}, ${controlPort}]`,
    `    healthUrl: ${JSON.stringify(`http://127.0.0.1:${controlPort}/__router/health`)}`,
    '  - name: runtime',
    `    command: ${JSON.stringify(runtimeBinary)}`,
    '    args:',
    `      - ${JSON.stringify(join(devHome, 'runtime.yml'))}`,
    `    cwd: ${JSON.stringify(devHome)}`,
    '    ports: []',
    '    healthUrl: null',
    '',
  ].join('\n');
}

export function isolatedTestRunnerEnvironment({
  baseEnv,
  skiffRoot,
  cargoTarget,
  devHome,
  controlPort,
  routerHttpPort,
  profile = 'skiff-test',
}) {
  const cleanBaseEnv = { ...baseEnv };
  delete cleanBaseEnv.SKIFF_DEV_RELOAD_URL;
  delete cleanBaseEnv.SKIFF_TEST_ARTIFACT_ROOT;
  return {
    ...cleanBaseEnv,
    CARGO_TARGET_DIR: cargoTarget,
    SKIFF_DEV_HOME: devHome,
    SKIFF_TEST_RUNTIME_ARTIFACT_ROOT: join(devHome, 'artifacts'),
    SKIFF_TEST_INGRESS_URL: `http://127.0.0.1:${routerHttpPort}`,
    SKIFF_TEST_ENVIRONMENT: profile,
    SKIFF_TEST_PLATFORM_SOURCE_ROOT: skiffRoot,
  };
}

export function isolatedTestInstanceRuntimeFiles({
  profile,
  devHome,
  basePort,
  mongoPort,
}) {
  if (!Number.isSafeInteger(basePort) || basePort <= 0) {
    throw new Error('isolated test instance basePort must be a positive integer');
  }
  if (!Number.isSafeInteger(mongoPort) || mongoPort <= 0) {
    throw new Error('isolated test instance mongoPort must be a positive integer');
  }
  const controlPort = basePort + 1;
  const secretsDir = join(devHome, 'secrets');
  return {
    routerConfigPath: join(devHome, 'router.yml'),
    runtimeConfigPath: join(devHome, 'runtime.yml'),
    keyringPath: join(secretsDir, 'service-db-keyring.json'),
    dirs: {
      mongoData: join(devHome, 'mongo-data'),
      pids: join(devHome, 'pids'),
      logs: join(devHome, 'logs'),
      secrets: secretsDir,
      runtimeHome: join(devHome, 'runtime-home'),
      artifacts: join(devHome, 'artifacts'),
    },
    routerConfig: renderRouterConfig({
      profile,
      host: '127.0.0.1',
      artifactsPath: join(devHome, 'artifacts'),
      devReload: true,
      requestTimeoutMs: 20000,
      httpPort: basePort,
      httpMaxRequestBytes: 67108864,
      httpMaxResponseBytes: 8388608,
      runtimePort: controlPort,
      runtimePath: '/runtime',
      serviceDbMongoUrl:
        `mongodb://127.0.0.1:${mongoPort}/?directConnection=true&replicaSet=rs0&retryWrites=false`,
    }),
    runtimeConfig: renderRuntimeConfig({
      routerUrl: `ws://127.0.0.1:${controlPort}/runtime`,
      runtimeHome: join(devHome, 'runtime-home'),
      serviceDbEncryptionKeyringFile: join(secretsDir, 'service-db-keyring.json'),
    }),
  };
}

export function bootstrapCanonicalArgs({
  skiffRoot,
  artifactRoot,
  profile = 'skiff-test',
}) {
  return [
    'run',
    '--quiet',
    '--locked',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-package-service-smoke-fixture',
    '--',
    // The isolated instance boots from an empty pointer table; the dedicated
    // bootstrap fixture seeds the canonical generation-0 records (std + the
    // bootstrap service deployment and its release pointer) into the store
    // before the Router starts.
    '--seed-committed',
    join(skiffRoot, 'test-runner', 'fixtures', 'isolated-test-bootstrap'),
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    resolve(skiffRoot),
    '--profile',
    profile,
  ];
}

export function isolatedRuntimeHealthReady(health, bootstrapReceipt) {
  const bootstrap = bootstrapReceipt?.bootstrap;
  const profile = bootstrapReceipt?.profile;
  const active = health?.activeAssembly;
  if (
    bootstrap === undefined
    || typeof profile !== 'string'
    || profile.length === 0
    || active?.profile !== profile
  ) {
    return false;
  }
  const capabilityConnections = health?.capabilityConnections;
  const replicas = health?.replicas;
  return Array.isArray(capabilityConnections)
    && Array.isArray(replicas)
    && replicas.some((replica) => (
      typeof replica?.replicaId === 'string'
      && replica.replicaId.length > 0
      && replica.connected === true
      && replica?.state === 'healthy'
      && replica?.profile === profile
      && capabilityConnections.some((connection) => (
        connection?.connected === true
        && connection?.runtimeId === replica.replicaId
      ))
    ));
}

export function isolatedInstanceOperations({ skiffRoot, baseEnv }) {
  return {
    writeConfig: async (configPath, config, ownershipReceipt) => {
      await assertIsolatedTestWorkspaceOwned(ownershipReceipt);
      await mkdir(dirname(configPath), { recursive: true });
      await assertIsolatedTestWorkspaceOwned(ownershipReceipt);
      await writeFile(configPath, config, {
        encoding: 'utf8',
        flag: 'wx',
        mode: 0o600,
      });
    },
    seedBootstrap: async ({ artifactRoot, profile, env, signal }) => {
      const result = await captureCheckedCommand(
        'cargo',
        bootstrapCanonicalArgs({ skiffRoot, artifactRoot, profile }),
        { cwd: skiffRoot, env, signal },
      );
      return JSON.parse(result.stdout);
    },
    spawnSupervisor: ({ configPath, startupGate, startupReady, env }) => {
      // child-process-owner: isolated-supervisor
      return spawnSupervisorChild(
        'node',
        [
          join(skiffRoot, 'scripts', 'skiff-instance.mjs'),
          'supervise',
          '--runtime',
          dirname(configPath),
          '--startup-gate',
          startupGate,
          '--startup-ready',
          startupReady,
        ],
        { cwd: skiffRoot, env, stdio: 'inherit' },
      );
    },
    initializeInstance: async ({
      profile,
      devHome,
      basePort,
      mongoPort,
      ownershipReceipt,
    }) => {
      await assertIsolatedTestWorkspaceOwned(ownershipReceipt, { requireConfig: true });
      const files = isolatedTestInstanceRuntimeFiles({
        profile,
        devHome,
        basePort,
        mongoPort,
      });
      for (const [name, directory] of Object.entries(files.dirs)) {
        await mkdir(directory, {
          recursive: true,
          mode: name === 'secrets' ? 0o700 : undefined,
        });
      }
      await ensureLocalServiceDbKeyring(files.keyringPath);
      await writeFile(files.routerConfigPath, files.routerConfig, {
        encoding: 'utf8',
        mode: 0o600,
      });
      await writeFile(files.runtimeConfigPath, files.runtimeConfig, {
        encoding: 'utf8',
        mode: 0o600,
      });
    },
    waitMongoStarted,
    waitMongoPrimary: initializeSingleNodeReplicaSet,
    releaseStartupGate: async (startupGate, ownershipReceipt) => {
      await assertIsolatedTestWorkspaceOwned(ownershipReceipt, { requireConfig: true });
      await writeFile(startupGate, 'bootstrap-seeded\n', {
        encoding: 'utf8',
        flag: 'wx',
        mode: 0o600,
      });
    },
    waitReady: waitForIsolatedRuntime,
    stopSupervisor,
    stopOwnedInstance: async (ownershipReceipt) => {
      await assertIsolatedTestWorkspaceOwned(ownershipReceipt, { requireConfig: true });
      return runOwnedCommand(
        'node',
        [
          join(skiffRoot, 'scripts', 'skiff-instance.mjs'),
          'down',
          '--runtime',
          dirname(ownershipReceipt.config.path),
        ],
        { cwd: skiffRoot, env: baseEnv },
      );
    },
    verifyInstanceStopped: (ownershipReceipt) => verifyInstanceStopped({
      skiffRoot,
      ownershipReceipt,
      env: baseEnv,
    }),
  };
}

async function waitMongoStarted({ startupReady, supervisor, signal }) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < START_TIMEOUT_MS) {
    signal.throwIfAborted();
    if (supervisor.exitCode !== null || supervisor.signalCode !== null) {
      throw new Error(
        `isolated MongoDB supervisor exited during spawn with ${
          supervisor.signalCode ?? supervisor.exitCode
        }`,
      );
    }
    try {
      await access(startupReady);
      return;
    } catch {
      await delay(50);
    }
  }
  throw new Error(`isolated MongoDB did not report a successful spawn within ${START_TIMEOUT_MS}ms`);
}

async function waitForIsolatedRuntime({
  controlUrl,
  bootstrap,
  supervisor,
  signal,
}) {
  const exit = childExit(supervisor);
  const startedAt = Date.now();
  let lastError;
  let routerReady = false;
  while (Date.now() - startedAt < START_TIMEOUT_MS) {
    signal.throwIfAborted();
    if (supervisor.exitCode !== null || supervisor.signalCode !== null) {
      throw new Error(`isolated Router/Runtime supervisor exited before readiness with ${supervisor.signalCode ?? supervisor.exitCode}`);
    }
    try {
      const response = await fetch(`${controlUrl}/__router/health`, { signal });
      if (response.ok) {
        routerReady = true;
        const health = await response.json();
        if (isolatedRuntimeHealthReady(health, bootstrap)) {
          return;
        }
      }
    } catch (error) {
      lastError = error;
    }
    await Promise.race([delay(100), exit.then(() => undefined)]);
  }
  const component = routerReady ? 'Runtime' : 'Router';
  throw new Error(
    `isolated ${component} startup failed at ${controlUrl} within ${START_TIMEOUT_MS}ms${lastError ? `: ${errorMessage(lastError)}` : ''}`,
  );
}

async function initializeSingleNodeReplicaSet({ mongoPort, supervisor, signal }) {
  const uri = `mongodb://127.0.0.1:${mongoPort}/admin?directConnection=true`;
  const initiate = [
    'try {',
    '  const status = rs.status();',
    '  if (status.myState !== 1) quit(2);',
    '} catch (error) {',
    `  rs.initiate({_id:'rs0',members:[{_id:0,host:'127.0.0.1:${mongoPort}'}]});`,
    '  quit(2);',
    '}',
  ].join(' ');
  const startedAt = Date.now();
  let lastError;
  while (Date.now() - startedAt < START_TIMEOUT_MS) {
    signal.throwIfAborted();
    if (supervisor.exitCode !== null || supervisor.signalCode !== null) {
      throw new Error('isolated MongoDB supervisor exited before primary election');
    }
    try {
      await captureCheckedCommand(
        'mongosh',
        [uri, '--quiet', '--eval', initiate],
        { signal },
      );
      return;
    } catch (error) {
      lastError = error;
    }
    await delay(100);
  }
  throw new Error(
    `isolated MongoDB did not elect its single-node primary: ${errorMessage(lastError)}`,
  );
}

async function stopSupervisor(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  const exit = childExit(child);
  child.kill('SIGTERM');
  const stopped = await Promise.race([
    exit.then(() => true),
    delay(STOP_TIMEOUT_MS).then(() => false),
  ]);
  if (stopped) {
    return;
  }
  child.kill('SIGKILL');
  const killed = await Promise.race([
    exit.then(() => true),
    delay(5_000).then(() => false),
  ]);
  if (!killed) {
    throw new Error(`isolated runtime supervisor pid ${child.pid} did not stop`);
  }
}

async function verifyInstanceStopped({ skiffRoot, ownershipReceipt, env }) {
  await assertIsolatedTestWorkspaceOwned(ownershipReceipt, { requireConfig: true });
  const configPath = ownershipReceipt.config.path;
  const result = await runCommandCapture('node', [
    join(skiffRoot, 'scripts', 'skiff-instance.mjs'),
    'status',
    '--runtime',
    dirname(configPath),
    '--json',
  ], { cwd: skiffRoot, env });
  const status = JSON.parse(result.stdout);
  const active = (status.processes ?? []).filter((processStatus) =>
    ['mongo', 'router', 'runtime'].includes(processStatus.name)
    && processStatus.alive === true);
  if (active.length > 0) {
    throw new Error(`isolated instance still owns active components: ${active.map((entry) => `${entry.name}:${entry.pid ?? 'unknown'}`).join(', ')}`);
  }
}

function childExit(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve({ code: child.exitCode, signal: child.signalCode });
  }
  return new Promise((resolvePromise, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolvePromise({ code, signal }));
  });
}

async function runCommandCapture(command, args, options) {
  try {
    return await captureCheckedCommand(command, args, options);
  } catch (error) {
    throw new Error([
      `${command} exited with ${error?.signal ?? error?.code ?? 'UNKNOWN'}`,
      streamDiagnostic('stderr', error?.stderr),
      streamDiagnostic('stdout', error?.stdout),
    ].filter(Boolean).join('\n'));
  }
}

function streamDiagnostic(label, value) {
  return typeof value === 'string' && value.trim().length > 0
    ? `${label}:\n${value.trim()}`
    : '';
}

function errorMessage(error) {
  return error?.message || String(error);
}
