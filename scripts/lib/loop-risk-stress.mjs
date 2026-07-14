export async function runLoopRiskStress(config, adapters) {
  assertStressInputs(config);
  assertAdapters(adapters);

  const logCountsBefore = config.skipLogCheck
    ? []
    : await readRuntimeRequestErrorCounts(config.runtimeLogs, adapters);
  if (!config.skipCpu) {
    await assertRuntimePidsAlive(config.runtimePids, adapters);
  }

  const stormStartedAt = new Date(adapters.now()).toISOString();
  const storm = await runWebSocketStorm(config, adapters);
  const stormStoppedAt = new Date(adapters.now()).toISOString();

  let health;
  if (config.skipHealth) {
    health = { checked: false, message: 'skipped by --skip-health' };
  } else {
    const result = await adapters.pollHealth({
      url: config.healthUrl,
      touchedRuntimeIds: config.runtimeIds,
      timeoutMs: config.healthTimeoutMs,
      intervalMs: config.healthIntervalMs,
    });
    if (!result.ok) {
      throw new Error(
        `loop-risk health check failed: ${JSON.stringify({
          message: result.message,
          reasons: result.reasons,
          latestError: result.latestError,
        })}`,
      );
    }
    health = { checked: true, url: config.healthUrl };
  }

  const cpu = config.skipCpu
    ? { checked: false, message: 'skipped by --skip-cpu' }
    : await sampleRuntimeCpu(config, adapters);
  const runtimeRequestErrorLogs = config.skipLogCheck
    ? { checked: false, message: 'skipped by --skip-log-check' }
    : await checkRuntimeRequestErrorLogs(config, logCountsBefore, adapters);

  return {
    ok: true,
    wsUrl: config.wsUrl,
    messages: config.messages,
    concurrency: config.concurrency,
    stormStartedAt,
    stormStoppedAt,
    storm,
    touchedRuntimeIds: config.runtimeIds,
    health,
    cpu,
    runtimeRequestErrorLogs,
  };
}

async function runWebSocketStorm(config, adapters) {
  let nextIndex = 0;
  let completed = 0;
  const failures = [];

  async function worker() {
    while (true) {
      const index = nextIndex;
      nextIndex += 1;
      if (index >= config.messages) {
        return;
      }
      try {
        await runWebSocketAttempt(config, adapters, index);
        completed += 1;
      } catch (error) {
        failures.push({
          index,
          message: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(config.concurrency, config.messages) },
      () => worker(),
    ),
  );
  if (failures.length > 0) {
    throw new Error(
      `websocket stress had ${failures.length} failures: ${JSON.stringify(failures.slice(0, 5))}`,
    );
  }
  return { completed, failures: failures.length };
}

function runWebSocketAttempt(config, adapters, index) {
  return new Promise((resolve, reject) => {
    const headers = { ...config.headers };
    const sessionId = `${config.sessionPrefix}-${index}`;
    headers.cookie = headers.cookie
      ? `${headers.cookie}; sessionId=${sessionId}`
      : `sessionId=${sessionId}`;

    let socket;
    try {
      socket = adapters.createWebSocket(config.wsUrl, { headers });
    } catch (error) {
      reject(error);
      return;
    }
    let settled = false;
    let opened = false;
    const timeout = adapters.setTimer(() => {
      settle(reject, new Error(`websocket attempt ${index} timed out`));
      socket.terminate();
    }, config.openTimeoutMs + config.closeTimeoutMs + config.closeDelayMs);

    const settle = (fn, value) => {
      if (settled) {
        return;
      }
      settled = true;
      adapters.clearTimer(timeout);
      socket.removeAllListeners();
      fn(value);
    };
    socket.once('open', () => {
      opened = true;
      const payload = config.payloadTemplate.replaceAll('{index}', String(index));
      socket.send(payload, (error) => {
        if (error) {
          settle(reject, error);
          return;
        }
        adapters.setTimer(() => {
          if (adapters.isWebSocketOpen(socket)) {
            socket.close();
          }
        }, config.closeDelayMs);
      });
    });
    socket.once('close', () => settle(resolve));
    socket.once('error', (error) => {
      if (!opened) {
        settle(reject, error);
      }
    });
  });
}

async function sampleRuntimeCpu(config, adapters) {
  const samples = [];
  for (let index = 0; index < config.cpu.seconds; index += 1) {
    await assertRuntimePidsAlive(config.runtimePids, adapters);
    const values = await Promise.all(
      config.runtimePids.map((pid) => adapters.readCpu(pid)),
    );
    const totalCpu = values.reduce((sum, value) => sum + value, 0);
    if (!Number.isFinite(totalCpu)) {
      throw new Error('runtime CPU sampler returned a non-finite value');
    }
    samples.push(totalCpu);
    adapters.onCpuSample?.({ index, runtimePids: config.runtimePids, totalCpu });
    if (index + 1 < config.cpu.seconds) {
      await adapters.sleep(config.cpu.intervalMs);
    }
  }

  const median = computeMedian(samples);
  const postGraceSamples = samples.slice(
    Math.min(config.cpu.graceSeconds, samples.length),
  );
  const maxPostGrace = postGraceSamples.length > 0 ? Math.max(...postGraceSamples) : 0;
  if (median >= config.cpu.medianThreshold) {
    throw new Error(
      `runtime CPU median ${median.toFixed(2)}% is >= ${config.cpu.medianThreshold}%`,
    );
  }
  if (maxPostGrace > config.cpu.postGraceThreshold) {
    throw new Error(
      `runtime CPU sample ${maxPostGrace.toFixed(2)}% exceeded ${config.cpu.postGraceThreshold}% after ${config.cpu.graceSeconds}s grace`,
    );
  }
  return {
    checked: true,
    runtimePids: config.runtimePids,
    samples,
    median,
    maxPostGrace,
    medianThreshold: config.cpu.medianThreshold,
    postGraceThreshold: config.cpu.postGraceThreshold,
    graceSeconds: config.cpu.graceSeconds,
  };
}

async function assertRuntimePidsAlive(runtimePids, adapters) {
  const dead = [];
  for (const pid of runtimePids) {
    if (!await adapters.isPidAlive(pid)) {
      dead.push(pid);
    }
  }
  if (dead.length > 0) {
    throw new Error(`runtime PID(s) are not alive: ${dead.join(', ')}`);
  }
}

async function readRuntimeRequestErrorCounts(logFiles, adapters) {
  return await Promise.all(logFiles.map(async (file) => ({
    file,
    count: countRuntimeRequestErrors(await adapters.readLog(file)),
  })));
}

async function checkRuntimeRequestErrorLogs(config, beforeCounts, adapters) {
  const afterCounts = await readRuntimeRequestErrorCounts(config.runtimeLogs, adapters);
  const beforeByFile = new Map(beforeCounts.map((entry) => [entry.file, entry.count]));
  const files = afterCounts.map((entry) => ({
    file: entry.file,
    before: beforeByFile.get(entry.file) ?? 0,
    after: entry.count,
    delta: entry.count - (beforeByFile.get(entry.file) ?? 0),
  }));
  const totalDelta = files.reduce((sum, entry) => sum + Math.max(0, entry.delta), 0);
  if (totalDelta > config.maxNewRuntimeRequestErrors) {
    throw new Error(
      `runtime.request_error log delta ${totalDelta} exceeded ${config.maxNewRuntimeRequestErrors}: ${JSON.stringify(files)}`,
    );
  }
  return {
    checked: true,
    totalDelta,
    maxNewErrors: config.maxNewRuntimeRequestErrors,
    files,
  };
}

function countRuntimeRequestErrors(text) {
  return (text.match(/runtime\.request_error/g) ?? []).length;
}

function computeMedian(values) {
  const sorted = [...values].sort((left, right) => left - right);
  const midpoint = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[midpoint - 1] + sorted[midpoint]) / 2
    : sorted[midpoint];
}

function assertStressInputs(config) {
  if (!config.wsUrl) {
    throw new Error('loop-risk stress requires wsUrl');
  }
  if (!config.skipHealth && (!config.healthUrl || config.runtimeIds.length === 0)) {
    throw new Error('enabled health check requires healthUrl and runtimeIds');
  }
  if (!config.skipCpu && config.runtimePids.length === 0) {
    throw new Error('enabled CPU check requires runtimePids');
  }
  if (!config.skipLogCheck && config.runtimeLogs.length === 0) {
    throw new Error('enabled log check requires runtimeLogs');
  }
}

function assertAdapters(adapters) {
  const required = [
    'createWebSocket',
    'isWebSocketOpen',
    'isPidAlive',
    'readCpu',
    'readLog',
    'now',
    'sleep',
    'setTimer',
    'clearTimer',
    'pollHealth',
  ];
  const missing = required.filter((name) => typeof adapters?.[name] !== 'function');
  if (missing.length > 0) {
    throw new Error(`loop-risk stress adapters missing: ${missing.join(', ')}`);
  }
}
