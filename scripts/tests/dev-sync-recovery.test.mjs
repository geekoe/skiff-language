import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  reclaimDeadOwnerLock,
  startRecoveringPoll,
  withOwnedDirectoryLock,
} from '../lib/dev-sync-recovery.mjs';

test('watch schedules retries and recovers after the initial build fails', async () => {
  let attempts = 0;
  const errors = [];
  let scheduled;
  const watch = await startRecoveringPoll({
    pollIntervalMs: 10,
    async runCycle() {
      attempts += 1;
      if (attempts === 1) {
        throw new Error('broken input');
      }
    },
    onError(error) {
      errors.push(error.message);
    },
    setIntervalFn(callback, interval) {
      scheduled = { callback, interval };
      return { fake: true };
    },
  });

  assert.equal(attempts, 1);
  assert.deepEqual(errors, ['broken input']);
  assert.equal(scheduled.interval, 10);

  await watch.trigger();
  assert.equal(attempts, 2);
  assert.deepEqual(errors, ['broken input']);
});

test('dead local owner lock is reclaimed before acquiring the build lock', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-dead-lock-'));
  const lockDir = join(root, 'service.lock');
  try {
    await mkdir(lockDir);
    await writeFile(join(lockDir, 'owner.json'), JSON.stringify({
      pid: 424242,
      serviceId: 'example.com/service',
    }));

    let reclaimed = false;
    await withOwnedDirectoryLock({
      lockDir,
      owner: { serviceId: 'example.com/service' },
      timeoutMs: 10,
      sleep: async () => {},
      isProcessAlive: async (pid) => pid !== 424242,
      localHostname: 'test-host',
      onReclaim() {
        reclaimed = true;
      },
      async action() {
        const owner = JSON.parse(await readFile(join(lockDir, 'owner.json'), 'utf8'));
        assert.equal(owner.pid, process.pid);
        assert.equal(owner.hostname, 'test-host');
      },
    });
    assert.equal(reclaimed, true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('live owner lock is not reclaimed', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-live-lock-'));
  const lockDir = join(root, 'service.lock');
  try {
    const raw = JSON.stringify({
      pid: 12345,
      hostname: 'test-host',
      lockId: 'live-owner',
    });
    await mkdir(lockDir);
    await writeFile(join(lockDir, 'owner.json'), raw);

    const reclaimed = await reclaimDeadOwnerLock(lockDir, {
      isProcessAlive: async () => true,
      localHostname: 'test-host',
    });

    assert.equal(reclaimed, false);
    assert.equal(await readFile(join(lockDir, 'owner.json'), 'utf8'), raw);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
