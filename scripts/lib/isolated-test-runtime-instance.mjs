import {
  spawn as spawnSupervisorChild,
} from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { runOwnedCommand } from './owned-command.mjs';
import { captureCheckedCommand } from './command-execution.mjs';

const STABLE_MONGO_PORT = 27017;
const START_TIMEOUT_MS = 120_000;
const STOP_TIMEOUT_MS = 20_000;
const BOOTSTRAP_SERVICE_ID = 'example.com/test-runtime-bootstrap';
const BOOTSTRAP_SERVICE_VERSION = '0.1.0';
const BOOTSTRAP_ROUTE = '/__skiff/test-runtime-bootstrap';

export function isolatedTestInstanceConfigText({ devHome, cargoTarget, basePort }) {
  return [
    `devHome: ${JSON.stringify(devHome)}`,
    `cargoTargetDir: ${JSON.stringify(cargoTarget)}`,
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

export function isolatedTestRunnerEnvironment({ baseEnv, devHome, controlPort }) {
  return {
    ...baseEnv,
    SKIFF_DEV_HOME: devHome,
    SKIFF_DEV_RELOAD_URL: `http://127.0.0.1:${controlPort}/__skiff/reload-artifacts`,
    SKIFF_TEST_ARTIFACT_ROOT: join(devHome, 'artifacts'),
  };
}

export function bootstrapDevSyncArgs({ skiffRoot, artifactRoot, buildRoot }) {
  return [
    join(skiffRoot, 'scripts', 'skiff-dev-sync.mjs'),
    '--root',
    join(skiffRoot, 'scripts', 'fixtures', 'isolated-test-bootstrap'),
    '--artifact-root',
    artifactRoot,
    '--build-root',
    buildRoot,
    '--no-reload',
  ];
}

export function isolatedRuntimeHealthReady(health, artifactRoot) {
  return Array.isArray(health?.runtimes)
    && health.runtimes.some((runtime) => runtime?.serviceId === BOOTSTRAP_SERVICE_ID)
    && Array.isArray(health?.artifact?.artifactRoots)
    && health.artifact.artifactRoots.includes(artifactRoot);
}

export function bootstrapProbeRequest(routerHttpUrl) {
  return {
    url: `${routerHttpUrl}${BOOTSTRAP_ROUTE}`,
    options: {
      headers: {
        'x-skiff-service': BOOTSTRAP_SERVICE_ID,
        'x-skiff-version': BOOTSTRAP_SERVICE_VERSION,
      },
    },
  };
}

export function isolatedInstanceOperations({ skiffRoot, baseEnv }) {
  return {
    writeConfig: async (configPath, config) => {
      await mkdir(dirname(configPath), { recursive: true });
      await writeFile(configPath, config, 'utf8');
    },
    seedBootstrap: ({ artifactRoot, buildRoot, env, signal }) => runOwnedCommand(
      'node',
      bootstrapDevSyncArgs({ skiffRoot, artifactRoot, buildRoot }),
      { cwd: skiffRoot, env, signal },
    ),
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
    stopOwnedInstance: (configPath) => runOwnedCommand(
      'node',
      [join(skiffRoot, 'scripts', 'skiff-instance.mjs'), 'down', configPath],
      { cwd: skiffRoot, env: baseEnv },
    ),
    verifyInstanceStopped: (configPath) => verifyInstanceStopped({
      skiffRoot,
      configPath,
      env: baseEnv,
    }),
  };
}

async function waitForIsolatedRuntime({
  controlUrl,
  routerHttpUrl,
  artifactRoot,
  supervisor,
  signal,
}) {
  const exit = childExit(supervisor);
  const startedAt = Date.now();
  let bootstrapResponded = false;
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
        if (bootstrapResponded && isolatedRuntimeHealthReady(health, artifactRoot)) {
          return;
        }
        if (health?.artifact?.artifactRoots?.includes(artifactRoot)) {
          const probe = bootstrapProbeRequest(routerHttpUrl);
          const probeResponse = await fetch(probe.url, { ...probe.options, signal });
          bootstrapResponded ||= probeResponse.ok;
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

async function verifyInstanceStopped({ skiffRoot, configPath, env }) {
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
  bootstrapServiceId: BOOTSTRAP_SERVICE_ID,
  bootstrapServiceVersion: BOOTSTRAP_SERVICE_VERSION,
  mongoPort: STABLE_MONGO_PORT,
};
