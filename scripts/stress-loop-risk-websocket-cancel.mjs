#!/usr/bin/env node

import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';

const execFileAsync = promisify(execFile);
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const args = parseArgs(process.argv.slice(2));

main().catch((error) => {
  console.error(JSON.stringify({
    ok: false,
    message: error instanceof Error ? error.message : String(error)
  }, null, 2));
  process.exit(1);
});

async function main() {
  if (hasFlag('help')) {
    printUsage();
    return;
  }

  const wsUrl = firstArg('ws-url') ?? process.env.SKIFF_LOOP_RISK_WS_URL;
  if (!wsUrl) {
    printUsage();
    throw new Error('--ws-url or SKIFF_LOOP_RISK_WS_URL is required');
  }

  const messages = readPositiveIntegerArg('messages', 1000);
  const concurrency = readPositiveIntegerArg('concurrency', 50);
  const healthUrl =
    firstArg('health-url') ?? 'http://127.0.0.1:4001/__router/health?detail=loop-risk';
  const healthTimeoutMs = readPositiveIntegerArg('health-timeout-ms', 5000);
  const headers = parseHeaders();
  const sessionPrefix = firstArg('session-prefix') ?? `loop-risk-stress-${Date.now()}`;
  const payloadTemplate =
    firstArg('payload') ?? '{"tag":"loop_risk_ws_cancel_stress","index":{index}}';
  const openTimeoutMs = readPositiveIntegerArg('open-timeout-ms', 5000);
  const closeTimeoutMs = readPositiveIntegerArg('close-timeout-ms', 5000);
  const closeDelayMs = readNonNegativeIntegerArg('close-delay-ms', 0);
  const skipHealth = hasFlag('skip-health');
  const skipCpu = hasFlag('skip-cpu');
  const skipLogCheck = hasFlag('skip-log-check');
  const logFiles = parseListArgs('runtime-log', 'log-file');
  const maxNewRuntimeRequestErrors = readNonNegativeIntegerArg(
    'max-new-runtime-request-errors',
    0
  );

  let touchedRuntimeIds = parseRuntimeIds();
  if (!skipHealth && touchedRuntimeIds.length === 0) {
    touchedRuntimeIds = await readConnectedRuntimeIds(healthUrl);
  }

  const logCountsBefore = skipLogCheck ? [] : await readRuntimeRequestErrorCounts(logFiles);
  const WebSocket = await loadWebSocket();
  const stormStartedAt = new Date().toISOString();
  const storm = await runWebSocketStorm({
    WebSocket,
    closeDelayMs,
    closeTimeoutMs,
    concurrency,
    headers,
    messages,
    openTimeoutMs,
    payloadTemplate,
    sessionPrefix,
    wsUrl
  });
  const stormStoppedAt = new Date().toISOString();

  if (!skipHealth) {
    await runHealthCheck({
      healthTimeoutMs,
      healthUrl,
      touchedRuntimeIds
    });
  }

  let cpuSummary = null;
  if (!skipCpu) {
    const runtimePids = await resolveRuntimePids();
    cpuSummary = await sampleRuntimeCpu(runtimePids);
  }

  const logSummary = skipLogCheck
    ? { checked: false, message: 'skipped by --skip-log-check' }
    : await checkRuntimeRequestErrorLogs(logFiles, logCountsBefore, maxNewRuntimeRequestErrors);

  console.log(JSON.stringify({
    ok: true,
    wsUrl,
    messages,
    concurrency,
    stormStartedAt,
    stormStoppedAt,
    storm,
    touchedRuntimeIds,
    health: skipHealth
      ? { checked: false, message: 'skipped by --skip-health' }
      : { checked: true, url: healthUrl },
    cpu: skipCpu
      ? { checked: false, message: 'skipped by --skip-cpu' }
      : cpuSummary,
    runtimeRequestErrorLogs: logSummary
  }, null, 2));
}

async function runWebSocketStorm(input) {
  let nextIndex = 0;
  let completed = 0;
  const failures = [];

  async function worker() {
    while (true) {
      const index = nextIndex;
      nextIndex += 1;
      if (index >= input.messages) {
        return;
      }
      try {
        await runWebSocketAttempt(input, index);
        completed += 1;
      } catch (error) {
        failures.push({
          index,
          message: error instanceof Error ? error.message : String(error)
        });
      }
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(input.concurrency, input.messages) }, () => worker())
  );

  if (failures.length > 0) {
    throw new Error(`websocket stress had ${failures.length} failures: ${JSON.stringify(failures.slice(0, 5))}`);
  }

  return {
    completed,
    failures: failures.length
  };
}

function runWebSocketAttempt(input, index) {
  return new Promise((resolve, reject) => {
    const headers = {
      ...input.headers
    };
    const sessionId = `${input.sessionPrefix}-${index}`;
    headers.cookie = headers.cookie
      ? `${headers.cookie}; sessionId=${sessionId}`
      : `sessionId=${sessionId}`;

    const ws = new input.WebSocket(input.wsUrl, { headers });
    let settled = false;
    let opened = false;
    const timeout = setTimeout(() => {
      settle(reject, new Error(`websocket attempt ${index} timed out`));
      ws.terminate();
    }, input.openTimeoutMs + input.closeTimeoutMs + input.closeDelayMs);

    const settle = (fn, value) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeout);
      ws.removeAllListeners();
      fn(value);
    };

    ws.once('open', () => {
      opened = true;
      const payload = input.payloadTemplate.replaceAll('{index}', String(index));
      ws.send(payload, (error) => {
        if (error) {
          settle(reject, error);
          return;
        }
        setTimeout(() => {
          if (ws.readyState === input.WebSocket.OPEN) {
            ws.close();
          }
        }, input.closeDelayMs);
      });
    });
    ws.once('close', () => {
      settle(resolve);
    });
    ws.once('error', (error) => {
      if (opened) {
        return;
      }
      settle(reject, error);
    });
  });
}

async function loadWebSocket() {
  const routerRequire = createRequire(path.join(scriptDir, '../router/package.json'));
  const resolved = routerRequire.resolve('ws');
  const imported = await import(pathToFileURL(resolved).href);
  return imported.default ?? imported.WebSocket ?? imported;
}

async function runHealthCheck(input) {
  const checkerPath = path.join(scriptDir, 'check-loop-risk-health.mjs');
  const checkerArgs = [
    checkerPath,
    '--url',
    input.healthUrl,
    '--timeout-ms',
    String(input.healthTimeoutMs)
  ];
  for (const runtimeId of input.touchedRuntimeIds) {
    checkerArgs.push('--runtime-id', runtimeId);
  }
  await execFileAsync(process.execPath, checkerArgs, {
    maxBuffer: 10 * 1024 * 1024
  });
}

async function readConnectedRuntimeIds(healthUrl) {
  const response = await fetch(healthUrl);
  if (!response.ok) {
    throw new Error(`health endpoint returned ${response.status}`);
  }
  const payload = await response.json();
  const runtimes = Array.isArray(payload.loopRisk?.runtimes)
    ? payload.loopRisk.runtimes
    : [];
  const runtimeIds = unique(
    runtimes
      .filter((runtime) => runtime.connected)
      .map((runtime) => runtime.runtimeId)
      .filter((runtimeId) => typeof runtimeId === 'string' && runtimeId.length > 0)
  );
  if (runtimeIds.length === 0) {
    throw new Error('no connected runtimes found; pass --runtime-id or --runtime-ids');
  }
  return runtimeIds;
}

async function resolveRuntimePids() {
  const explicitPids = parseListArgs('runtime-pid', 'runtime-pids')
    .map((value) => Number(value))
    .filter((value) => Number.isInteger(value) && value > 0);
  if (explicitPids.length > 0) {
    return unique(explicitPids);
  }

  const pattern = firstArg('runtime-pgrep') ?? 'skiff.*runtime|/runtime( |$)|target/.*/runtime( |$)';
  try {
    const { stdout } = await execFileAsync('pgrep', ['-f', pattern]);
    const pids = stdout
      .split(/\s+/)
      .map((value) => Number(value))
      .filter((value) => Number.isInteger(value) && value > 0 && value !== process.pid);
    if (pids.length > 0) {
      return unique(pids);
    }
  } catch {
    // pgrep exits non-zero when no process matches.
  }
  throw new Error('no runtime pid found; pass --runtime-pid or --runtime-pgrep');
}

async function sampleRuntimeCpu(runtimePids) {
  const seconds = readPositiveIntegerArg('cpu-seconds', 30);
  const intervalMs = readPositiveIntegerArg('cpu-interval-ms', 1000);
  const medianThreshold = readNumberArg('cpu-median-threshold', 5);
  const postGraceThreshold = readNumberArg('cpu-post-grace-threshold', 25);
  const graceSeconds = readNonNegativeIntegerArg('cpu-grace-seconds', 10);
  const samples = [];

  for (let index = 0; index < seconds; index += 1) {
    const totalCpu = await readTotalCpu(runtimePids);
    samples.push(totalCpu);
    console.log(JSON.stringify({
      event: 'runtime_cpu_sample',
      index,
      runtimePids,
      totalCpu
    }));
    if (index + 1 < seconds) {
      await sleep(intervalMs);
    }
  }

  const median = computeMedian(samples);
  const postGraceSamples = samples.slice(Math.min(graceSeconds, samples.length));
  const maxPostGrace = postGraceSamples.length > 0 ? Math.max(...postGraceSamples) : 0;
  if (median >= medianThreshold) {
    throw new Error(`runtime CPU median ${median.toFixed(2)}% is >= ${medianThreshold}%`);
  }
  if (maxPostGrace > postGraceThreshold) {
    throw new Error(
      `runtime CPU sample ${maxPostGrace.toFixed(2)}% exceeded ${postGraceThreshold}% after ${graceSeconds}s grace`
    );
  }

  return {
    checked: true,
    runtimePids,
    samples,
    median,
    maxPostGrace,
    medianThreshold,
    postGraceThreshold,
    graceSeconds
  };
}

async function readTotalCpu(runtimePids) {
  const values = await Promise.all(runtimePids.map((pid) => readCpuForPid(pid)));
  return values.reduce((sum, value) => sum + value, 0);
}

async function readCpuForPid(pid) {
  try {
    const { stdout } = await execFileAsync('ps', ['-o', '%cpu=', '-p', String(pid)]);
    const value = Number(stdout.trim());
    return Number.isFinite(value) ? value : 0;
  } catch {
    return 0;
  }
}

async function readRuntimeRequestErrorCounts(logFiles) {
  if (logFiles.length === 0) {
    return [];
  }
  return await Promise.all(
    logFiles.map(async (file) => ({
      file,
      count: countRuntimeRequestErrors(await fs.readFile(file, 'utf8'))
    }))
  );
}

async function checkRuntimeRequestErrorLogs(logFiles, beforeCounts, maxNewErrors) {
  if (logFiles.length === 0) {
    return {
      checked: false,
      message: 'no log file provided; pass --runtime-log to check runtime.request_error storm'
    };
  }
  const afterCounts = await readRuntimeRequestErrorCounts(logFiles);
  const beforeByFile = new Map(beforeCounts.map((entry) => [entry.file, entry.count]));
  const deltas = afterCounts.map((entry) => ({
    file: entry.file,
    before: beforeByFile.get(entry.file) ?? 0,
    after: entry.count,
    delta: entry.count - (beforeByFile.get(entry.file) ?? 0)
  }));
  const totalDelta = deltas.reduce((sum, entry) => sum + Math.max(0, entry.delta), 0);
  if (totalDelta > maxNewErrors) {
    throw new Error(
      `runtime.request_error log delta ${totalDelta} exceeded ${maxNewErrors}: ${JSON.stringify(deltas)}`
    );
  }
  return {
    checked: true,
    totalDelta,
    maxNewErrors,
    files: deltas
  };
}

function countRuntimeRequestErrors(text) {
  return (text.match(/runtime\.request_error/g) ?? []).length;
}

function parseArgs(argv) {
  const parsed = new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith('--')) {
      continue;
    }
    const [key, inlineValue] = arg.slice(2).split('=', 2);
    let value = inlineValue;
    if (value === undefined && argv[index + 1] && !argv[index + 1].startsWith('--')) {
      value = argv[index + 1];
      index += 1;
    }
    const values = parsed.get(key) ?? [];
    values.push(value ?? 'true');
    parsed.set(key, values);
  }
  return parsed;
}

function hasFlag(key) {
  return args.has(key);
}

function firstArg(key) {
  return args.get(key)?.[0];
}

function parseRuntimeIds() {
  return unique(parseListArgs('runtime-id', 'runtime-ids'));
}

function parseListArgs(...keys) {
  return keys
    .flatMap((key) => args.get(key) ?? [])
    .flatMap((value) => value.split(','))
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
}

function parseHeaders() {
  const headers = {};
  for (const entry of parseListArgs('header')) {
    const separator = entry.indexOf('=');
    if (separator <= 0) {
      throw new Error(`--header must be name=value, got ${entry}`);
    }
    headers[entry.slice(0, separator).trim().toLowerCase()] = entry
      .slice(separator + 1)
      .trim();
  }
  return headers;
}

function readPositiveIntegerArg(key, fallback) {
  const value = readNumberArg(key, fallback);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`--${key} must be a positive integer`);
  }
  return value;
}

function readNonNegativeIntegerArg(key, fallback) {
  const value = readNumberArg(key, fallback);
  if (!Number.isInteger(value) || value < 0) {
    throw new Error(`--${key} must be a non-negative integer`);
  }
  return value;
}

function readNumberArg(key, fallback) {
  const raw = firstArg(key);
  if (raw === undefined) {
    return fallback;
  }
  const value = Number(raw);
  if (!Number.isFinite(value)) {
    throw new Error(`--${key} must be a number`);
  }
  return value;
}

function computeMedian(values) {
  const sorted = [...values].sort((left, right) => left - right);
  const midpoint = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[midpoint - 1] + sorted[midpoint]) / 2
    : sorted[midpoint];
}

function unique(values) {
  return Array.from(new Set(values));
}

function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function printUsage() {
  console.log(`Usage:
  node scripts/stress-loop-risk-websocket-cancel.mjs --ws-url <url> [options]

Required:
  --ws-url <url>                 Stable-instance websocket URL, including service/version query if needed.

Stress:
  --messages <n>                 WebSocket send+close attempts. Default: 1000.
  --concurrency <n>              Parallel attempts. Default: 50.
  --payload <text>               Message payload. "{index}" is replaced per attempt.
  --header name=value            Extra WebSocket header. May be repeated.

Health:
  --health-url <url>             Router loop-risk health URL. Default: local stable control port.
  --runtime-id <id>              Touched runtime id. May be repeated or comma-separated.
  --runtime-ids <ids>            Comma-separated touched runtime ids.
  --health-timeout-ms <ms>       Health zero-window timeout. Default: 5000.

CPU:
  --runtime-pid <pid>            Runtime process id. May be repeated or comma-separated.
  --runtime-pgrep <pattern>      pgrep -f pattern when --runtime-pid is omitted.
  --cpu-seconds <n>              CPU sample count, one sample per second by default. Default: 30.
  --cpu-median-threshold <pct>   Median CPU threshold. Default: 5.
  --cpu-post-grace-threshold <pct> Max sample after grace. Default: 25.

Logs:
  --runtime-log <file>           Runtime log file for runtime.request_error delta checks.
  --max-new-runtime-request-errors <n> Default: 0.

Escape hatches:
  --skip-health
  --skip-cpu
  --skip-log-check`);
}
