// Per-side isolated two-replica orchestration for the actor parity
// differential (plan §7/§8/§9).
//
// Each side (TS and Rust Router) gets its own leased 4-port block (http /
// runtime / relay-1 / relay-2), its own devHome / artifact root / runtime
// homes, its own temporary mongod namespace and two real Runtime processes
// with deterministic replica ids. The Router process is resolved exclusively
// through the canonical RouterProcessSpec seam, never hardcoded.

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
  ACTOR_PARITY_ENVIRONMENT,
  ACTOR_PARITY_GENERATION,
  ACTOR_PARITY_REPLICA_ONE_ID,
  ACTOR_PARITY_REPLICA_TWO_ID,
  RUST_AUDIT_COLLECTION,
  RUST_DATABASE,
  RUST_STATE_COLLECTION,
  TS_AUDIT_COLLECTION,
  TS_DATABASE,
  TS_STATE_COLLECTION,
} from './actor_parity_constants.mjs';
import {
  countAuditEntries,
  createDifferentialMongosh,
  readActivationState,
  seedActivationState,
} from './mongo.mjs';
import { createActorParityRelay } from './actor_parity_relay.mjs';
import { projectActorParityFrameEvents } from './actor_parity_driver.mjs';

const LISTENER_TIMEOUT_MS = 30_000;
const HANDSHAKE_TIMEOUT_MS = 60_000;
const STOP_TIMEOUT_MS = 15_000;
const HANDSHAKE_SETTLE_MS = 300;

export async function createActorParitySide({
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
  environment = ACTOR_PARITY_ENVIRONMENT,
  generation = ACTOR_PARITY_GENERATION,
}) {
  const [httpPort, runtimePort, relayOnePort, relayTwoPort] = ports;
  const sideRoot = join(tempRoot, implementation);
  const devHome = join(sideRoot, 'dev-home');
  const runtimeOneHome = join(sideRoot, 'runtime-one-home');
  const runtimeTwoHome = join(sideRoot, 'runtime-two-home');
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
    schemaVersion: 'skiff-environment-activation-state-v2',
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
    artifactRoot,
    httpPort,
    runtimePort,
    relayOnePort,
    relayTwoPort,
    mongoPort,
    mongoUrl,
    database,
    stateCollection,
    auditCollection,
    mongosh,
    environment,
    generation,
    runtimeOneHome,
    runtimeTwoHome,
    state,
    ports,
    runtimeBin,
    routerSourceBinary,
    routerConfigPath: join(devHome, 'router.yml'),
    runtimeOneConfigPath: join(sideRoot, 'runtime-one.yml'),
    runtimeTwoConfigPath: join(sideRoot, 'runtime-two.yml'),
    routerLogs: {
      stdout: join(sideRoot, 'router.stdout.log'),
      stderr: join(sideRoot, 'router.stderr.log'),
    },
    runtimeOneLogs: {
      stdout: join(sideRoot, 'runtime-one.stdout.log'),
      stderr: join(sideRoot, 'runtime-one.stderr.log'),
    },
    runtimeTwoLogs: {
      stdout: join(sideRoot, 'runtime-two.stdout.log'),
      stderr: join(sideRoot, 'runtime-two.stderr.log'),
    },
  };
}

export async function startActorParitySide(side) {
  const {
    implementation,
    repoRoot,
    devHome,
    artifactRoot,
    httpPort,
    runtimePort,
    relayOnePort,
    relayTwoPort,
    mongoUrl,
    stateCollection,
    auditCollection,
    mongosh,
    environment,
    generation,
    runtimeOneHome,
    runtimeTwoHome,
    state,
    runtimeBin,
    routerConfigPath,
    runtimeOneConfigPath,
    runtimeTwoConfigPath,
    routerLogs,
    runtimeOneLogs,
    runtimeTwoLogs,
  } = side;

  await mkdir(join(devHome, 'bin'), { recursive: true });
  await mkdir(runtimeOneHome, { recursive: true });
  await mkdir(runtimeTwoHome, { recursive: true });

  const routerConfig = renderRouterConfig({
    profile: 'dev',
    host: '127.0.0.1',
    environment,
    artifactsPath: artifactRoot,
    releaseMode: true,
    requestTimeoutMs: 30_000,
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
  side.routerInvocation = routerProcessInvocation(spec);

  await seedActivationState({
    mongosh,
    mongoUrl,
    database: side.database,
    collection: stateCollection,
    environment,
    state,
  });
  await access(runtimeBin);
  await writeFile(join(runtimeOneHome, 'runtime-id'), `${ACTOR_PARITY_REPLICA_ONE_ID}\n`, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });
  await writeFile(join(runtimeTwoHome, 'runtime-id'), `${ACTOR_PARITY_REPLICA_TWO_ID}\n`, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });

  const runtimeOneConfig = renderRuntimeConfig({
    routerUrl: `ws://127.0.0.1:${relayOnePort}/runtime`,
    runtimeHome: runtimeOneHome,
    environment,
  });
  await writeFile(runtimeOneConfigPath, runtimeOneConfig, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });
  const runtimeTwoConfig = renderRuntimeConfig({
    routerUrl: `ws://127.0.0.1:${relayTwoPort}/runtime`,
    runtimeHome: runtimeTwoHome,
    environment,
  });
  await writeFile(runtimeTwoConfigPath, runtimeTwoConfig, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });

  side.routerProcess = await spawnWithLogs(
    side.routerInvocation.command,
    side.routerInvocation.args,
    { cwd: repoRoot, logs: routerLogs },
  );
  await waitForListeners({
    httpPort,
    runtimePort,
    child: side.routerProcess.child,
    stderrPath: routerLogs.stderr,
  });

  side.relayOne = await createActorParityRelay({
    port: relayOnePort,
    routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
  });
  side.relayTwo = await createActorParityRelay({
    port: relayTwoPort,
    routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
  });
  side.runtimeOneProcess = await spawnWithLogs(
    runtimeBin,
    [runtimeOneConfigPath],
    { cwd: repoRoot, logs: runtimeOneLogs },
  );
  side.runtimeTwoProcess = await spawnWithLogs(
    runtimeBin,
    [runtimeTwoConfigPath],
    { cwd: repoRoot, logs: runtimeTwoLogs },
  );

  await side.relayOne.waitForHandshake({ timeoutMs: HANDSHAKE_TIMEOUT_MS });
  await side.relayTwo.waitForHandshake({ timeoutMs: HANDSHAKE_TIMEOUT_MS });
  await delay(HANDSHAKE_SETTLE_MS);
}

export async function captureActorParitySide(side, driverResult) {
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
      steps: driverResult.steps,
    },
    frameEvents: projectFrameEvents(side),
    rawFrames: {
      replicaOne: side.relayOne.records.map((record) => ({ ...record })),
      replicaTwo: side.relayTwo.records.map((record) => ({ ...record })),
    },
    mongo: {
      state: stateDocument,
      auditCount,
    },
  };
}

function projectFrameEvents(side) {
  return projectActorParityFrameEvents([
    { replica: ACTOR_PARITY_REPLICA_ONE_ID, records: side.relayOne.records },
    { replica: ACTOR_PARITY_REPLICA_TWO_ID, records: side.relayTwo.records },
  ]);
}

export async function stopActorParitySide(side) {
  const errors = [];
  for (const [label, process] of [
    ['runtime one', side.runtimeOneProcess],
    ['runtime two', side.runtimeTwoProcess],
  ]) {
    if (process !== undefined) {
      try {
        const exit = await stopChild(process.child, 'SIGINT', {
          label: `${side.implementation} ${label}`,
        });
        if (label === 'runtime one') {
          side.runtimeOneExit = exit;
        } else {
          side.runtimeTwoExit = exit;
        }
      } catch (error) {
        errors.push(error);
      }
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
  for (const relay of [side.relayOne, side.relayTwo]) {
    if (relay !== undefined) {
      try {
        await relay.close();
      } catch (error) {
        errors.push(error);
      }
    }
  }
  const logPairs = [
    [side.routerProcess, side.routerLogs],
    [side.runtimeOneProcess, side.runtimeOneLogs],
    [side.runtimeTwoProcess, side.runtimeTwoLogs],
  ];
  for (const [process, logs] of logPairs) {
    if (process === undefined) {
      continue;
    }
    for (const handle of [process.stdoutLog, process.stderrLog]) {
      try {
        await handle.close();
      } catch (error) {
        errors.push(error);
      }
    }
  }
  try {
    await assertPortsClosed([
      side.httpPort,
      side.runtimePort,
      side.relayOnePort,
      side.relayTwoPort,
    ]);
    side.portsClosed = true;
  } catch (error) {
    side.portsClosed = false;
    errors.push(error);
  }
  if (errors.length > 0) {
    throw new AggregateError(
      errors,
      `${side.implementation} actor parity side stop failed`,
    );
  }
}

export async function readActorParitySideLogs(side) {
  return {
    routerStdout: await readFile(side.routerLogs.stdout, 'utf8'),
    routerStderr: await readFile(side.routerLogs.stderr, 'utf8'),
    runtimeOneStdout: await readFile(side.runtimeOneLogs.stdout, 'utf8'),
    runtimeOneStderr: await readFile(side.runtimeOneLogs.stderr, 'utf8'),
    runtimeTwoStdout: await readFile(side.runtimeTwoLogs.stdout, 'utf8'),
    runtimeTwoStderr: await readFile(side.runtimeTwoLogs.stderr, 'utf8'),
  };
}

export function actorParityTerminalObservation(side) {
  return {
    routerExitCode: side.routerExit?.code ?? null,
    runtimeOneExitCode: side.runtimeOneExit?.code ?? null,
    runtimeTwoExitCode: side.runtimeTwoExit?.code ?? null,
    portsClosed: side.portsClosed === true,
  };
}

export function actorParitySideContextObservation(side) {
  return {
    artifactsPath: side.artifactRoot,
    mongoUrl: side.mongoUrl,
    httpPort: side.httpPort,
    runtimePort: side.runtimePort,
    relayPort: side.relayOnePort,
    devHome: side.devHome,
    runtimeHome: side.runtimeOneHome,
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
