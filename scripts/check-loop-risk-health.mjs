#!/usr/bin/env node

import assert from 'node:assert/strict';

const args = parseArgs(process.argv.slice(2));

if (hasFlag('help')) {
  printUsage();
  process.exit(0);
}

if (hasFlag('self-test')) {
  runSelfTest();
  process.exit(0);
}

const url =
  firstArg('url') ?? 'http://127.0.0.1:4001/__router/health?detail=loop-risk';
const timeoutMs = readPositiveIntegerArg('timeout-ms', 5000);
const intervalMs = readPositiveIntegerArg('interval-ms', 250);
const touchedRuntimeIds = parseRuntimeIds();
const deadline = Date.now() + timeoutMs;

let latest;
let latestEvaluation;
let latestError;
while (Date.now() <= deadline) {
  try {
    latest = await readLoopRiskHealth(url);
    latestEvaluation = evaluateLoopRiskHealth(latest.loopRisk, {
      touchedRuntimeIds
    });
    latestError = undefined;
    if (latestEvaluation.ok) {
      console.log(JSON.stringify({
        ok: true,
        url,
        observedAt: latest.loopRisk.observedAt,
        touchedRuntimeIds,
        router: latest.loopRisk.router,
        runtimes: summarizeRuntimes(latest.loopRisk.runtimes, touchedRuntimeIds)
      }, null, 2));
      process.exit(0);
    }
  } catch (error) {
    latestError = error instanceof Error ? error.message : String(error);
  }
  await sleep(intervalMs);
}

console.error(JSON.stringify({
  ok: false,
  url,
  touchedRuntimeIds,
  message: `loop-risk counters did not satisfy zero-window within ${timeoutMs}ms`,
  reasons: latestEvaluation?.reasons ?? [],
  latestError,
  latest: latest?.loopRisk ?? null
}, null, 2));
process.exit(1);

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

function readPositiveIntegerArg(key, fallback) {
  const raw = firstArg(key);
  if (raw === undefined) {
    return fallback;
  }
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`--${key} must be a positive integer`);
  }
  return value;
}

function parseRuntimeIds() {
  return unique(
    [
      ...(args.get('runtime-id') ?? []),
      ...(args.get('runtime-ids') ?? [])
    ]
      .flatMap((value) => value.split(','))
      .map((value) => value.trim())
      .filter((value) => value.length > 0)
  );
}

async function readLoopRiskHealth(endpoint) {
  const response = await fetch(endpoint);
  if (!response.ok) {
    throw new Error(`health endpoint returned ${response.status}`);
  }
  const payload = await response.json();
  if (!payload.loopRisk) {
    throw new Error('health endpoint did not include loopRisk detail');
  }
  return payload;
}

function evaluateLoopRiskHealth(loopRisk, options) {
  const reasons = [];
  validateRouterCounters(loopRisk?.router, reasons);

  const runtimes = Array.isArray(loopRisk?.runtimes) ? loopRisk.runtimes : [];
  if (!Array.isArray(loopRisk?.runtimes)) {
    reasons.push('loopRisk.runtimes is missing or is not an array');
  }

  if (options.touchedRuntimeIds.length > 0) {
    validateTouchedRuntimeIds(runtimes, options.touchedRuntimeIds, reasons);
  } else {
    validateAllRuntimeSessions(runtimes, reasons);
  }

  return {
    ok: reasons.length === 0,
    reasons
  };
}

function validateRouterCounters(router, reasons) {
  expectCounter(router?.dispatcher?.pendingUnary, 'router.dispatcher.pendingUnary', reasons);
  expectCounter(router?.dispatcher?.pendingStream, 'router.dispatcher.pendingStream', reasons);
  expectCounter(router?.dispatcher?.pendingForward, 'router.dispatcher.pendingForward', reasons);
  expectCounter(router?.httpStream?.backpressureWaiters, 'router.httpStream.backpressureWaiters', reasons);
  expectCounter(router?.httpStream?.backpressureCancels, 'router.httpStream.backpressureCancels', reasons);
  expectCounter(router?.websocketReceive?.inFlight, 'router.websocketReceive.inFlight', reasons);
  expectCounter(router?.websocketReceive?.queued, 'router.websocketReceive.queued', reasons);
  expectCounter(router?.websocketReceive?.abortOnClose, 'router.websocketReceive.abortOnClose', reasons);
}

function expectCounter(value, name, reasons) {
  if (value !== 0) {
    reasons.push(`${name} is ${formatCounterValue(value)}, expected 0`);
  }
}

function validateTouchedRuntimeIds(runtimes, touchedRuntimeIds, reasons) {
  for (const runtimeId of touchedRuntimeIds) {
    const sessions = runtimes.filter((runtime) => runtime.runtimeId === runtimeId);
    if (sessions.length === 0) {
      reasons.push(`touched runtime ${runtimeId} disappeared from loopRisk.runtimes`);
      continue;
    }

    const connectedFreshZero = sessions.filter(
      (runtime) => runtime.connected && runtime.fresh && runtimeCountersAreZero(runtime.counters)
    );
    if (connectedFreshZero.length === 0) {
      reasons.push(`touched runtime ${runtimeId} has no connected fresh zero session`);
    }

    for (const [index, runtime] of sessions.entries()) {
      const label = `touched runtime ${runtimeId} session ${index}`;
      validateRuntimeSession(runtime, label, reasons, {
        requireConnectedFreshZero: runtime.connected
      });
      if (!runtime.connected) {
        reasons.push(`${label} is disconnected; touched runtimes must remain connected`);
        if (!runtimeCountersAreZero(runtime.counters)) {
          reasons.push(`${label} is disconnected with nonzero counters`);
        }
      }
    }
  }
}

function validateAllRuntimeSessions(runtimes, reasons) {
  if (runtimes.length === 0) {
    reasons.push('loopRisk.runtimes is empty');
    return;
  }
  if (
    !runtimes.some(
      (runtime) => runtime.connected && runtime.fresh && runtimeCountersAreZero(runtime.counters)
    )
  ) {
    reasons.push('loopRisk.runtimes has no connected fresh zero runtime session');
  }
  for (const [index, runtime] of runtimes.entries()) {
    validateRuntimeSession(runtime, `runtime session ${index}`, reasons, {
      requireConnectedFreshZero: runtime.connected
    });
    if (!runtime.connected && !runtimeCountersAreZero(runtime.counters)) {
      reasons.push(`runtime session ${index} is disconnected with nonzero counters`);
    }
  }
}

function validateRuntimeSession(runtime, label, reasons, options) {
  if (!runtimeCountersAreZero(runtime.counters)) {
    reasons.push(`${label} counters are nonzero: ${JSON.stringify(runtime.counters)}`);
  }
  if (options.requireConnectedFreshZero && !runtime.fresh) {
    reasons.push(`${label} is connected but not fresh`);
  }
}

function runtimeCountersAreZero(counters) {
  return (
    counters?.outboundRequestsPending === 0 &&
    counters?.outboundStreamLeasesActive === 0 &&
    counters?.streamRuntimeStreamsActive === 0 &&
    counters?.flagBackedCancelWaitersActive === 0 &&
    counters?.spawnedTasksActive === 0
  );
}

function summarizeRuntimes(runtimes, touchedRuntimeIds) {
  const touched = new Set(touchedRuntimeIds);
  const selected = touched.size === 0
    ? runtimes
    : runtimes.filter((runtime) => touched.has(runtime.runtimeId));
  return selected.map((runtime) => ({
    runtimeId: runtime.runtimeId,
    connected: runtime.connected,
    fresh: runtime.fresh,
    counters: runtime.counters
  }));
}

function formatCounterValue(value) {
  return value === undefined ? 'missing' : String(value);
}

function unique(values) {
  return Array.from(new Set(values));
}

function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

function runSelfTest() {
  const zeroCounters = {
    outboundRequestsPending: 0,
    outboundStreamLeasesActive: 0,
    streamRuntimeStreamsActive: 0,
    flagBackedCancelWaitersActive: 0,
    spawnedTasksActive: 0
  };
  const nonzeroCounters = {
    ...zeroCounters,
    outboundRequestsPending: 1
  };
  const zeroRouter = {
    dispatcher: { pendingUnary: 0, pendingStream: 0, pendingForward: 0 },
    httpStream: { backpressureWaiters: 0, backpressureCancels: 0 },
    websocketReceive: { inFlight: 0, queued: 0, abortOnClose: 0 }
  };

  assert.equal(
    evaluateLoopRiskHealth({
      router: zeroRouter,
      runtimes: [
        { runtimeId: 'runtime-a', connected: true, fresh: true, counters: zeroCounters }
      ]
    }, { touchedRuntimeIds: ['runtime-a'] }).ok,
    true
  );
  assert.equal(
    evaluateLoopRiskHealth({
      router: {
        ...zeroRouter,
        websocketReceive: { inFlight: 0, queued: 0, abortOnClose: 1 }
      },
      runtimes: [
        { runtimeId: 'runtime-a', connected: true, fresh: true, counters: zeroCounters }
      ]
    }, { touchedRuntimeIds: ['runtime-a'] }).ok,
    false
  );
  assert.equal(
    evaluateLoopRiskHealth({
      router: zeroRouter,
      runtimes: [
        { runtimeId: 'runtime-a', connected: false, fresh: false, counters: nonzeroCounters }
      ]
    }, { touchedRuntimeIds: ['runtime-a'] }).ok,
    false
  );
  assert.equal(
    evaluateLoopRiskHealth({
      router: zeroRouter,
      runtimes: [
        { runtimeId: 'runtime-a', connected: false, fresh: false, counters: zeroCounters },
        { runtimeId: 'runtime-a', connected: true, fresh: true, counters: zeroCounters }
      ]
    }, { touchedRuntimeIds: ['runtime-a'] }).ok,
    false
  );
  assert.equal(
    evaluateLoopRiskHealth({
      router: zeroRouter,
      runtimes: [
        { runtimeId: 'runtime-b', connected: true, fresh: true, counters: zeroCounters }
      ]
    }, { touchedRuntimeIds: ['runtime-a'] }).ok,
    false
  );

  console.log(JSON.stringify({ ok: true, selfTest: 'check-loop-risk-health' }));
}

function printUsage() {
  console.log(`Usage:
  node scripts/check-loop-risk-health.mjs [options]

Options:
  --url <url>                 Router health URL. Defaults to local stable control port.
  --timeout-ms <ms>           Poll timeout. Default: 5000.
  --interval-ms <ms>          Poll interval. Default: 250.
  --runtime-id <id>           Touched runtime id. May be repeated or comma-separated.
  --runtime-ids <ids>         Comma-separated touched runtime ids.
  --self-test                 Run local evaluator self-checks without network.

Touched runtime ids must remain present with at least one connected fresh zero
session. Any disconnected touched runtime session fails the check because this
script has no pre-stress baseline mechanism.`);
}
