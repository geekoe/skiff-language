import { randomUUID } from 'node:crypto';
import { lstat, open, unlink } from 'node:fs/promises';

const ownerField = 'pidMetadataOwner';

export async function installManagedPidMetadata(path, metadata) {
  const nonce = randomUUID();
  let handle;
  try {
    handle = await open(path, 'wx', 0o600);
  } catch (cause) {
    if (cause?.code !== 'EEXIST') {
      throw cause;
    }
    const error = new Error(
      `[skiff-instance] refusing to replace pre-existing PID metadata at ${path}`,
      { cause },
    );
    error.code = 'EEXIST';
    error.path = path;
    throw error;
  }

  let identity;
  let writeError;
  try {
    identity = fileIdentity(await handle.stat({ bigint: true }));
    await handle.writeFile(`${JSON.stringify({
      ...metadata,
      [ownerField]: { nonce, ...identity },
    }, null, 2)}\n`);
    await handle.sync();
  } catch (error) {
    writeError = error;
  }

  const [closeResult] = await Promise.allSettled([handle.close()]);
  const errors = [
    ...(writeError === undefined ? [] : [writeError]),
    ...(closeResult.status === 'rejected' ? [closeResult.reason] : []),
  ];
  if (errors.length > 0) {
    if (identity !== undefined) {
      const cleanupResult = await Promise.allSettled([
        removePathWithIdentity(path, identity),
      ]);
      if (cleanupResult[0].status === 'rejected') {
        errors.push(cleanupResult[0].reason);
      }
    }
    throw errors.length === 1
      ? errors[0]
      : new AggregateError(
          errors,
          `[skiff-instance] PID metadata installation failed at ${path}`,
          { cause: errors },
        );
  }

  return Object.freeze({ path, nonce, ...identity });
}

export async function readManagedPidMetadataFile(path) {
  let handle;
  try {
    handle = await open(path, 'r');
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  }

  const results = await Promise.allSettled([
    handle.stat({ bigint: true }),
    handle.readFile({ encoding: 'utf8' }),
  ]);
  const [closeResult] = await Promise.allSettled([handle.close()]);
  const errors = [...results, closeResult]
    .flatMap((result) => result.status === 'rejected' ? [result.reason] : []);
  if (errors.length > 0) {
    throw errors.length === 1
      ? errors[0]
      : new AggregateError(
          errors,
          `[skiff-instance] PID metadata read failed at ${path}`,
          { cause: errors },
        );
  }
  return {
    identity: fileIdentity(results[0].value),
    text: results[1].value,
  };
}

export function managedPidMetadataOwner(path, metadata, identity) {
  const recordedOwner = metadata?.[ownerField];
  const nonce = recordedOwner?.nonce;
  if (typeof nonce !== 'string'
    || nonce.length === 0
    || identity === undefined
    || !sameIdentity(recordedOwner, identity)) {
    return null;
  }
  return Object.freeze({ path, nonce, ...identity });
}

export async function removeManagedPidMetadata(owner) {
  const snapshot = await readManagedPidMetadataFile(owner.path);
  if (snapshot === null) {
    return { removed: false, reason: 'missing' };
  }
  if (!sameIdentity(snapshot.identity, owner)) {
    return { removed: false, reason: 'replacement' };
  }

  let metadata;
  try {
    metadata = JSON.parse(snapshot.text);
  } catch {
    return { removed: false, reason: 'foreign' };
  }
  const recordedOwner = metadata?.[ownerField];
  if (recordedOwner?.nonce !== owner.nonce || !sameIdentity(recordedOwner, owner)) {
    return { removed: false, reason: 'foreign' };
  }

  const currentIdentity = await pathIdentity(owner.path);
  if (currentIdentity === null) {
    return { removed: false, reason: 'missing' };
  }
  if (!sameIdentity(currentIdentity, owner)) {
    return { removed: false, reason: 'replacement' };
  }
  try {
    await unlink(owner.path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return { removed: false, reason: 'missing' };
    }
    throw error;
  }
  return { removed: true, reason: 'owned' };
}

async function removePathWithIdentity(path, identity) {
  const currentIdentity = await pathIdentity(path);
  if (currentIdentity !== null && sameIdentity(currentIdentity, identity)) {
    await unlink(path);
  }
}

async function pathIdentity(path) {
  try {
    return fileIdentity(await lstat(path, { bigint: true }));
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  }
}

function fileIdentity(stats) {
  return { device: String(stats.dev), inode: String(stats.ino) };
}

function sameIdentity(left, right) {
  return left.device === right.device && left.inode === right.inode;
}
