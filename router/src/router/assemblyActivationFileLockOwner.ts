import { randomBytes } from 'node:crypto';
import { hostname } from 'node:os';

export type ActivationFileIdentity = Readonly<{
  device: string;
  inode: string;
}>;

export type ActivationFileLockOwner = Readonly<{
  schemaVersion: 'skiff-local-activation-lock-v1';
  nonce: string;
  pid: number;
  hostname: string;
  device: string;
  inode: string;
  createdAtMs: number;
}>;

export function activationFileIdentity(stats: {
  dev: bigint;
  ino: bigint;
}): ActivationFileIdentity {
  return { device: stats.dev.toString(), inode: stats.ino.toString() };
}

export function createActivationFileLockOwner(
  identity: ActivationFileIdentity
): ActivationFileLockOwner {
  return {
    schemaVersion: 'skiff-local-activation-lock-v1',
    nonce: randomBytes(16).toString('hex'),
    pid: process.pid,
    hostname: hostname(),
    device: identity.device,
    inode: identity.inode,
    createdAtMs: Date.now()
  };
}

export function decodeActivationFileLockOwner(
  bytes: Buffer,
  identity: ActivationFileIdentity
): ActivationFileLockOwner {
  const input: unknown = JSON.parse(bytes.toString('utf8'));
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('activation state lock owner must be an object');
  }
  const value = input as Record<string, unknown>;
  const fields = ['createdAtMs', 'device', 'hostname', 'inode', 'nonce', 'pid', 'schemaVersion'];
  if (Object.keys(value).sort().join(',') !== fields.join(',')) {
    throw new Error('activation state lock owner fields are incomplete');
  }
  if (
    value.schemaVersion !== 'skiff-local-activation-lock-v1' ||
    typeof value.nonce !== 'string' ||
    !/^[0-9a-f]{32}$/.test(value.nonce) ||
    !Number.isSafeInteger(value.pid) ||
    (value.pid as number) <= 0 ||
    typeof value.hostname !== 'string' ||
    value.hostname.length === 0 ||
    typeof value.device !== 'string' ||
    typeof value.inode !== 'string' ||
    !/^\d+$/.test(value.device) ||
    !/^\d+$/.test(value.inode) ||
    !Number.isSafeInteger(value.createdAtMs) ||
    (value.createdAtMs as number) <= 0
  ) {
    throw new Error('activation state lock owner is invalid');
  }
  const owner = value as ActivationFileLockOwner;
  if (owner.device !== identity.device || owner.inode !== identity.inode) {
    throw new Error('activation state lock owner identity does not match the file');
  }
  if (!bytes.equals(Buffer.from(encodeActivationFileLockOwner(owner)))) {
    throw new Error('activation state lock owner is not canonical JSON');
  }
  return owner;
}

export function encodeActivationFileLockOwner(owner: ActivationFileLockOwner): string {
  return JSON.stringify(owner);
}

export function sameActivationFileIdentity(
  left: ActivationFileIdentity,
  right: ActivationFileIdentity
): boolean {
  return left.device === right.device && left.inode === right.inode;
}

export function sameActivationFileLockOwner(
  left: ActivationFileLockOwner,
  right: ActivationFileLockOwner
): boolean {
  return encodeActivationFileLockOwner(left) === encodeActivationFileLockOwner(right);
}

export function activationOwnerPidIsAbsent(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return false;
  } catch (error) {
    return isNodeError(error, 'ESRCH');
  }
}

function isNodeError(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code;
}
