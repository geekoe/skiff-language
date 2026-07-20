export async function readLoopRiskHealth(endpoint, { fetchImpl = globalThis.fetch } = {}) {
  const response = await fetchImpl(endpoint);
  if (!response.ok) {
    throw new Error(`health endpoint returned ${response.status}`);
  }
  const payload = await response.json();
  if (!payload?.loopRisk) {
    throw new Error('health endpoint did not include loopRisk detail');
  }
  return payload;
}

export async function pollLoopRiskHealth(input, adapters = {}) {
  const now = adapters.now ?? Date.now;
  const sleep = adapters.sleep ?? defaultSleep;
  const deadline = now() + input.timeoutMs;
  let latest;
  let latestEvaluation;
  let latestError;
  while (now() <= deadline) {
    try {
      latest = await readLoopRiskHealth(input.url, { fetchImpl: adapters.fetch });
      latestEvaluation = evaluateLoopRiskHealth(latest.loopRisk, {
        touchedRuntimeIds: input.touchedRuntimeIds,
      });
      latestError = undefined;
      if (latestEvaluation.ok) {
        return {
          ok: true,
          observedAt: latest.loopRisk.observedAt,
          router: latest.loopRisk.router,
          runtimes: summarizeRuntimes(latest.loopRisk.runtimes, input.touchedRuntimeIds),
        };
      }
    } catch (error) {
      latestError = error instanceof Error ? error.message : String(error);
    }
    await sleep(input.intervalMs);
  }
  return {
    ok: false,
    message: `loop-risk counters did not satisfy zero-window within ${input.timeoutMs}ms`,
    reasons: latestEvaluation?.reasons ?? [],
    latestError,
    latest: latest?.loopRisk ?? null,
  };
}

export function evaluateLoopRiskHealth(loopRisk, { touchedRuntimeIds }) {
  const reasons = [];
  validateRouterCounters(loopRisk?.router, reasons);

  const runtimes = Array.isArray(loopRisk?.runtimes) ? loopRisk.runtimes : [];
  if (!Array.isArray(loopRisk?.runtimes)) {
    reasons.push('loopRisk.runtimes is missing or is not an array');
  }

  if (touchedRuntimeIds.length > 0) {
    validateTouchedRuntimeIds(runtimes, touchedRuntimeIds, reasons);
  } else {
    validateAllRuntimeSessions(runtimes, reasons);
  }
  return { ok: reasons.length === 0, reasons };
}

export function runtimeCountersAreZero(counters) {
  return (
    counters?.outboundRequestsPending === 0
    && counters?.outboundStreamLeasesActive === 0
    && counters?.streamRuntimeStreamsActive === 0
    && counters?.flagBackedCancelWaitersActive === 0
    && counters?.spawnedTasksActive === 0
  );
}

export function summarizeRuntimes(runtimes, touchedRuntimeIds) {
  const touched = new Set(touchedRuntimeIds);
  const selected = touched.size === 0
    ? runtimes
    : runtimes.filter((runtime) => touched.has(runtime.runtimeId));
  return selected.map((runtime) => ({
    runtimeId: runtime.runtimeId,
    connected: runtime.connected,
    fresh: runtime.fresh,
    counters: runtime.counters,
  }));
}

function validateRouterCounters(router, reasons) {
  expectCounter(router?.dispatcher?.pendingUnary, 'router.dispatcher.pendingUnary', reasons);
  expectCounter(router?.dispatcher?.pendingStream, 'router.dispatcher.pendingStream', reasons);
  expectCounter(
    router?.httpStream?.backpressureWaiters,
    'router.httpStream.backpressureWaiters',
    reasons,
  );
  expectCounter(
    router?.httpStream?.backpressureCancels,
    'router.httpStream.backpressureCancels',
    reasons,
  );
  expectCounter(router?.websocketReceive?.inFlight, 'router.websocketReceive.inFlight', reasons);
  expectCounter(router?.websocketReceive?.queued, 'router.websocketReceive.queued', reasons);
  expectCounter(
    router?.websocketReceive?.abortOnClose,
    'router.websocketReceive.abortOnClose',
    reasons,
  );
}

function expectCounter(value, name, reasons) {
  if (value !== 0) {
    reasons.push(`${name} is ${value === undefined ? 'missing' : String(value)}, expected 0`);
  }
}

function validateTouchedRuntimeIds(runtimes, touchedRuntimeIds, reasons) {
  for (const runtimeId of touchedRuntimeIds) {
    const sessions = runtimes.filter((runtime) => runtime.runtimeId === runtimeId);
    if (sessions.length === 0) {
      reasons.push(`touched runtime ${runtimeId} disappeared from loopRisk.runtimes`);
      continue;
    }
    if (!sessions.some((runtime) =>
      runtime.connected && runtime.fresh && runtimeCountersAreZero(runtime.counters))) {
      reasons.push(`touched runtime ${runtimeId} has no connected fresh zero session`);
    }
    for (const [index, runtime] of sessions.entries()) {
      const label = `touched runtime ${runtimeId} session ${index}`;
      validateRuntimeSession(runtime, label, reasons, {
        requireFresh: runtime.connected,
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
  if (!runtimes.some((runtime) =>
    runtime.connected && runtime.fresh && runtimeCountersAreZero(runtime.counters))) {
    reasons.push('loopRisk.runtimes has no connected fresh zero runtime session');
  }
  for (const [index, runtime] of runtimes.entries()) {
    validateRuntimeSession(runtime, `runtime session ${index}`, reasons, {
      requireFresh: runtime.connected,
    });
    if (!runtime.connected && !runtimeCountersAreZero(runtime.counters)) {
      reasons.push(`runtime session ${index} is disconnected with nonzero counters`);
    }
  }
}

function validateRuntimeSession(runtime, label, reasons, { requireFresh }) {
  if (!runtimeCountersAreZero(runtime.counters)) {
    reasons.push(`${label} counters are nonzero: ${JSON.stringify(runtime.counters)}`);
  }
  if (requireFresh && !runtime.fresh) {
    reasons.push(`${label} is connected but not fresh`);
  }
}

function defaultSleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
