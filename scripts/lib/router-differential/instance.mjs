// Per-side isolated instance orchestration for the differential harness.
//
// Each side (TS and Rust) gets its own leased port block, its own devHome /
// artifact root / runtime home, its own temporary mongod namespace and its
// own real Runtime process. The Router process is resolved exclusively
// through the canonical RouterProcessSpec seam (§5.1), never hardcoded.

import { spawn } from 'node:child_process';
import {
  access,
  copyFile,
  mkdir,
  open,
  readFile,
  writeFile,
} from 'node:fs/promises';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import {
  assertRouterProcessSpec,
  resolveRouterProcessSpec,
  routerBinaryName,
  routerProcessInvocation,
} from '../dev-runtime-paths.mjs';
import { assertPortsClosed } from '../local-port-lease.mjs';
import { renderRouterConfig, renderRuntimeConfig } from '../runtime-stack-config.mjs';

import {
  ACTIVATION_STATE_SCHEMA_VERSION,
  ENVIRONMENT,
  GENERATION,
  REPLICA_ID,
  RUST_AUDIT_COLLECTION,
  RUST_DATABASE,
  RUST_STATE_COLLECTION,
  TS_AUDIT_COLLECTION,
  TS_DATABASE,
  TS_STATE_COLLECTION,
} from './constants.mjs';
import {
  countAuditEntries,
  createDifferentialMongosh,
  readActivationState,
  seedActivationState,
} from './mongo.mjs';
import { createRuntimeRelay } from './relay.mjs';

const LISTENER_TIMEOUT_MS = 30_000;
const HANDSHAKE_TIMEOUT_MS = 60_000;
const STOP_TIMEOUT_MS = 15_000;
const HANDSHAKE_SETTLE_MS = 500;

export async function createSideContext({
  repoRoot,
  implementation,
  tempRoot,
  ports,
  mongoPort,
  artifactRoot,
  assemblyIdentity,
  configSnapshotId,
  runtimeBin,
  routerSourceBinary,
  environment = ENVIRONMENT,
  generation = GENERATION,
  replicaId = REPLICA_ID,
}) {
  const [httpPort, runtimePort, relayPort] = ports;
  const sideRoot = join(tempRoot, implementation);
  const devHome = join(sideRoot, 'dev-home');
  const runtimeHome = join(sideRoot, 'runtime-home');
  const mongosh = createDifferentialMongosh();
  const database = implementation === 'ts' ? TS_DATABASE : RUST_DATABASE;
  const stateCollection = implementation === 'ts'
    ? TS_STATE_COLLECTION
    : RUST_STATE_COLLECTION;
  const auditCollection = implementation === 'ts'
    ? TS_AUDIT_COLLECTION
    : RUST_AUDIT_COLLECTION;
  const mongoUrl = implementation === 'ts'
    ? `mongodb://127.0.0.1:${mongoPort}/${database}?directConnection=true&replicaSet=rs0&retryWrites=false`
    : `mongodb://127.0.0.1:${mongoPort}/?directConnection=true&replicaSet=rs0&retryWrites=false`;

  const state = {
    schemaVersion: ACTIVATION_STATE_SCHEMA_VERSION,
    environment,
    committed: {
      generation,
      assembly: { assemblyIdentity },
      configSnapshot: { snapshotId: configSnapshotId },
    },
    pending: null,
  };

  return {
    implementation,
    repoRoot,
    sideRoot,
    devHome,
    runtimeHome,
    artifactRoot,
    httpPort,
    runtimePort,
    relayPort,
    mongoPort,
    mongoUrl,
    database,
    stateCollection,
    auditCollection,
    mongosh,
    environment,
    generation,
    replicaId,
    state,
    ports,
    runtimeBin,
    routerSourceBinary,
    routerConfigPath: join(devHome, 'router.yml'),
    runtimeConfigPath: join(sideRoot, 'runtime.yml'),
    routerLogs: {
      stdout: join(sideRoot, 'router.stdout.log'),
      stderr: join(sideRoot, 'router.stderr.log'),
    },
    runtimeLogs: {
      stdout: join(sideRoot, 'runtime.stdout.log'),
      stderr: join(sideRoot, 'runtime.stderr.log'),
    },
  };
}

export async function startDifferentialSide(side) {
  const {
    implementation,
    repoRoot,
    sideRoot,
    devHome,
    runtimeHome,
    artifactRoot,
    httpPort,
    runtimePort,
    relayPort,
    mongoUrl,
    stateCollection,
    auditCollection,
    mongosh,
    environment,
    generation,
    replicaId,
    state,
    runtimeBin,
    routerConfigPath,
    runtimeConfigPath,
    routerLogs,
    runtimeLogs,
  } = side;

  await mkdir(join(devHome, 'bin'), { recursive: true });
  await mkdir(runtimeHome, { recursive: true });

  const routerConfig = renderRouterConfig({
    profile: 'dev',
    host: '127.0.0.1',
    environment,
    artifactsPath: artifactRoot,
    releaseMode: true,
    requestTimeoutMs: 20_000,
    activationPrepareTimeoutMs: 120_000,
    httpPort,
    httpMaxRequestBytes: 1_048_576,
    httpMaxResponseBytes: 1_048_576,
    runtimePort,
    runtimePath: '/runtime',
    runtimeMaxConcurrency: 128,
    serviceDbMongoUrl: mongoUrl,
  });
  await writeFile(routerConfigPath, routerConfig, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });

  if (implementation === 'rust') {
    const installed = join(devHome, 'bin', routerBinaryName());
    await copyFile(side.routerSourceBinary, installed);
    await access(installed);
  }

  const spec = assertRouterProcessSpec(resolveRouterProcessSpec({
    devHome,
    implementation,
    repoRoot,
  }));
  side.routerSpec = spec;
  const invocation = routerProcessInvocation(spec);
  side.routerInvocation = invocation;

  await seedActivationState({
    mongosh,
    mongoUrl,
    database: side.database,
    collection: stateCollection,
    environment,
    state,
  });
  await access(runtimeBin);
  await writeFile(join(runtimeHome, 'runtime-id'), `${replicaId}\n`, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });

  const runtimeConfig = renderRuntimeConfig({
    routerUrl: `ws://127.0.0.1:${relayPort}/runtime`,
    runtimeHome,
    environment,
  });
  await writeFile(runtimeConfigPath, runtimeConfig, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });

  side.routerProcess = await spawnWithLogs(
    invocation.command,
    invocation.args,
    { cwd: repoRoot, logs: routerLogs },
  );
  side.runtimeProcess = undefined;
  side.relay = undefined;

  await waitForListeners({
    httpPort,
    runtimePort,
    child: side.routerProcess.child,
    stderrPath: routerLogs.stderr,
  });

  side.relay = await createRuntimeRelay({
    port: relayPort,
    routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
  });

  side.runtimeProcess = await spawnWithLogs(
    runtimeBin,
    [runtimeConfigPath],
    { cwd: repoRoot, logs: runtimeLogs },
  );

  await side.relay.waitForHandshake({ timeoutMs: HANDSHAKE_TIMEOUT_MS });
  await delay(HANDSHAKE_SETTLE_MS);
}

export async function captureDifferentialSide(side) {
  const fetchOptions = { signal: AbortSignal.timeout(10_000) };
  const controlHealth = await fetch(
    `http://127.0.0.1:${side.runtimePort}/__router/health`,
    fetchOptions,
  );
  const controlHealthStatus = controlHealth.status;
  const controlHealthBody = await controlHealth.text();
  const publicStatus = await fetch(
    `http://127.0.0.1:${side.httpPort}/`,
    fetchOptions,
  );

  const stateDocument = await readActivationState({
    mongosh: side.mongosh,
    mongoUrl: side.mongoUrl,
    database: side.database,
    collection: side.stateCollection,
    environment: side.environment,
  });
  const auditCount = await countAuditEntries({
    mongosh: side.mongosh,
    mongoUrl: side.mongoUrl,
    database: side.database,
    collection: side.auditCollection,
  });

  return {
    http: {
      publicStatus: publicStatus.status,
      controlHealthStatus,
      controlHealthBody,
    },
    runtimeFrames: side.relay.records.map((record) => ({ ...record })),
    mongo: {
      state: stateDocument,
      auditCount,
    },
  };
}

export async function stopDifferentialSide(side) {
  const errors = [];
  if (side.runtimeProcess !== undefined) {
    try {
      side.runtimeExit = await stopChild(side.runtimeProcess.child, 'SIGINT', {
        label: `${side.implementation} runtime`,
      });
    } catch (error) {
      errors.push(error);
    }
  }
  if (side.routerProcess !== undefined) {
    try {
      side.routerExit = await stopChild(side.routerProcess.child, 'SIGTERM', {
        label: `${side.implementation} router`,
      });
    } catch (error) {
      errors.push(error);
    }
  }
  if (side.relay !== undefined) {
    try {
      await side.relay.close();
    } catch (error) {
      errors.push(error);
    }
  }
  for (const child of [side.routerProcess, side.runtimeProcess]) {
    if (child !== undefined) {
      for (const handle of [child.stdoutLog, child.stderrLog]) {
        try {
          await handle.close();
        } catch (error) {
          errors.push(error);
        }
      }
    }
  }
  try {
    await assertPortsClosed([side.httpPort, side.runtimePort, side.relayPort]);
    side.portsClosed = true;
  } catch (error) {
    side.portsClosed = false;
    errors.push(error);
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, `${side.implementation} differential side stop failed`);
  }
}

export async function readSideLogs(side) {
  return {
    routerStdout: await readFile(side.routerLogs.stdout, 'utf8'),
    routerStderr: await readFile(side.routerLogs.stderr, 'utf8'),
    runtimeStdout: await readFile(side.runtimeLogs.stdout, 'utf8'),
    runtimeStderr: await readFile(side.runtimeLogs.stderr, 'utf8'),
  };
}

export function terminalObservation(side) {
  return {
    routerExitCode: side.routerExit?.code ?? null,
    runtimeExitCode: side.runtimeExit?.code ?? null,
    portsClosed: side.portsClosed === true,
  };
}

export function sideContextObservation(side) {
  return {
    artifactsPath: side.artifactRoot,
    mongoUrl: side.mongoUrl,
    httpPort: side.httpPort,
    runtimePort: side.runtimePort,
    relayPort: side.relayPort,
    devHome: side.devHome,
    runtimeHome: side.runtimeHome,
    ports: side.ports,
  };
}

async function spawnWithLogs(command, args, { cwd, logs }) {
  const stdoutLog = await open(logs.stdout, 'w');
  const stderrLog = await open(logs.stderr, 'w');
  const child = spawn(command, args, {
    cwd,
    stdio: ['ignore', stdoutLog.fd, stderrLog.fd],
    env: process.env,
  });
  return { child, stdoutLog, stderrLog };
}

async function waitForListeners({ httpPort, runtimePort, child, stderrPath }) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < LISTENER_TIMEOUT_MS) {
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(
        `router exited before listeners were ready (${child.signalCode ?? child.exitCode}); `
        + `stderr tail: ${await logTail(stderrPath)}`,
      );
    }
    if (await canConnect(httpPort) && await canConnect(runtimePort)) {
      return;
    }
    await delay(50);
  }
  throw new Error(`router listeners did not become ready within ${LISTENER_TIMEOUT_MS}ms`);
}

function canConnect(port) {
  return new Promise((resolve) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.setTimeout(100);
    socket.once('connect', () => {
      socket.destroy();
      resolve(true);
    });
    socket.once('timeout', () => {
      socket.destroy();
      resolve(false);
    });
    socket.once('error', () => resolve(false));
  });
}

async function stopChild(child, signal, { label }) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode };
  }
  return await new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error(`${label} did not exit within ${STOP_TIMEOUT_MS}ms`));
    }, STOP_TIMEOUT_MS);
    child.once('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once('exit', (code, exitSignal) => {
      clearTimeout(timer);
      resolve({ code, signal: exitSignal });
    });
    child.kill(signal);
  });
}

async function logTail(path, lines = 30) {
  try {
    const text = await readFile(path, 'utf8');
    return text.trim().split(/\r?\n/).slice(-lines).join('\n');
  } catch {
    return '<unavailable>';
  }
}
