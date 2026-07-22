import { lstat, open, readFile, unlink } from 'node:fs/promises';
import { hostname } from 'node:os';

import {
  throwCleanupErrors,
  throwPrimaryWithCleanup
} from './assemblyActivationCleanupErrors.js';
import {
  inspectActivationFileLock,
  reclaimStaleActivationFileLock,
  type ActivationFileLockObservation
} from './assemblyActivationFileLockRecovery.js';
import {
  activationFileIdentity,
  activationOwnerPidIsAbsent,
  createActivationFileLockOwner,
  decodeActivationFileLockOwner,
  encodeActivationFileLockOwner,
  sameActivationFileIdentity,
  sameActivationFileLockOwner,
  type ActivationFileIdentity,
  type ActivationFileLockOwner
} from './assemblyActivationFileLockOwner.js';

export type ActivationFileLockOptions = Readonly<{
  acquireTimeoutMs?: number;
  retryDelayMs?: number;
  staleGraceMs?: number;
}>;

type OwnedLock = Readonly<{
  handle: Awaited<ReturnType<typeof open>>;
  identity: ActivationFileIdentity;
  owner: ActivationFileLockOwner;
}>;

const DEFAULT_ACQUIRE_TIMEOUT_MS = 5_000;
const DEFAULT_RETRY_DELAY_MS = 5;
const DEFAULT_STALE_GRACE_MS = 1_000;

/** Local cooperative lock only; it does not provide an NFS or distributed lease. */
export async function withActivationFileLock<T>(
  path: string,
  operation: () => Promise<T>,
  options: ActivationFileLockOptions = {}
): Promise<T> {
  const owned = await acquire(path, options);
  let result: T | undefined;
  let completed = false;
  let failed = false;
  let primaryError: unknown;
  try {
    result = await operation();
    completed = true;
  } catch (error) {
    primaryError = error;
    failed = true;
  }
  const cleanupErrors = await release(path, owned, false);
  if (failed) {
    throwPrimaryWithCleanup(
      primaryError,
      cleanupErrors,
      'activation state mutation and lock cleanup both failed'
    );
  }
  throwCleanupErrors(cleanupErrors, 'activation state lock cleanup failed');
  if (!completed) {
    throw new Error('activation state mutation completed without a result');
  }
  return result as T;
}

async function acquire(
  path: string,
  options: ActivationFileLockOptions
): Promise<OwnedLock> {
  const timeout = positiveOption(options.acquireTimeoutMs, DEFAULT_ACQUIRE_TIMEOUT_MS);
  const retryDelay = positiveOption(options.retryDelayMs, DEFAULT_RETRY_DELAY_MS);
  const staleGrace = nonNegativeOption(options.staleGraceMs, DEFAULT_STALE_GRACE_MS);
  const deadline = Date.now() + timeout;
  while (true) {
    try {
      return await createOwnedLock(path);
    } catch (error) {
      if (!isNodeError(error, 'EEXIST')) {
        throw error;
      }
    }

    let observation: ActivationFileLockObservation | undefined;
    try {
      observation = await inspectActivationFileLock(path);
    } catch (error) {
      if (isNodeError(error, 'ENOENT')) {
        continue;
      }
      if (Date.now() >= deadline) {
        throw new Error('activation state lock is incomplete or invalid; refusing unsafe takeover', {
          cause: error
        });
      }
      await delay(retryDelay);
      continue;
    }

    if (observation.owner.hostname !== hostname()) {
      throw new Error('activation state lock belongs to a foreign host; refusing unsafe takeover');
    }
    const ownerIsAbsent = activationOwnerPidIsAbsent(observation.owner.pid);
    const staleAt = Math.max(
      observation.owner.createdAtMs,
      observation.modifiedAtMs
    ) + staleGrace;
    if (ownerIsAbsent && Date.now() >= staleAt) {
      if (await reclaimStaleActivationFileLock(path, observation, staleGrace)) {
        continue;
      }
    }
    if (Date.now() >= deadline) {
      const reason = ownerIsAbsent
        ? 'stale grace did not elapse'
        : 'owner PID is live or may have been reused';
      throw new Error(`activation state lock remained owned (${reason}); refusing unsafe takeover`);
    }
    await delay(Math.min(retryDelay, Math.max(1, deadline - Date.now())));
  }
}

async function createOwnedLock(path: string): Promise<OwnedLock> {
  const handle = await open(path, 'wx', 0o600);
  let owned: OwnedLock | undefined;
  try {
    const stats = await handle.stat({ bigint: true });
    const identity = activationFileIdentity(stats);
    const owner = createActivationFileLockOwner(identity);
    owned = { handle, identity, owner };
    await handle.writeFile(encodeActivationFileLockOwner(owner));
    await handle.sync();
    return owned;
  } catch (primaryError) {
    const cleanupErrors = owned === undefined
      ? await releaseUninitialized(path, handle)
      : await release(path, owned, true);
    throwPrimaryWithCleanup(
      primaryError,
      cleanupErrors,
      'activation state lock creation and cleanup both failed'
    );
  }
}

async function releaseUninitialized(
  path: string,
  handle: Awaited<ReturnType<typeof open>>
): Promise<unknown[]> {
  const errors: unknown[] = [];
  let identity: ActivationFileIdentity | undefined;
  try {
    identity = activationFileIdentity(await handle.stat({ bigint: true }));
  } catch (error) {
    errors.push(error);
  }
  try {
    await handle.close();
  } catch (error) {
    errors.push(error);
  }
  if (identity === undefined) {
    errors.push(new Error('activation state lock identity unavailable; refusing unsafe cleanup'));
    return errors;
  }
  try {
    const stats = await lstat(path, { bigint: true });
    if (
      stats.isSymbolicLink() ||
      !sameActivationFileIdentity(activationFileIdentity(stats), identity)
    ) {
      throw new Error('activation state lock identity changed; refusing to remove foreign lock');
    }
    await unlink(path);
  } catch (error) {
    errors.push(error);
  }
  return errors;
}

async function release(
  path: string,
  owned: OwnedLock,
  allowIncompleteOwner: boolean
): Promise<unknown[]> {
  const errors: unknown[] = [];
  try {
    await owned.handle.close();
  } catch (error) {
    errors.push(error);
  }
  try {
    const stats = await lstat(path, { bigint: true });
    if (
      stats.isSymbolicLink() ||
      !sameActivationFileIdentity(activationFileIdentity(stats), owned.identity)
    ) {
      throw new Error('activation state lock identity changed; refusing to remove foreign lock');
    }
    if (!allowIncompleteOwner) {
      const bytes = await readFile(path);
      const current = decodeActivationFileLockOwner(bytes, owned.identity);
      if (!sameActivationFileLockOwner(current, owned.owner)) {
        throw new Error('activation state lock owner changed; refusing to remove foreign lock');
      }
    }
    await unlink(path);
  } catch (error) {
    errors.push(error);
  }
  return errors;
}

function isNodeError(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}

function positiveOption(value: number | undefined, fallback: number): number {
  return value === undefined || !Number.isFinite(value) || value <= 0 ? fallback : value;
}

function nonNegativeOption(value: number | undefined, fallback: number): number {
  return value === undefined || !Number.isFinite(value) || value < 0 ? fallback : value;
}

async function delay(milliseconds: number): Promise<void> {
  await new Promise<void>((resolve) => setTimeout(resolve, milliseconds));
}
