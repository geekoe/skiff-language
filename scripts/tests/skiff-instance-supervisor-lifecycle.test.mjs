import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { access, mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';

import {
  installManagedPidMetadata,
  removeManagedPidMetadata,
} from '../lib/managed-pid-metadata.mjs';
import { createSupervisedEntryLifecycle } from '../lib/supervised-entry-lifecycle.mjs';

const childSource = String.raw`
  let finished = false;
  const keepAlive = setInterval(() => {}, 1_000);
  const finish = () => {
    if (finished) return;
    finished = true;
    clearInterval(keepAlive);
    if (process.connected) process.disconnect();
    process.exitCode = 0;
  };
  process.on('message', (message) => {
    if (message === 'exit') finish();
  });
  process.on('SIGTERM', finish);
  process.stdout.write('supervisor lifecycle child ready\\n');
  process.stderr.write('supervisor lifecycle child ready\\n');
  process.send('ready');
`;

const matrix = [
  'exit-before-stop',
  'stop-before-exit',
  'same-turn-exit-first',
  'same-turn-stop-first',
];

test('supervised entry owns real log handles across exactly 20 IPC child exit/stop interleavings', {
  timeout: 30_000,
}, async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-supervisor-lifecycle-'));
  const rounds = new Map(matrix.map((scenario) => [scenario, 0]));
  try {
    for (const scenario of matrix) {
      await t.test(`${scenario}: five real FileHandle and IPC child rounds`, async () => {
        for (let index = 0; index < 5; index += 1) {
          await runRound({ root, scenario, index });
          rounds.set(scenario, rounds.get(scenario) + 1);
        }
      });
    }

    assert.deepEqual(Object.fromEntries(rounds), {
      'exit-before-stop': 5,
      'stop-before-exit': 5,
      'same-turn-exit-first': 5,
      'same-turn-stop-first': 5,
    });
    assert.equal([...rounds.values()].reduce((sum, count) => sum + count, 0), 20);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
  await assert.rejects(access(root), { code: 'ENOENT' });
});

test('false process-group stop rejects, preserves PID metadata, closes both handles, and blocks restart', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-supervisor-false-stop-'));
  const pidPath = join(root, 'component.pid');
  const stdoutFile = await open(join(root, 'stdout.log'), 'a');
  const stderrFile = await open(join(root, 'stderr.log'), 'a');
  const stdoutHandle = instrumentFileHandle(stdoutFile);
  const stderrHandle = instrumentFileHandle(stderrFile);
  let child;
  let closePromise;
  let pidRemovalCount = 0;
  try {
    await writeFile(pidPath, 'owned-and-still-live\n');
    child = spawn(process.execPath, ['--input-type=module', '--eval', childSource], {
      detached: true,
      stdio: ['ignore', stdoutFile.fd, stderrFile.fd, 'ipc'],
    });
    closePromise = once(child, 'close');
    const lifecycle = createSupervisedEntryLifecycle({
      component: 'false-stop',
      child,
      pgid: child.pid,
      stdoutHandle,
      stderrHandle,
      stopProcessGroup: async (pgid) => ({ pgid, stopped: false, forced: true }),
      isProcessGroupAlive: () => true,
      removePidMetadata: async () => {
        pidRemovalCount += 1;
        await rm(pidPath, { force: true });
      },
    });
    await once(child, 'message');

    let restartCount = 0;
    void lifecycle.completion.then(
      () => {
        restartCount += 1;
      },
      () => {},
    );
    const error = await rejectionOf(lifecycle.stop('false stop'));
    assert.ok(error instanceof AggregateError);
    assert.deepEqual(
      error.errors.map(({ step }) => step),
      ['process-group-stop', 'process-group-absence'],
    );
    assert.equal(pidRemovalCount, 0);
    assert.equal(await access(pidPath).then(() => true), true);
    assert.equal(stdoutHandle.closeCount, 1);
    assert.equal(stderrHandle.closeCount, 1);
    assert.equal(stdoutFile.fd, -1);
    assert.equal(stderrFile.fd, -1);
    assert.equal(processGroupAlive(child.pid), true);
    await delay(0);
    assert.equal(restartCount, 0);
  } finally {
    if (child !== undefined && processGroupAlive(child.pid)) {
      signalProcessGroup(child, child.pid, 'SIGKILL');
    }
    if (closePromise !== undefined) {
      await Promise.allSettled([closePromise]);
    }
    await Promise.allSettled([stdoutFile.close(), stderrFile.close()]);
    await rm(root, { recursive: true, force: true });
  }
});

test('unsupervised PID-write primary and one-sided close failure use the same lifecycle completion', async (t) => {
  await t.test('PID metadata write failure stops the child and closes both real handles', async () => {
    const fixture = await createChildFixture('pid-write');
    const pidPath = join(fixture.root, 'component.pid');
    let pidRemovalCount = 0;
    let pidOwner;
    let pidInstall;
    try {
      await writeFile(pidPath, 'foreign-pre-existing-owner\n');
      const lifecycle = createSupervisedEntryLifecycle({
        component: 'pid-write',
        child: fixture.child,
        pgid: fixture.child.pid,
        stdoutHandle: fixture.stdoutHandle,
        stderrHandle: fixture.stderrHandle,
        stopProcessGroup: (pgid) => stopRealProcessGroup(fixture.child, pgid),
        isProcessGroupAlive: processGroupAlive,
        removePidMetadata: async () => {
          pidRemovalCount += 1;
          await Promise.allSettled([pidInstall]);
          if (pidOwner !== undefined) {
            await removeManagedPidMetadata(pidOwner);
          }
        },
      });
      pidInstall = installManagedPidMetadata(pidPath, {
        schemaVersion: 1,
        component: 'pid-write',
        pid: fixture.child.pid,
        pgid: fixture.child.pid,
      }).then((owner) => {
        pidOwner = owner;
        return owner;
      });
      const pidWriteErrorPromise = rejectionOf(pidInstall);
      await once(fixture.child, 'message');
      const pidWriteError = await pidWriteErrorPromise;
      assert.equal(pidWriteError.code, 'EEXIST');
      lifecycle.recordPrimary(pidWriteError);
      assert.strictEqual(await rejectionOf(lifecycle.stop('PID write failure')), pidWriteError);
      await fixture.closePromise;
      assert.equal(pidRemovalCount, 1);
      assert.equal(fixture.stdoutHandle.closeCount, 1);
      assert.equal(fixture.stderrHandle.closeCount, 1);
      assert.equal(fixture.stdoutFile.fd, -1);
      assert.equal(fixture.stderrFile.fd, -1);
      assert.equal(processGroupAlive(fixture.child.pid), false);
      assert.equal(await readFile(pidPath, 'utf8'), 'foreign-pre-existing-owner\n');
    } finally {
      await fixture.cleanup();
    }
  });

  await t.test('delegated stdout close failure still closes stderr, stops the child, and removes PID', async () => {
    const closeMarker = new Error('injected unsupervised stdout close failure');
    const fixture = await createChildFixture('detach-close', closeMarker);
    const pidPath = join(fixture.root, 'component.pid');
    let pidRemovalCount = 0;
    try {
      await writeFile(pidPath, 'owned\n');
      const lifecycle = createSupervisedEntryLifecycle({
        component: 'detach-close',
        child: fixture.child,
        pgid: fixture.child.pid,
        stdoutHandle: fixture.stdoutHandle,
        stderrHandle: fixture.stderrHandle,
        stopProcessGroup: (pgid) => stopRealProcessGroup(fixture.child, pgid),
        isProcessGroupAlive: processGroupAlive,
        removePidMetadata: async () => {
          pidRemovalCount += 1;
          await rm(pidPath);
        },
      });
      await once(fixture.child, 'message');
      const error = await rejectionOf(lifecycle.detach());
      assert.ok(error instanceof AggregateError);
      assert.equal(error.errors.length, 1);
      assert.equal(error.errors[0].step, 'stdout-close');
      assert.strictEqual(error.errors[0].cause, closeMarker);
      await fixture.closePromise;
      assert.equal(pidRemovalCount, 1);
      assert.equal(fixture.stdoutHandle.closeCount, 1);
      assert.equal(fixture.stderrHandle.closeCount, 1);
      assert.equal(fixture.stdoutFile.fd, -1);
      assert.equal(fixture.stderrFile.fd, -1);
      assert.equal(processGroupAlive(fixture.child.pid), false);
      await assert.rejects(access(pidPath), { code: 'ENOENT' });
    } finally {
      await fixture.cleanup();
    }
  });
});

async function runRound({ root, scenario, index }) {
  const component = `${scenario}-${index}`;
  const stdoutPath = join(root, `${component}.stdout.log`);
  const stderrPath = join(root, `${component}.stderr.log`);
  const pidPath = join(root, `${component}.pid`);
  const stdoutFile = await open(stdoutPath, 'a');
  const stderrFile = await open(stderrPath, 'a');
  const combinedCleanupMarker = scenario === 'same-turn-stop-first' && index === 4
    ? new Error('injected delegated stdout close failure')
    : null;
  const cleanupOnlyMarker = scenario === 'same-turn-exit-first' && index === 4
    ? new Error('injected delegated stderr close failure')
    : null;
  const stdoutHandle = instrumentFileHandle(stdoutFile, combinedCleanupMarker);
  const stderrHandle = instrumentFileHandle(stderrFile, cleanupOnlyMarker);
  const primary = combinedCleanupMarker === null && scenario === 'exit-before-stop' && index === 4
    ? markedPrimary('primary-only marker')
    : combinedCleanupMarker === null
      ? null
      : markedPrimary('primary-plus-cleanup marker');
  let pidRemovalCount = 0;
  let processGroupStopCount = 0;
  let child;
  let lifecycle;
  let closePromise;

  try {
    await writeFile(pidPath, 'owned\n');
    child = spawn(process.execPath, ['--input-type=module', '--eval', childSource], {
      detached: true,
      stdio: ['ignore', stdoutFile.fd, stderrFile.fd, 'ipc'],
    });
    assert.ok(Number.isInteger(child.pid) && child.pid > 0);
    closePromise = once(child, 'close');

    lifecycle = createSupervisedEntryLifecycle({
      component,
      child,
      pgid: child.pid,
      stdoutHandle,
      stderrHandle,
      stopProcessGroup: async (pgid) => {
        processGroupStopCount += 1;
        return await stopRealProcessGroup(child, pgid);
      },
      isProcessGroupAlive: processGroupAlive,
      removePidMetadata: async () => {
        pidRemovalCount += 1;
        await rm(pidPath, { force: true });
      },
    });
    if (primary !== null) {
      lifecycle.recordPrimary(primary);
      lifecycle.recordPrimary(new Error('must not replace the first primary'));
    }

    assert.equal(await once(child, 'message').then(([message]) => message), 'ready');
    const completion = lifecycle.completion;
    let sendCompletion;
    switch (scenario) {
      case 'exit-before-stop':
        sendCompletion = requestChildExit(child);
        await lifecycle.exit;
        assert.strictEqual(lifecycle.stop('after exit'), completion);
        break;
      case 'stop-before-exit':
        assert.strictEqual(lifecycle.stop('before exit'), completion);
        assert.strictEqual(lifecycle.stop('repeated before exit'), completion);
        break;
      case 'same-turn-exit-first':
        sendCompletion = requestChildExit(child);
        assert.strictEqual(lifecycle.stop('same turn after exit request'), completion);
        break;
      case 'same-turn-stop-first':
        assert.strictEqual(lifecycle.stop('same turn before exit request'), completion);
        sendCompletion = requestChildExit(child);
        break;
      default:
        assert.fail(`unknown lifecycle scenario ${scenario}`);
    }
    assert.strictEqual(lifecycle.finish(), completion);
    assert.strictEqual(lifecycle.finish(), completion);
    assert.strictEqual(lifecycle.stop('repeated stop'), completion);
    if (sendCompletion !== undefined) {
      await sendCompletion;
    }

    if (combinedCleanupMarker !== null) {
      const error = await rejectionOf(completion);
      assert.ok(error instanceof AggregateError);
      assert.strictEqual(error.cause[0], primary);
      assert.strictEqual(error.errors[0], primary);
      assert.equal(error.cause[0].marker, 'primary-plus-cleanup marker');
      assert.equal(error.cause[1].component, component);
      assert.equal(error.cause[1].step, 'stdout-close');
      assert.equal(error.cause[1].stream, 'stdout');
      assert.strictEqual(error.cause[1].cause, combinedCleanupMarker);
      assert.deepEqual(error.errors, error.cause);
    } else if (cleanupOnlyMarker !== null) {
      const error = await rejectionOf(completion);
      assert.ok(error instanceof AggregateError);
      assert.equal(error.cause.length, 1);
      assert.equal(error.cause[0].component, component);
      assert.equal(error.cause[0].step, 'stderr-close');
      assert.equal(error.cause[0].stream, 'stderr');
      assert.strictEqual(error.cause[0].cause, cleanupOnlyMarker);
      assert.deepEqual(error.errors, error.cause);
    } else if (primary !== null) {
      assert.strictEqual(await rejectionOf(completion), primary);
      assert.equal(primary.marker, 'primary-only marker');
    } else {
      await completion;
    }
    await closePromise;

    assert.equal(stdoutHandle.closeCount, 1);
    assert.equal(stderrHandle.closeCount, 1);
    assert.equal(stdoutFile.fd, -1);
    assert.equal(stderrFile.fd, -1);
    assert.equal(pidRemovalCount, 1);
    assert.equal(child.connected, false);
    assert.equal(processGroupAlive(child.pid), false);
    assert.ok(child.exitCode !== null || child.signalCode !== null);
    assert.ok(processGroupStopCount <= 1);
    await assert.rejects(access(pidPath), { code: 'ENOENT' });
  } catch (error) {
    if (lifecycle !== undefined) {
      await Promise.allSettled([lifecycle.stop('test failure cleanup')]);
    } else {
      await Promise.allSettled([stdoutFile.close(), stderrFile.close()]);
    }
    if (child !== undefined && processGroupAlive(child.pid)) {
      signalProcessGroup(child, child.pid, 'SIGKILL');
    }
    if (closePromise !== undefined) {
      await Promise.allSettled([closePromise]);
    }
    throw error;
  }
}

async function createChildFixture(name, stdoutFailure = null) {
  const root = await mkdtemp(join(tmpdir(), `skiff-supervisor-${name}-`));
  const stdoutFile = await open(join(root, 'stdout.log'), 'a');
  const stderrFile = await open(join(root, 'stderr.log'), 'a');
  const stdoutHandle = instrumentFileHandle(stdoutFile, stdoutFailure);
  const stderrHandle = instrumentFileHandle(stderrFile);
  const child = spawn(process.execPath, ['--input-type=module', '--eval', childSource], {
    detached: true,
    stdio: ['ignore', stdoutFile.fd, stderrFile.fd, 'ipc'],
  });
  const closePromise = once(child, 'close');
  return {
    root,
    child,
    closePromise,
    stdoutFile,
    stderrFile,
    stdoutHandle,
    stderrHandle,
    async cleanup() {
      if (processGroupAlive(child.pid)) {
        signalProcessGroup(child, child.pid, 'SIGKILL');
      }
      await Promise.allSettled([closePromise, stdoutFile.close(), stderrFile.close()]);
      await rm(root, { recursive: true, force: true });
    },
  };
}

function instrumentFileHandle(fileHandle, injectedFailure = null) {
  let closeCount = 0;
  const closeRealFileHandle = fileHandle.close.bind(fileHandle);
  Object.defineProperties(fileHandle, {
    closeCount: {
      configurable: true,
      get() {
        return closeCount;
      },
    },
    close: {
      configurable: true,
      async value() {
        closeCount += 1;
        await closeRealFileHandle();
        if (injectedFailure !== null) {
          throw injectedFailure;
        }
      },
    },
  });
  return fileHandle;
}

function markedPrimary(marker) {
  const error = new Error(marker);
  error.marker = marker;
  return error;
}

function requestChildExit(child) {
  return new Promise((resolve, reject) => {
    try {
      child.send('exit', (error) => {
        if (error === null) {
          resolve();
        } else {
          reject(error);
        }
      });
    } catch (error) {
      reject(error);
    }
  });
}

function signalProcessGroup(child, pgid, signal) {
  try {
    if (process.platform === 'win32') {
      child.kill(signal);
    } else {
      process.kill(-pgid, signal);
    }
  } catch (error) {
    if (error?.code !== 'ESRCH') {
      throw error;
    }
  }
}

async function stopRealProcessGroup(child, pgid) {
  signalProcessGroup(child, pgid, 'SIGTERM');
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (!processGroupAlive(pgid)) {
      return { pgid, stopped: true, forced: false };
    }
    await delay(5);
  }
  signalProcessGroup(child, pgid, 'SIGKILL');
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (!processGroupAlive(pgid)) {
      return { pgid, stopped: true, forced: true };
    }
    await delay(5);
  }
  return { pgid, stopped: false, forced: true };
}

function processGroupAlive(pgid) {
  if (!Number.isInteger(pgid) || pgid <= 0) {
    return false;
  }
  try {
    process.kill(process.platform === 'win32' ? pgid : -pgid, 0);
    return true;
  } catch (error) {
    if (error?.code === 'EPERM') {
      return true;
    }
    if (error?.code === 'ESRCH') {
      return false;
    }
    throw error;
  }
}

async function rejectionOf(promise) {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  assert.fail('expected lifecycle completion to reject');
}
