import { link, lstat, readFile, unlink } from 'node:fs/promises';
import { hostname } from 'node:os';

import {
  throwCleanupErrors,
  throwPrimaryWithCleanup
} from './assemblyActivationCleanupErrors.js';
import {
  activationFileIdentity,
  activationOwnerPidIsAbsent,
  decodeActivationFileLockOwner,
  sameActivationFileIdentity,
  sameActivationFileLockOwner,
  type ActivationFileIdentity,
  type ActivationFileLockOwner
} from './assemblyActivationFileLockOwner.js';

export type ActivationFileLockObservation = Readonly<{
  owner: ActivationFileLockOwner;
  identity: ActivationFileIdentity;
  modifiedAtMs: number;
}>;

export async function inspectActivationFileLock(
  path: string
): Promise<ActivationFileLockObservation> {
  const stats = await lstat(path, { bigint: true });
  if (stats.isSymbolicLink() || !stats.isFile()) {
    throw new Error('activation state lock must be a regular file');
  }
  const identity = activationFileIdentity(stats);
  const owner = decodeActivationFileLockOwner(await readFile(path), identity);
  return {
    owner,
    identity,
    modifiedAtMs: Number(stats.mtimeNs / 1_000_000n)
  };
}

export async function reclaimStaleActivationFileLock(
  path: string,
  expected: ActivationFileLockObservation,
  staleGraceMs: number
): Promise<boolean> {
  const claimPath = `${path}.reclaim`;
  try {
    await link(path, claimPath);
  } catch (error) {
    if (isNodeError(error, 'ENOENT') || isNodeError(error, 'EEXIST')) {
      return false;
    }
    throw error;
  }

  let reclaimed = false;
  let failed = false;
  let primaryError: unknown;
  try {
    const claimStats = await lstat(claimPath, { bigint: true });
    if (
      claimStats.isSymbolicLink() ||
      !sameActivationFileIdentity(activationFileIdentity(claimStats), expected.identity)
    ) {
      throw new Error('activation state reclaim claim identity changed');
    }
    const current = await inspectActivationFileLock(path);
    if (isSameStaleOwner(current, expected, staleGraceMs)) {
      await unlink(path);
      reclaimed = true;
    }
  } catch (error) {
    if (!isNodeError(error, 'ENOENT')) {
      primaryError = error;
      failed = true;
    }
  }

  let cleanupError: unknown;
  let cleanupFailed = false;
  try {
    const claimStats = await lstat(claimPath, { bigint: true });
    if (
      claimStats.isSymbolicLink() ||
      !sameActivationFileIdentity(activationFileIdentity(claimStats), expected.identity)
    ) {
      throw new Error('activation state reclaim claim identity changed during cleanup');
    }
    await unlink(claimPath);
  } catch (error) {
    cleanupError = error;
    cleanupFailed = true;
  }
  const cleanupErrors = cleanupFailed ? [cleanupError] : [];
  if (failed) {
    throwPrimaryWithCleanup(
      primaryError,
      cleanupErrors,
      'activation state stale reclaim and cleanup both failed'
    );
  }
  throwCleanupErrors(cleanupErrors, 'activation state stale reclaim cleanup failed');
  return reclaimed;
}

function isSameStaleOwner(
  current: ActivationFileLockObservation,
  expected: ActivationFileLockObservation,
  staleGraceMs: number
): boolean {
  return (
    sameActivationFileIdentity(current.identity, expected.identity) &&
    sameActivationFileLockOwner(current.owner, expected.owner) &&
    current.owner.hostname === hostname() &&
    activationOwnerPidIsAbsent(current.owner.pid) &&
    Date.now() >= Math.max(current.owner.createdAtMs, current.modifiedAtMs) + staleGraceMs
  );
}

function isNodeError(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}
