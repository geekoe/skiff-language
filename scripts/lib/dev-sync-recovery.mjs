import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { hostname } from 'node:os';
import { dirname, join } from 'node:path';

export async function startRecoveringPoll({
  runCycle,
  pollIntervalMs,
  onError,
  setIntervalFn = setInterval,
}) {
  let building = false;
  let pending = false;

  async function trigger() {
    if (building) {
      pending = true;
      return;
    }
    building = true;
    try {
      do {
        pending = false;
        await runCycle();
      } while (pending);
    } catch (error) {
      onError(error);
    } finally {
      building = false;
    }
  }

  await trigger();
  const timer = setIntervalFn(() => {
    void trigger();
  }, pollIntervalMs);
  return { timer, trigger };
}

export async function withOwnedDirectoryLock({
  lockDir,
  owner,
  action,
  timeoutMs,
  retryIntervalMs = 200,
  sleep,
  now = Date.now,
  isProcessAlive = processIsAlive,
  localHostname = hostname(),
  onReclaim = () => {},
}) {
  await mkdir(dirname(lockDir), { recursive: true });
  const startedAt = now();
  while (true) {
    try {
      await mkdir(lockDir);
      await writeFile(join(lockDir, 'owner.json'), JSON.stringify({
        ...owner,
        pid: process.pid,
        hostname: localHostname,
        lockId: randomUUID(),
        startedAt: new Date().toISOString(),
      }, null, 2));
      break;
    } catch (error) {
      if (error?.code !== 'EEXIST') {
        throw error;
      }
      if (await reclaimDeadOwnerLock(lockDir, {
        isProcessAlive,
        localHostname,
      })) {
        onReclaim(lockDir);
        continue;
      }
      if (now() - startedAt > timeoutMs) {
        throw new Error(`timed out waiting for ${lockDir}`);
      }
      await sleep(retryIntervalMs);
    }
  }

  try {
    return await action();
  } finally {
    await rm(lockDir, { recursive: true, force: true });
  }
}

export async function reclaimDeadOwnerLock(lockDir, {
  isProcessAlive = processIsAlive,
  localHostname = hostname(),
} = {}) {
  const ownerSnapshot = await readLockOwnerSnapshot(lockDir);
  if (ownerSnapshot === null || !ownerIsConfirmedLocal(ownerSnapshot.value, localHostname)) {
    return false;
  }
  if (await isProcessAlive(ownerSnapshot.value.pid)) {
    return false;
  }

  const reclaimDir = `${lockDir}.reclaim-${process.pid}-${randomUUID()}`;
  try {
    await rename(lockDir, reclaimDir);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return true;
    }
    throw error;
  }

  const movedSnapshot = await readLockOwnerSnapshot(reclaimDir);
  if (movedSnapshot?.raw !== ownerSnapshot.raw) {
    try {
      await rename(reclaimDir, lockDir);
    } catch (error) {
      if (error?.code !== 'EEXIST') {
        throw error;
      }
    }
    return false;
  }
  await rm(reclaimDir, { recursive: true, force: true });
  return true;
}

async function readLockOwnerSnapshot(lockDir) {
  try {
    const raw = await readFile(join(lockDir, 'owner.json'), 'utf8');
    const value = JSON.parse(raw);
    return { raw, value };
  } catch (error) {
    if (error?.code === 'ENOENT' || error instanceof SyntaxError) {
      return null;
    }
    throw error;
  }
}

function ownerIsConfirmedLocal(owner, localHostname) {
  if (!Number.isSafeInteger(owner?.pid) || owner.pid <= 0) {
    return false;
  }
  return owner.hostname === undefined || owner.hostname === localHostname;
}

async function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === 'ESRCH') {
      return false;
    }
    return true;
  }
}
