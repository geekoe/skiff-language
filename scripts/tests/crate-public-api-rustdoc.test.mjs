import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import test from 'node:test';

import {
  NIGHTLY_PROBE_TIMEOUT_MS,
  buildRustdocJson,
  createRustdocCommandRunner,
  probeCargoNightly,
} from '../lib/crate-public-api-rustdoc.mjs';

test('rustdoc command owner captures streams and settles only on close', async () => {
  const child = fakeChild();
  const spawned = [];
  const runner = fakeRunner(child, spawned);
  let settled = false;
  const completion = runner('cargo', ['rustdoc'], { cwd: '/repo' });
  completion.then(() => { settled = true; }, () => { settled = true; });

  child.stdout.emit('data', 'out-1');
  child.stdout.emit('data', 'out-2');
  child.stderr.emit('data', 'err');
  child.emit('exit', 0, null);
  await Promise.resolve();
  assert.equal(settled, false);
  child.emit('close', 0, null);

  assert.deepEqual(await completion, { stdout: 'out-1out-2', stderr: 'err' });
  assert.deepEqual(spawned, [{
    command: 'cargo',
    args: ['rustdoc'],
    options: {
      cwd: '/repo',
      env: undefined,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  }]);
  assert.equal(child.stdout.encoding, 'utf8');
  assert.equal(child.stderr.encoding, 'utf8');
});

test('async child error settles immediately and late close cannot replace it', async () => {
  const child = fakeChild();
  const runner = fakeRunner(child, []);
  const completion = runner('cargo', ['metadata']);
  const failure = new Error('spawn failed asynchronously');
  child.stdout.emit('data', 'partial-out');
  child.stderr.emit('data', 'partial-err');
  child.emit('error', failure);

  await assert.rejects(completion, (error) => {
    assert.equal(error, failure);
    assert.equal(error.command, 'cargo metadata');
    assert.equal(error.stdout, 'partial-out');
    assert.equal(error.stderr, 'partial-err');
    return true;
  });
  child.emit('close', 1, null);
});

test('timeout sends SIGKILL once and waits for close before rejection', async () => {
  const child = fakeChild();
  const timers = fakeTimers();
  const runner = fakeRunner(child, [], timers);
  let settled = false;
  const completion = runner('cargo', ['+nightly', '--version'], { timeoutMs: 10_000 });
  const observed = completion.then(
    () => { settled = true; return undefined; },
    (error) => { settled = true; return error; },
  );

  assert.equal(timers.delay, 10_000);
  timers.fire();
  await Promise.resolve();
  assert.equal(settled, false);
  assert.deepEqual(child.killSignals, ['SIGKILL']);
  child.emit('close', null, 'SIGKILL');

  const error = await observed;
  assert.match(error.message, /timed out after 10000ms/);
  assert.equal(error.timedOut, true);
  assert.equal(error.timeoutMs, 10_000);
  assert.equal(error.signal, 'SIGKILL');
  assert.deepEqual(timers.cleared, [timers.token]);
});

test('nonzero and signalled close retain diagnostics while synchronous spawn throw rejects', async () => {
  const child = fakeChild();
  const runner = fakeRunner(child, []);
  const completion = runner('cargo', ['rustdoc']);
  child.stdout.emit('data', 'stdout');
  child.stderr.emit('data', 'stderr');
  child.emit('close', 7, null);
  await assert.rejects(completion, (error) => {
    assert.equal(error.exitCode, 7);
    assert.equal(error.signal, null);
    assert.equal(error.stdout, 'stdout');
    assert.equal(error.stderr, 'stderr');
    assert.match(error.message, /exited with 7/);
    return true;
  });

  const syncFailure = new Error('synchronous spawn failure');
  const throwingRunner = createRustdocCommandRunner({
    clearTimer() {},
    createChild() { throw syncFailure; },
    setTimer() {},
  });
  await assert.rejects(throwingRunner('cargo', ['metadata']), (error) => error === syncFailure);
});

test('nightly probe owns the fixed timeout and rustdoc build preserves fallback order', async () => {
  const probeCalls = [];
  const probeError = new Error('nightly missing');
  const probe = await probeCargoNightly({
    env: { PATH: '/fake' },
    root: '/repo',
    async runCommand(command, args, options) {
      probeCalls.push({ command, args, options });
      throw probeError;
    },
  });
  assert.equal(NIGHTLY_PROBE_TIMEOUT_MS, 10_000);
  assert.deepEqual(probe, { available: false, error: probeError });
  assert.deepEqual(probeCalls, [{
    command: 'cargo',
    args: ['+nightly', '--version'],
    options: { cwd: '/repo', env: { PATH: '/fake' }, timeoutMs: 10_000 },
  }]);

  const buildCalls = [];
  const outcome = await buildRustdocJson({
    crateName: 'managed-crate',
    env: { SENTINEL: 'kept' },
    nightlyProbe: { available: true },
    root: '/repo',
    async runCommand(command, args, options) {
      buildCalls.push({ command, args, options });
      if (buildCalls.length === 1) {
        throw new Error('nightly rustdoc failed');
      }
      return { stdout: '', stderr: '' };
    },
  });
  assert.deepEqual(outcome, {
    fallbackLabel: 'RUSTC_BOOTSTRAP=1 cargo rustdoc',
  });
  assert.deepEqual(buildCalls.map(({ args }) => args.slice(0, 2)), [
    ['+nightly', 'rustdoc'],
    ['rustdoc', '-p'],
  ]);
  assert.equal(buildCalls[0].options.env, undefined);
  assert.deepEqual(buildCalls[1].options.env, {
    SENTINEL: 'kept',
    RUSTC_BOOTSTRAP: '1',
  });
});

function fakeRunner(child, spawned, timers = fakeTimers()) {
  return createRustdocCommandRunner({
    clearTimer: timers.clearTimer,
    createChild(command, args, options) {
      spawned.push({ command, args, options });
      return child;
    },
    setTimer: timers.setTimer,
  });
}

function fakeChild() {
  const child = new EventEmitter();
  child.stdout = fakeStream();
  child.stderr = fakeStream();
  child.killSignals = [];
  child.kill = (signal) => {
    child.killSignals.push(signal);
    return true;
  };
  return child;
}

function fakeStream() {
  const stream = new EventEmitter();
  stream.setEncoding = (encoding) => { stream.encoding = encoding; };
  return stream;
}

function fakeTimers() {
  const timers = {
    callback: undefined,
    cleared: [],
    delay: undefined,
    token: { timer: true },
  };
  timers.setTimer = (callback, delay) => {
    timers.callback = callback;
    timers.delay = delay;
    return timers.token;
  };
  timers.clearTimer = (token) => { timers.cleared.push(token); };
  timers.fire = () => timers.callback();
  return timers;
}
