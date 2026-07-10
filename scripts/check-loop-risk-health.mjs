#!/usr/bin/env node

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const arg = process.argv[index];
  if (!arg.startsWith('--')) {
    continue;
  }
  const [key, inlineValue] = arg.slice(2).split('=', 2);
  const value = inlineValue ?? process.argv[index + 1];
  if (inlineValue === undefined) {
    index += 1;
  }
  args.set(key, value);
}

const url =
  args.get('url') ?? 'http://127.0.0.1:4001/__router/health?detail=loop-risk';
const timeoutMs = Number(args.get('timeout-ms') ?? 5000);
const intervalMs = Number(args.get('interval-ms') ?? 250);
const deadline = Date.now() + timeoutMs;

let latest;
while (Date.now() <= deadline) {
  latest = await readLoopRiskHealth(url);
  if (loopRiskHealthIsZero(latest.loopRisk)) {
    console.log(JSON.stringify({
      ok: true,
      url,
      observedAt: latest.loopRisk.observedAt,
      router: latest.loopRisk.router,
      runtimes: latest.loopRisk.runtimes.length
    }, null, 2));
    process.exit(0);
  }
  await sleep(intervalMs);
}

console.error(JSON.stringify({
  ok: false,
  url,
  message: `loop-risk counters did not return to zero within ${timeoutMs}ms`,
  latest: latest?.loopRisk ?? null
}, null, 2));
process.exit(1);

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

function loopRiskHealthIsZero(loopRisk) {
  return (
    loopRisk.router.dispatcher.pendingUnary === 0 &&
    loopRisk.router.dispatcher.pendingStream === 0 &&
    loopRisk.router.dispatcher.pendingForward === 0 &&
    loopRisk.router.httpStream.backpressureWaiters === 0 &&
    loopRisk.router.websocketReceive.inFlight === 0 &&
    loopRisk.router.websocketReceive.queued === 0 &&
    loopRisk.runtimes.length > 0 &&
    loopRisk.runtimes.every((runtime) =>
      runtime.connected &&
      runtime.fresh &&
      runtime.counters.outboundRequestsPending === 0 &&
      runtime.counters.outboundStreamLeasesActive === 0 &&
      runtime.counters.streamRuntimeStreamsActive === 0 &&
      runtime.counters.flagBackedCancelWaitersActive === 0 &&
      runtime.counters.spawnedTasksActive === 0
    )
  );
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
