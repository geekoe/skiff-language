import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  LOOP_RISK_CONFIG_PROFILES,
  parseLoopRiskConfig,
} from '../lib/loop-risk-config.mjs';
import { runLoopRiskStress } from '../lib/loop-risk-stress.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const stressCliPath = join(root, 'scripts', 'check-loop-risk-stress-live.mjs');

test('stress config schema enforces canonical ws, PID, and absolute log fields', () => {
  const valid = {
    healthUrl: 'http://router.test:4101/__router/health?detail=loop-risk',
    runtimeIds: ['runtime-a'],
    stress: {
      wsUrl: 'ws://router.test:4101/service/chat?version=a=b',
      runtimePids: [123],
      runtimeLogs: ['/tmp/runtime.log'],
    },
  };
  assert.deepEqual(
    parseLoopRiskConfig(valid, { profile: LOOP_RISK_CONFIG_PROFILES.STRESS }),
    valid,
  );
  for (const [value, expected] of [
    [{ ...valid, stress: undefined }, /requires stress/],
    [{ ...valid, stress: { ...valid.stress, wsUrl: undefined } }, /wsUrl/],
    [{ ...valid, stress: { ...valid.stress, wsUrl: 'http://router.test/path' } }, /must be ws/],
    [{ ...valid, stress: { ...valid.stress, runtimePids: [] } }, /runtimePids/],
    [{ ...valid, stress: { ...valid.stress, runtimePids: [1, -2] } }, /positive/],
    [{ ...valid, stress: { ...valid.stress, runtimePids: [1, 1] } }, /unique/],
    [{ ...valid, stress: { ...valid.stress, runtimeLogs: [] } }, /runtimeLogs/],
    [{ ...valid, stress: { ...valid.stress, runtimeLogs: ['relative.log'] } }, /absolute/],
    [{ ...valid, stress: { ...valid.stress, extra: true } }, /unknown field.*extra/],
  ]) {
    assert.throws(
      () => parseLoopRiskConfig(value, { profile: LOOP_RISK_CONFIG_PROFILES.STRESS }),
      expected,
    );
  }
});

test('stress core runs all canonical gates through injected hermetic adapters', async () => {
  const state = {};
  const result = await runLoopRiskStress(stressConfig(), fakeAdapters(state));
  assert.equal(result.ok, true);
  assert.equal(result.storm.completed, 1);
  assert.equal(result.health.checked, true);
  assert.equal(result.cpu.checked, true);
  assert.equal(result.runtimeRequestErrorLogs.checked, true);
  assert.equal(state.webSockets, 1);
  assert.equal(state.healthPolls, 1);
  assert.ok(state.pidChecks >= 2);
  assert.deepEqual(state.cpuReads, [123]);
  assert.equal(state.realSleeps ?? 0, 0);
});

test('stress core fails closed for dead/disappearing PID, ps, log, health, and log delta', async (t) => {
  await t.test('dead PID fails before workload', async () => {
    const state = { pidAlive: false };
    await assert.rejects(
      runLoopRiskStress(stressConfig(), fakeAdapters(state)),
      /not alive/,
    );
    assert.equal(state.webSockets ?? 0, 0);
  });

  await t.test('unreadable log fails before workload', async () => {
    const state = { readLogError: new Error('log unreadable') };
    await assert.rejects(
      runLoopRiskStress(stressConfig(), fakeAdapters(state)),
      /log unreadable/,
    );
    assert.equal(state.webSockets ?? 0, 0);
  });

  await t.test('PID disappearing before CPU sampling fails', async () => {
    const state = { pidAliveSequence: [true, false] };
    await assert.rejects(
      runLoopRiskStress(stressConfig(), fakeAdapters(state)),
      /not alive/,
    );
    assert.equal(state.webSockets, 1);
  });

  await t.test('ps failure is not converted to zero CPU', async () => {
    const state = { readCpuError: new Error('ps failed') };
    await assert.rejects(
      runLoopRiskStress(stressConfig(), fakeAdapters(state)),
      /ps failed/,
    );
  });

  await t.test('health timeout fails the run', async () => {
    const state = { healthResult: { ok: false, message: 'timeout', reasons: ['nonzero'] } };
    await assert.rejects(
      runLoopRiskStress(stressConfig(), fakeAdapters(state)),
      /health check failed.*timeout/,
    );
  });

  await t.test('new runtime.request_error fails the log gate', async () => {
    const state = { logTexts: ['', 'runtime.request_error'] };
    await assert.rejects(
      runLoopRiskStress(stressConfig(), fakeAdapters(state)),
      /log delta 1 exceeded 0/,
    );
  });
});

test('direct explicit skips remain visible while canonical-shaped runs are fully checked', async () => {
  const config = stressConfig({
    skipHealth: true,
    skipCpu: true,
    skipLogCheck: true,
    runtimeIds: [],
    runtimePids: [],
    runtimeLogs: [],
  });
  const result = await runLoopRiskStress(config, fakeAdapters({}));
  assert.equal(result.health.checked, false);
  assert.equal(result.cpu.checked, false);
  assert.equal(result.runtimeRequestErrorLogs.checked, false);
});

test('importing stress CLI and libraries has no execution or real adapter activity', async () => {
  for (const path of [
    stressCliPath,
    join(root, 'scripts', 'lib', 'loop-risk-stress.mjs'),
    join(root, 'scripts', 'lib', 'loop-risk-stress-node.mjs'),
  ]) {
    const result = await runProcess(process.execPath, [
      '--input-type=module',
      '--eval',
      `await import(${JSON.stringify(pathToFileURL(path).href)})`,
    ]);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(result.stdout, '');
    assert.equal(result.stderr, '');
  }
});

function stressConfig(overrides = {}) {
  const base = {
    canonical: true,
    wsUrl: 'ws://router.test:4101/runtime?token=a=b',
    healthUrl: 'http://router.test:4101/__router/health?detail=loop-risk',
    runtimeIds: ['runtime-a'],
    runtimePids: [123],
    runtimeLogs: ['/tmp/runtime.log'],
    skipHealth: false,
    skipCpu: false,
    skipLogCheck: false,
    messages: 1,
    concurrency: 1,
    healthTimeoutMs: 5,
    healthIntervalMs: 1,
    headers: {},
    sessionPrefix: 'session',
    payloadTemplate: '{"index":{index}}',
    openTimeoutMs: 5,
    closeTimeoutMs: 5,
    closeDelayMs: 0,
    maxNewRuntimeRequestErrors: 0,
    cpu: {
      seconds: 1,
      intervalMs: 1,
      medianThreshold: 100,
      postGraceThreshold: 100,
      graceSeconds: 0,
    },
  };
  return { ...base, ...overrides, cpu: { ...base.cpu, ...overrides.cpu } };
}

function fakeAdapters(state) {
  let timerId = 0;
  let pidIndex = 0;
  let logIndex = 0;
  return {
    createWebSocket() {
      state.webSockets = (state.webSockets ?? 0) + 1;
      return new FakeWebSocket();
    },
    isWebSocketOpen: (socket) => socket.readyState === 1,
    isPidAlive() {
      state.pidChecks = (state.pidChecks ?? 0) + 1;
      if (state.pidAliveSequence) {
        return state.pidAliveSequence[Math.min(pidIndex++, state.pidAliveSequence.length - 1)];
      }
      return state.pidAlive ?? true;
    },
    async readCpu(pid) {
      state.cpuReads ??= [];
      state.cpuReads.push(pid);
      if (state.readCpuError) {
        throw state.readCpuError;
      }
      return 1;
    },
    async readLog() {
      if (state.readLogError) {
        throw state.readLogError;
      }
      const values = state.logTexts ?? ['', ''];
      return values[Math.min(logIndex++, values.length - 1)];
    },
    now: () => state.now = (state.now ?? 0) + 1,
    async sleep() {
      state.realSleeps = (state.realSleeps ?? 0) + 1;
    },
    setTimer(callback) {
      timerId += 1;
      if (timerId % 2 === 0) {
        queueMicrotask(callback);
      }
      return timerId;
    },
    clearTimer() {},
    async pollHealth() {
      state.healthPolls = (state.healthPolls ?? 0) + 1;
      return state.healthResult ?? { ok: true };
    },
  };
}

class FakeWebSocket extends EventEmitter {
  constructor() {
    super();
    this.readyState = 0;
    queueMicrotask(() => {
      this.readyState = 1;
      this.emit('open');
    });
  }

  send(_payload, callback) {
    queueMicrotask(() => callback());
  }

  close() {
    this.readyState = 3;
    queueMicrotask(() => this.emit('close'));
  }

  terminate() {
    this.close();
  }
}

function runProcess(command, args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', (code, signal) => resolvePromise({ code, signal, stdout, stderr }));
  });
}
