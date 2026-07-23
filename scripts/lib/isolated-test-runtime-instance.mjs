import {
  spawn as spawnSupervisorChild,
} from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { runOwnedCommand } from './owned-command.mjs';
import { captureCheckedCommand } from './command-execution.mjs';
import { assertIsolatedTestWorkspaceOwned } from './isolated-test-runtime-workspace.mjs';

const STABLE_MONGO_PORT = 27017;
const START_TIMEOUT_MS = 120_000;
const STOP_TIMEOUT_MS = 20_000;

export function isolatedTestInstanceConfigText({
  devHome,
  cargoTarget,
  basePort,
  environment = 'skiff-test',
}) {
  return [
    `devHome: ${JSON.stringify(devHome)}`,
    `cargoTargetDir: ${JSON.stringify(cargoTarget)}`,
    `environment: ${JSON.stringify(environment)}`,
    'packageDirs:',
    'ports:',
    `  base: ${basePort}`,
    `  mongo: ${STABLE_MONGO_PORT}`,
    'components:',
    '  telemetry: disabled',
    '  mongo: disabled',
    '  watch: disabled',
    'telemetry:',
    '  memory: true',
    'mongo:',
    '  binary: mongod',
    '  dbPath: service-db',
    'watch:',
    '  config: watch.json',
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
  environment = 'skiff-test',
}) {
  const cleanBaseEnv = { ...baseEnv };
  delete cleanBaseEnv.SKIFF_DEV_RELOAD_URL;
  delete cleanBaseEnv.SKIFF_TEST_ARTIFACT_ROOT;
  return {
    ...cleanBaseEnv,
    CARGO_TARGET_DIR: cargoTarget,
    SKIFF_DEV_HOME: devHome,
    SKIFF_TEST_RUNTIME_ARTIFACT_ROOT: join(devHome, 'artifacts'),
    SKIFF_TEST_ACTIVATION_URL: `http://127.0.0.1:${controlPort}/__skiff/activate-assembly`,
    SKIFF_TEST_INGRESS_URL: `http://127.0.0.1:${routerHttpPort}`,
    SKIFF_TEST_ENVIRONMENT: environment,
    SKIFF_TEST_EXPECTED_GENERATION: '0',
    SKIFF_TEST_PLATFORM_SOURCE_ROOT: skiffRoot,
  };
}

export function bootstrapCanonicalArgs({
  skiffRoot,
  artifactRoot,
  environment = 'skiff-test',
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
    '--bootstrap-only',
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    resolve(skiffRoot),
    '--environment',
    environment,
  ];
}

export function isolatedRuntimeHealthReady(health, bootstrapReceipt) {
  const bootstrap = bootstrapReceipt?.bootstrap;
  const environment = bootstrapReceipt?.environment;
  const generation = bootstrap?.generation;
  const assemblyIdentity = bootstrap?.assembly?.assemblyIdentity;
  const active = health?.activeAssembly;
  if (
    bootstrap === undefined
    || typeof environment !== 'string'
    || environment.length === 0
    || !Number.isSafeInteger(generation)
    || generation < 0
    || typeof assemblyIdentity !== 'string'
    || assemblyIdentity.length === 0
    || active?.environment !== environment
    || active?.generation !== generation
    || active?.assemblyIdentity !== assemblyIdentity
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
      && replica?.environment === environment
      && replica?.generation === generation
      && replica?.assemblyIdentity === assemblyIdentity
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
    seedBootstrap: async ({ artifactRoot, environment, env, signal }) => {
      const result = await captureCheckedCommand(
        'cargo',
        bootstrapCanonicalArgs({ skiffRoot, artifactRoot, environment }),
        { cwd: skiffRoot, env, signal },
      );
      return JSON.parse(result.stdout);
    },
    spawnSupervisor: ({ configPath, env }) => {
      // child-process-owner: isolated-supervisor
      return spawnSupervisorChild(
        'node',
        [join(skiffRoot, 'scripts', 'skiff-instance.mjs'), 'supervise', configPath],
        { cwd: skiffRoot, env, stdio: 'inherit' },
      );
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
          ownershipReceipt.config.path,
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

async function waitForIsolatedRuntime({
  controlUrl,
  bootstrap,
  supervisor,
  signal,
}) {
  const exit = childExit(supervisor);
  const startedAt = Date.now();
  let lastError;
  while (Date.now() - startedAt < START_TIMEOUT_MS) {
    signal.throwIfAborted();
    if (supervisor.exitCode !== null || supervisor.signalCode !== null) {
      throw new Error(`isolated runtime supervisor exited before readiness with ${supervisor.signalCode ?? supervisor.exitCode}`);
    }
    try {
      const response = await fetch(`${controlUrl}/__router/health`, { signal });
      if (response.ok) {
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
  throw new Error(
    `isolated runtime did not become ready at ${controlUrl} within ${START_TIMEOUT_MS}ms${lastError ? `: ${errorMessage(lastError)}` : ''}`,
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
    configPath,
    '--json',
  ], { cwd: skiffRoot, env });
  const status = JSON.parse(result.stdout);
  const active = (status.processes ?? []).filter((processStatus) =>
    ['router', 'runtime'].includes(processStatus.name)
    && processStatus.category !== 'stopped');
  if (active.length > 0) {
    throw new Error(`isolated instance still owns active components: ${active.map((entry) => `${entry.name}:${entry.category}`).join(', ')}`);
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

export const isolatedTestInstanceConstants = {
  mongoPort: STABLE_MONGO_PORT,
};
