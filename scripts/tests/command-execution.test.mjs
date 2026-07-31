import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { access, mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { test } from 'node:test';
import { inspect } from 'node:util';

import {
  captureAttachedCommand,
  captureCheckedCommand,
  childCompletion,
  runAttachedCommand,
} from '../lib/command-execution.mjs';
import { captureOwnedCommand, runOwnedCommand } from '../lib/owned-command.mjs';

const commandModuleUrl = new URL('../lib/command-execution.mjs', import.meta.url).href;
const missingCommand = `skiff-missing-command-${process.pid}-${Date.now()}`;

test('command execution module exports only the fixed public API', async () => {
  const module = await import(commandModuleUrl);
  assert.deepEqual(Object.keys(module).sort(), [
    'captureAttachedCommand',
    'captureCheckedCommand',
    'childCompletion',
    'runAttachedCommand',
  ]);
});

test('attached command resolves zero and safely rejects nonzero and signal exits', async () => {
  await runAttachedCommand(process.execPath, ['--eval', 'process.exit(0)']);

  await assert.rejects(
    runAttachedCommand(process.execPath, ['--eval', 'process.exit(17)']),
    (error) => assertCommandError(error, { command: process.execPath, code: 17, signal: null }),
  );

  if (process.platform !== 'win32') {
    await assert.rejects(
      runAttachedCommand(process.execPath, [
        '--eval',
        "process.kill(process.pid, 'SIGTERM')",
      ]),
      (error) => assertCommandError(error, {
        command: process.execPath,
        code: null,
        signal: 'SIGTERM',
      }),
    );
  }
});

test('capture outcome preserves zero, nonzero, signal, and asynchronous spawn failure', async () => {
  const success = await captureAttachedCommand(process.execPath, [
    '--eval',
    "process.stdout.write('out'); process.stderr.write('err')",
  ]);
  assert.deepEqual(success, {
    code: 0,
    signal: null,
    stdout: 'out',
    stderr: 'err',
    error: null,
  });

  const nonzero = await captureAttachedCommand(process.execPath, [
    '--eval',
    "process.stdout.write('out'); process.stderr.write('err'); process.exit(19)",
  ]);
  assert.deepEqual(nonzero, {
    code: 19,
    signal: null,
    stdout: 'out',
    stderr: 'err',
    error: null,
  });

  if (process.platform !== 'win32') {
    const signalled = await captureAttachedCommand(process.execPath, [
      '--eval',
      "process.kill(process.pid, 'SIGTERM')",
    ]);
    assert.equal(signalled.code, null);
    assert.equal(signalled.signal, 'SIGTERM');
    assert.equal(signalled.error, null);
  }

  const missing = await captureAttachedCommand(missingCommand, ['secret-async-arg']);
  assert.equal(missing.error.name, 'SpawnFailure');
  assert.equal(missing.error.command, missingCommand);
  assert.equal(missing.error.code, 'ENOENT');
  assert.equal(Object.isFrozen(missing.error), true);
  assert.equal(Object.getPrototypeOf(missing.error), Object.prototype);
  assert.equal(missing.stdout, '');
  assert.equal(missing.stderr, '');
});

test('checked capture returns only streams and rejects outcomes with hidden read-only streams', async () => {
  assert.deepEqual(
    await captureCheckedCommand(process.execPath, [
      '--eval',
      "process.stdout.write('ok'); process.stderr.write('note')",
    ]),
    { stdout: 'ok', stderr: 'note' },
  );

  await assert.rejects(
    captureCheckedCommand(process.execPath, [
      '--eval',
      "process.stdout.write('failure-out'); process.stderr.write('failure-err'); process.exit(23)",
    ]),
    (error) => {
      assertCommandError(error, { command: process.execPath, code: 23, signal: null });
      assert.equal(error.stdout, 'failure-out');
      assert.equal(error.stderr, 'failure-err');
      for (const property of ['stdout', 'stderr']) {
        const descriptor = Object.getOwnPropertyDescriptor(error, property);
        assert.equal(descriptor.enumerable, false);
        assert.equal(descriptor.writable, false);
        assert.equal(descriptor.configurable, false);
      }
      assert.equal(Object.keys(error).includes('stdout'), false);
      assert.equal(JSON.stringify(error).includes('failure-out'), false);
      assert.equal(inspect(error).includes('failure-out'), false);
      return true;
    },
  );

  if (process.platform !== 'win32') {
    await assert.rejects(
      captureCheckedCommand(process.execPath, [
        '--eval',
        "process.kill(process.pid, 'SIGTERM')",
      ]),
      (error) => assertCommandError(error, {
        command: process.execPath,
        code: null,
        signal: 'SIGTERM',
      }),
    );
  }
});

test('synchronous spawn throws use the same safe failure semantics for all attached APIs', async () => {
  const secret = `nul-secret-${Date.now()}`;
  const invalidArg = `\0${secret}`;
  const outcome = await captureAttachedCommand(process.execPath, [invalidArg]);
  assert.equal(outcome.code, null);
  assert.equal(outcome.signal, null);
  assert.equal(outcome.stdout, '');
  assert.equal(outcome.stderr, '');
  assert.equal(outcome.error.name, 'SpawnFailure');
  assert.equal(Object.isFrozen(outcome.error), true);
  assertNoSecret(outcome, secret, { showHidden: true });

  for (const invoke of [
    () => runAttachedCommand(process.execPath, [invalidArg]),
    () => captureCheckedCommand(process.execPath, [invalidArg]),
  ]) {
    await assert.rejects(invoke(), (error) => {
      assert.equal(error.name, 'CommandExecutionError');
      assert.equal(error.command, process.execPath);
      assertNoSecret(error, secret);
      assert.equal(Object.hasOwn(error, 'cause'), false);
      return true;
    });
  }
});

test('cwd and env pass through without changing the default environment contract', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'skiff-command-cwd-'));
  try {
    const custom = await captureAttachedCommand(process.execPath, [
      '--eval',
      "process.stdout.write(`${process.cwd()}|${process.env.SKIFF_COMMAND_TEST}|${process.env.PATH ?? ''}`)",
    ], {
      cwd,
      env: { ...process.env, SKIFF_COMMAND_TEST: 'custom-env' },
    });
    assert.equal(custom.code, 0);
    assert.equal(custom.stdout, `${await realpath(cwd)}|custom-env|${process.env.PATH ?? ''}`);

    const inherited = await captureAttachedCommand(process.execPath, [
      '--eval',
      "process.stdout.write(process.env.PATH ?? '')",
    ]);
    assert.equal(inherited.stdout, process.env.PATH ?? '');
  } finally {
    await rm(cwd, { recursive: true, force: true });
  }
});

test('attached output is inherited while capture stays silent and receives stdin EOF', async () => {
  const attachedSource = [
    `import { runAttachedCommand } from ${JSON.stringify(commandModuleUrl)};`,
    `await runAttachedCommand(${JSON.stringify(process.execPath)}, ['--eval', ${JSON.stringify("process.stdout.write('attached-out'); process.stderr.write('attached-err')")}]);`,
  ].join('\n');
  const attached = await runProcess(process.execPath, [
    '--input-type=module',
    '--eval',
    attachedSource,
  ]);
  assert.equal(attached.code, 0);
  assert.equal(attached.stdout, 'attached-out');
  assert.equal(attached.stderr, 'attached-err');

  const captureSource = [
    `import { captureAttachedCommand } from ${JSON.stringify(commandModuleUrl)};`,
    `const result = await captureAttachedCommand(${JSON.stringify(process.execPath)}, ['--eval', ${JSON.stringify("process.stdin.resume(); process.stdin.on('end', () => { process.stdout.write('captured-out'); process.stderr.write('captured-err') })")}]);`,
    "process.stdout.write(JSON.stringify({ marker: 'wrapper-only', result }));",
  ].join('\n');
  const captured = await runProcess(process.execPath, [
    '--input-type=module',
    '--eval',
    captureSource,
  ]);
  assert.equal(captured.code, 0);
  assert.equal(captured.stderr, '');
  assert.deepEqual(JSON.parse(captured.stdout), {
    marker: 'wrapper-only',
    result: {
      code: 0,
      signal: null,
      stdout: 'captured-out',
      stderr: 'captured-err',
      error: null,
    },
  });
});

test('capture drains both large pipes and preserves split UTF-8', async () => {
  const bytes = 512 * 1024;
  const large = await captureAttachedCommand(process.execPath, [
    '--eval',
    `process.stdout.write('o'.repeat(${bytes})); process.stderr.write('e'.repeat(${bytes}))`,
  ]);
  assert.equal(large.code, 0);
  assert.equal(large.stdout, 'o'.repeat(bytes));
  assert.equal(large.stderr, 'e'.repeat(bytes));

  const split = await captureAttachedCommand(process.execPath, [
    '--eval',
    [
      "process.stdout.write(Buffer.from([0xe4]));",
      "setTimeout(() => process.stdout.write(Buffer.from([0xbd, 0xa0])), 20);",
    ].join(''),
  ]);
  assert.equal(split.code, 0);
  assert.equal(split.stdout, '你');
});

test('capture waits for close when a descendant keeps inherited pipes open', {
  skip: process.platform === 'win32',
}, async () => {
  const grandchild = "setTimeout(() => process.stdout.write('late'), 180)";
  const parent = [
    "const { spawn } = require('node:child_process');",
    `spawn(process.execPath, ['--eval', ${JSON.stringify(grandchild)}], { stdio: ['ignore', 1, 2] });`,
  ].join('');
  const startedAt = Date.now();
  const result = await captureAttachedCommand(process.execPath, ['--eval', parent]);
  assert.equal(result.code, 0);
  assert.equal(result.stdout, 'late');
  assert.ok(Date.now() - startedAt >= 150, 'capture must wait for close, not direct-child exit');
});

test('childCompletion is close-only, cached, and keeps a safe late error listener', async () => {
  const child = new EventEmitter();
  child.spawnfile = 'fake-command';
  const first = childCompletion(child);
  const second = childCompletion(child);
  assert.strictEqual(second, first);
  let completed = false;
  first.then(() => { completed = true; });

  child.emit('error', rawSpawnError('ENOENT', 'first-secret'));
  child.emit('error', rawSpawnError('EACCES', 'second-secret'));
  child.emit('exit', 0, null);
  await Promise.resolve();
  assert.equal(completed, false);
  child.emit('close', null, null);
  const outcome = await first;
  assert.deepEqual(outcome, {
    code: null,
    signal: null,
    error: {
      name: 'SpawnFailure',
      code: 'ENOENT',
      command: 'fake-command',
      message: 'failed to spawn fake-command: ENOENT',
    },
  });
  assert.equal(Object.isFrozen(outcome.error), true);
  assert.strictEqual(childCompletion(child), first);
  assert.ok(child.listenerCount('error') >= 1);

  child.emit('exit', 99, 'SIGKILL');
  child.emit('close', 99, 'SIGKILL');
  child.emit('error', rawSpawnError('EPERM', 'late-secret'));
  assert.strictEqual(await childCompletion(child), outcome);
  assertNoSecret(outcome, 'first-secret', { showHidden: true });
});

test('missing-command diagnostics never retain secret argv or raw spawn errors', async () => {
  const secret = `argv-secret-${Date.now()}`;
  const outcome = await captureAttachedCommand(missingCommand, [secret]);
  assertNoSecret(outcome, secret, { showHidden: true });
  assert.equal(Object.hasOwn(outcome.error, 'cause'), false);
  assert.equal(Object.hasOwn(outcome.error, 'spawnargs'), false);

  for (const invoke of [
    () => runAttachedCommand(missingCommand, [secret]),
    () => captureCheckedCommand(missingCommand, [secret]),
  ]) {
    await assert.rejects(invoke(), (error) => {
      assertNoSecret(error, secret);
      assert.equal(Object.hasOwn(error, 'cause'), false);
      assert.equal(Object.hasOwn(error, 'spawnargs'), false);
      return true;
    });
  }
});

test('owned commands safely normalize nonzero and synchronous spawn failures', async () => {
  await assert.rejects(
    runOwnedCommand(process.execPath, ['--eval', 'process.exit(37)'], { stdio: 'ignore' }),
    (error) => assertCommandError(error, {
      command: process.execPath,
      code: 37,
      signal: null,
    }),
  );

  const secret = `owned-nul-secret-${Date.now()}`;
  await assert.rejects(
    runOwnedCommand(process.execPath, [`\0${secret}`], { stdio: 'ignore' }),
    (error) => {
      assert.equal(error.name, 'CommandExecutionError');
      assert.equal(error.command, process.execPath);
      assertNoSecret(error, secret);
      assert.equal(Object.hasOwn(error, 'cause'), false);
      return true;
    },
  );

  const asyncSecret = `owned-enoent-secret-${Date.now()}`;
  await assert.rejects(
    runOwnedCommand(missingCommand, [asyncSecret], { stdio: 'ignore' }),
    (error) => {
      assertCommandError(error, {
        command: missingCommand,
        code: 'ENOENT',
        signal: null,
      });
      assertNoSecret(error, asyncSecret);
      return true;
    },
  );
});

test('captured owned command returns outcomes without throwing for zero, nonzero, and spawn failure', async () => {
  const success = await captureOwnedCommand(process.execPath, [
    '--eval',
    "process.stdout.write('out'); process.stderr.write('err')",
  ]);
  assert.deepEqual(success, {
    code: 0,
    signal: null,
    stdout: 'out',
    stderr: 'err',
    error: null,
  });

  const nonzero = await captureOwnedCommand(process.execPath, [
    '--eval',
    "process.stdout.write('out'); process.stderr.write('err'); process.exit(19)",
  ]);
  assert.deepEqual(nonzero, {
    code: 19,
    signal: null,
    stdout: 'out',
    stderr: 'err',
    error: null,
  });

  if (process.platform !== 'win32') {
    const signalled = await captureOwnedCommand(process.execPath, [
      '--eval',
      "process.kill(process.pid, 'SIGTERM')",
    ]);
    assert.equal(signalled.code, null);
    assert.equal(signalled.signal, 'SIGTERM');
    assert.equal(signalled.error, null);
  }

  const missing = await captureOwnedCommand(missingCommand, ['secret-async-arg']);
  assert.equal(missing.error.name, 'SpawnFailure');
  assert.equal(missing.error.command, missingCommand);
  assert.equal(missing.error.code, 'ENOENT');
  assert.equal(Object.isFrozen(missing.error), true);
  assert.equal(missing.stdout, '');
  assert.equal(missing.stderr, '');
});

test('captured owned command aborts through a signal and terminates the process group', {
  skip: process.platform === 'win32',
}, async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-owned-capture-abort-'));
  const grandchildMarker = join(fixture, 'grandchild-ran');
  try {
    const controller = new AbortController();
    const script = [
      `const { spawn } = require('node:child_process');`,
      `const fs = require('node:fs');`,
      `const marker = ${JSON.stringify(grandchildMarker)};`,
      `spawn(process.execPath, ['--eval', 'setTimeout(() => require("node:fs").writeFileSync(process.argv[1], "ran"), 1500)', marker], { stdio: 'ignore' });`,
      `setInterval(() => {}, 500);`,
    ].join('\n');
    const completion = captureOwnedCommand(process.execPath, ['--eval', script], {
      signal: controller.signal,
    });
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 200));
    controller.abort(new Error('test abort'));
    const outcome = await completion;
    assert.equal(outcome.code, null);
    assert.ok(
      outcome.signal === 'SIGTERM' || outcome.signal === 'SIGKILL',
      `unexpected termination signal ${outcome.signal}`,
    );
    assert.match(outcome.error.message, /test abort/);
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 400));
    await assert.rejects(access(grandchildMarker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('captured owned command reports an already-aborted signal without spawning', async () => {
  const controller = new AbortController();
  controller.abort(new Error('already aborted'));
  const outcome = await captureOwnedCommand(process.execPath, [
    '--eval',
    'process.exit(0)',
  ], {
    signal: controller.signal,
  });
  assert.equal(outcome.code, null);
  assert.equal(outcome.signal, null);
  assert.equal(outcome.stdout, '');
  assert.equal(outcome.stderr, '');
  assert.equal(outcome.error.name, 'Error');
  assert.match(outcome.error.message, /already aborted/);
});

test('checked errors hide child-written secrets by default but expose explicit streams', async () => {
  const secret = `child-stream-secret-${Date.now()}`;
  await assert.rejects(
    captureCheckedCommand(process.execPath, [
      '--eval',
      `process.stdout.write(${JSON.stringify(secret)}); process.stderr.write(${JSON.stringify(secret)}); process.exit(31)`,
    ]),
    (error) => {
      assertNoSecret(error, secret);
      assert.equal(error.stdout, secret);
      assert.equal(error.stderr, secret);
      return true;
    },
  );
});

function assertCommandError(error, { command, code, signal }) {
  assert.equal(error.name, 'CommandExecutionError');
  assert.equal(error.command, command);
  assert.equal(error.code, code);
  assert.equal(error.signal, signal);
  assert.equal(Object.hasOwn(error, 'cause'), false);
  return true;
}

function assertNoSecret(value, secret, { showHidden = false } = {}) {
  for (const rendered of [
    String(value),
    value?.stack ?? '',
    inspect(value),
    JSON.stringify(value),
    ...(showHidden ? [inspect(value, { showHidden: true, depth: null })] : []),
  ]) {
    assert.equal(rendered.includes(secret), false, rendered);
    assert.equal(rendered.includes('spawnargs'), false, rendered);
  }
}

function rawSpawnError(code, secret) {
  const error = new Error(`raw spawn failure ${secret}`);
  error.code = code;
  error.spawnargs = [secret];
  return error;
}

function runProcess(command, args, options = {}) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
}
