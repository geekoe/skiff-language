import {
  spawn as spawnIsolatedChild,
} from 'node:child_process';
import { access, mkdir, open, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { captureCheckedCommand } from './command-execution.mjs';
import { assertIsolatedTestWorkspaceOwned } from './isolated-test-runtime-workspace.mjs';
import { renderRouterConfig, renderRuntimeConfig } from './runtime-stack-config.mjs';
import { ensureLocalServiceDbKeyring } from './service-db-keyring.mjs';

const START_TIMEOUT_MS = 120_000;
const STOP_TIMEOUT_MS = 20_000;

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
    SKIFF_TEST_CONTROL_URL: `http://127.0.0.1:${controlPort}`,
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
    // The isolated stack boots from an empty pointer table; the dedicated
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
      && capabilityConnections.some((connection) => (
        connection?.connected === true
        && connection?.runtimeId === replica.replicaId
      ))
    ));
}

export function isolatedInstanceOperations({ skiffRoot, baseEnv }) {
  return {
    seedBootstrap: async ({ artifactRoot, profile, env, signal }) => {
      const result = await captureCheckedCommand(
        'cargo',
        bootstrapCanonicalArgs({ skiffRoot, artifactRoot, profile }),
        { cwd: skiffRoot, env, signal },
      );
      return JSON.parse(result.stdout);
    },
    initializeInstance: async ({
      profile,
      devHome,
      basePort,
      mongoPort,
      ownershipReceipt,
    }) => {
      await assertIsolatedTestWorkspaceOwned(ownershipReceipt, { requireConfig: false });
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
    spawnMongo: async ({ mongoBinary, mongoPort, mongoDataDir, cwd, env, logDir }) => {
      // child-process-owner: isolated-mongo
      const stdio = await logFileStdio(logDir, 'mongo');
      return spawnIsolatedChild(
        mongoBinary,
        [
          '--dbpath', mongoDataDir,
          '--port', String(mongoPort),
          '--replSet', 'rs0',
          '--bind_ip', '127.0.0.1',
        ],
        { cwd, env, stdio },
      );
    },
    waitMongoPrimary: async ({ mongoPort, child, signal }) => {
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
        if (child.exitCode !== null || child.signalCode !== null) {
          throw new Error(`isolated MongoDB exited before primary election with ${child.signalCode ?? child.exitCode}`);
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
    },
    spawnRouter: async ({ routerBinary, routerConfigPath, cwd, env, logDir }) => {
      // child-process-owner: isolated-router
      const stdio = await logFileStdio(logDir, 'router');
      return spawnIsolatedChild(
        routerBinary,
        [routerConfigPath],
        { cwd, env, stdio },
      );
    },
    spawnRuntime: async ({ runtimeBinary, runtimeConfigPath, cwd, env, logDir }) => {
      // child-process-owner: isolated-runtime
      const stdio = await logFileStdio(logDir, 'runtime');
      return spawnIsolatedChild(
        runtimeBinary,
        [runtimeConfigPath],
        { cwd, env, stdio },
      );
    },
    waitReady: waitForIsolatedRuntime,
    stopProcesses: stopIsolatedChildren,
  };
}

async function waitForIsolatedRuntime({
  controlUrl,
  bootstrap,
  children,
  signal,
}) {
  const startedAt = Date.now();
  let lastError;
  let routerReady = false;
  while (Date.now() - startedAt < START_TIMEOUT_MS) {
    signal.throwIfAborted();
    for (const child of children) {
      if (child.exitCode !== null || child.signalCode !== null) {
        throw new Error(
          `isolated process ${child.pid} (${child.spawnargs?.[0] ?? 'unknown'}) exited before readiness with ${child.signalCode ?? child.exitCode}`,
        );
      }
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
    await delay(100);
  }
  const component = routerReady ? 'Runtime' : 'Router';
  throw new Error(
    `isolated ${component} startup failed at ${controlUrl} within ${START_TIMEOUT_MS}ms${lastError ? `: ${errorMessage(lastError)}` : ''}`,
  );
}

async function stopIsolatedChildren(children) {
  const errors = [];
  for (const child of children) {
    if (child.exitCode !== null || child.signalCode !== null) {
      continue;
    }
    const exit = childExit(child);
    child.kill('SIGTERM');
    const stopped = await Promise.race([
      exit.then(() => true),
      delay(STOP_TIMEOUT_MS).then(() => false),
    ]);
    if (stopped) {
      continue;
    }
    child.kill('SIGKILL');
    const killed = await Promise.race([
      exit.then(() => true),
      delay(5_000).then(() => false),
    ]);
    if (!killed) {
      errors.push(new Error(`isolated process pid ${child.pid} did not stop`));
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'isolated stack shutdown failed');
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

async function logFileStdio(logDir, name) {
  await mkdir(logDir, { recursive: true });
  const out = await open(join(logDir, `${name}.out.log`), 'a');
  const err = await open(join(logDir, `${name}.err.log`), 'a');
  return ['ignore', out.fd, err.fd];
}

function errorMessage(error) {
  return error?.message || String(error);
}
