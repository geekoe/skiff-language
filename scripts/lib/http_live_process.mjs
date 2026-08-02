// Process orchestration for the `router-live:http` managed harness.
//
// Post-cutover the Router is always the Rust binary: the harness starts the
// explicit `skiff-router` binary with the canonical
// `RouterProcessSpec`/`devHome/router.yml` and the same committed activation
// tuple. The real Runtime process is started per phase; a test-only WS relay
// records every frame so the harness can assert the handshake tuple, request
// frames and cancel frames without production seams.

import { spawn } from 'node:child_process';
import {
  access,
  copyFile,
  mkdir,
  open,
  readFile,
} from 'node:fs/promises';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { captureCheckedCommand } from './command-execution.mjs';
import {
  resolveRouterProcessSpec,
  routerBinaryName,
  routerProcessInvocation,
} from './dev-runtime-paths.mjs';
import { assertPortsClosed } from './local-port-lease.mjs';
import { renderRouterConfig, renderRuntimeConfig } from './runtime-stack-config.mjs';

const LISTENER_TIMEOUT_MS = 45_000;
const HANDSHAKE_TIMEOUT_MS = 90_000;
const STOP_TIMEOUT_MS = 20_000;

const HANDSHAKE_SEQUENCE = [
  'router.bootstrap',
  'runtime.capabilities',
  'assembly.activation',
  'runtime.registered',
  'runtime.health',
];

export function createHttpLiveRouterSpecs({ repoRoot, devHome }) {
  const rustSpec = resolveRouterProcessSpec({ devHome, repoRoot });
  return {
    rust: {
      implementation: 'rust',
      spec: rustSpec,
      invocation: routerProcessInvocation(rustSpec),
    },
  };
}

export function renderHttpLiveRouterConfig({
  environment,
  artifactsPath,
  httpPort,
  runtimePort,
  mongoUrl,
  requestTimeoutMs = 4000,
  httpMaxRequestBytes = 65536,
  httpMaxResponseBytes = 4096,
  runtimeMaxConcurrency = 16,
}) {
  return renderRouterConfig({
    profile: 'dev',
    host: '127.0.0.1',
    environment,
    artifactsPath,
    releaseMode: true,
    devReload: false,
    requestTimeoutMs,
    activationPrepareTimeoutMs: 120_000,
    httpPort,
    httpMaxRequestBytes,
    httpMaxResponseBytes,
    runtimePort,
    runtimePath: '/runtime',
    runtimeMaxConcurrency,
    serviceDbMongoUrl: mongoUrl,
  });
}

export async function writeHttpLiveRouterConfig(configPath, configText) {
  await mkdir(join(configPath, '..'), { recursive: true });
  await open(configPath, 'wx').then(async (handle) => {
    await handle.writeFile(configText, 'utf8');
    await handle.sync();
    await handle.close();
  });
}

export async function installHttpLiveRustBinary({
  sourceBinary,
  devHome,
  platform = process.platform,
}) {
  const installed = join(devHome, 'bin', routerBinaryName(platform));
  await mkdir(join(devHome, 'bin'), { recursive: true });
  await copyFile(sourceBinary, installed);
  await access(installed);
  return installed;
}

export function writeHttpLiveRuntimeConfig(runtimeConfigPath, { relayPort, runtimeHome, environment }) {
  return open(runtimeConfigPath, 'wx').then(async (handle) => {
    await handle.writeFile(
      renderRuntimeConfig({
        routerUrl: `ws://127.0.0.1:${relayPort}/runtime`,
        runtimeHome,
        environment,
      }),
      'utf8',
    );
    await handle.sync();
    await handle.close();
  });
}

export async function spawnLoggedProcess(command, args, {
  cwd,
  stdoutPath,
  stderrPath,
}) {
  const stdoutLog = await open(stdoutPath, 'w');
  const stderrLog = await open(stderrPath, 'w');
  // child-process-owner: http-live-process-spawn
  const child = spawn(command, args, {
    cwd,
    stdio: ['ignore', stdoutLog.fd, stderrLog.fd],
    env: process.env,
  });
  return { child, stdoutLog, stderrLog, command, args };
}

export async function waitForListeners({
  httpPort,
  runtimePort,
  child,
  stderrPath,
  timeoutMs = LISTENER_TIMEOUT_MS,
}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
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
  throw new Error(`router listeners did not become ready within ${timeoutMs}ms`);
}

export async function waitForHandshakeAfter(
  relay,
  fromIndex,
  { timeoutMs = HANDSHAKE_TIMEOUT_MS } = {},
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (hasHandshakeAfter(relay.records, fromIndex)) {
      return;
    }
    await delay(100);
  }
  throw new Error(
    `runtime handshake did not complete after relay index ${fromIndex} within ${timeoutMs}ms; `
    + `new frames: ${JSON.stringify(frameTypes(relay.records.slice(fromIndex)))}`,
  );
}

export function hasHandshakeAfter(records, fromIndex) {
  const types = records
    .slice(fromIndex)
    .filter((record) => typeof record.type === 'string')
    .map((record) => record.type);
  let index = 0;
  for (const type of types) {
    if (HANDSHAKE_SEQUENCE[index] === type) {
      index += 1;
      if (index === HANDSHAKE_SEQUENCE.length) {
        return true;
      }
    }
  }
  return false;
}

export function latestBootstrapTupleAfter(records, fromIndex) {
  for (let index = records.length - 1; index >= fromIndex; index -= 1) {
    const record = records[index];
    if (record?.type !== 'router.bootstrap') {
      continue;
    }
    const activation = record?.header?.activation;
    return {
      environment: activation?.environment ?? null,
      generation: activation?.generation ?? null,
      assemblyIdentity: activation?.assembly?.assemblyIdentity ?? null,
      configSnapshotId: activation?.configSnapshot?.snapshotId ?? null,
    };
  }
  return null;
}

export async function waitForCancelFrame(
  relay,
  fromIndex,
  requestId,
  { timeoutMs = 15_000 } = {},
) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const cancels = relay.records
      .slice(fromIndex)
      .filter((record) => record.type === 'request.cancel'
        && record.header?.requestId === requestId);
    if (cancels.length > 0) {
      return cancels.map((record) => record.header?.reason ?? null);
    }
    await delay(50);
  }
  return [];
}

export async function stopProcess(child, signal, { label, timeoutMs = STOP_TIMEOUT_MS }) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode };
  }
  return await new Promise((resolvePromise, reject) => {
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error(`${label} did not exit within ${timeoutMs}ms`));
    }, timeoutMs);
    child.once('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once('exit', (code, exitSignal) => {
      clearTimeout(timer);
      resolvePromise({ code, signal: exitSignal });
    });
    child.kill(signal);
  });
}

export async function closeLogs(processHandle) {
  const errors = [];
  for (const handle of [processHandle.stdoutLog, processHandle.stderrLog]) {
    try {
      await handle.close();
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'http live process log close failed');
  }
}

export async function readProcessLogs(processHandle) {
  return {
    stdout: await readFile(processHandle.stdoutLog.path, 'utf8'),
    stderr: await readFile(processHandle.stderrLog.path, 'utf8'),
  };
}

export function assertRouterExit(label, exit) {
  if (exit.code !== 0) {
    throw new Error(
      `${label} must exit 0 on shutdown, got ${JSON.stringify(exit)}`,
    );
  }
}

export async function assertRouterPortsClosed(ports) {
  await assertPortsClosed(ports);
}

export function canConnect(port) {
  return new Promise((resolvePromise) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.setTimeout(100);
    socket.once('connect', () => {
      socket.destroy();
      resolvePromise(true);
    });
    socket.once('timeout', () => {
      socket.destroy();
      resolvePromise(false);
    });
    socket.once('error', () => resolvePromise(false));
  });
}

export function frameTypes(records) {
  return records
    .filter((record) => typeof record.type === 'string')
    .map((record) => record.type);
}

export async function logTail(path, lines = 40) {
  try {
    const text = await readFile(path, 'utf8');
    return text.trim().split(/\r?\n/).slice(-lines).join('\n');
  } catch {
    return '<unavailable>';
  }
}
