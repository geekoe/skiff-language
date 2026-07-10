import { randomBytes } from 'node:crypto';
import { open, lstat, link, mkdir, readFile, rm } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

export const serviceDbKeyringFormat = 'skiff-service-db-keyring-v1';
export const localServiceDbKeyId = 'local-v1';

const keyIdPattern = /^[A-Za-z0-9._-]{1,64}$/;
const lockRetryMs = 10;
const lockTimeoutMs = 30_000;
const incompleteLockGraceMs = 1_000;

export async function ensureLocalServiceDbKeyring(keyringPath, options = {}) {
  const directory = dirname(keyringPath);
  await mkdir(directory, { recursive: true, mode: 0o700 });

  const existing = await readProvisionedKeyringIfPresent(keyringPath);
  if (existing !== null) {
    return { action: 'kept', path: keyringPath };
  }

  const lockPath = `${keyringPath}.lock`;
  const lock = await acquireKeyringLock(lockPath, keyringPath, options);
  if (lock.winnerInstalled) {
    return { action: 'kept', path: keyringPath };
  }

  let temporaryPath;
  try {
    const winner = await readProvisionedKeyringIfPresent(keyringPath);
    if (winner !== null) {
      return { action: 'kept', path: keyringPath };
    }

    const keyring = {
      format: serviceDbKeyringFormat,
      activeKeyId: localServiceDbKeyId,
      keys: {
        [localServiceDbKeyId]: randomBytes(32).toString('base64'),
      },
    };
    const contents = `${JSON.stringify(keyring, null, 2)}\n`;
    temporaryPath = join(
      directory,
      `.${basename(keyringPath)}.${process.pid}.${randomBytes(12).toString('hex')}.tmp`,
    );
    const temporary = await open(temporaryPath, 'wx', 0o600);
    try {
      await temporary.chmod(0o600);
      await temporary.writeFile(contents, 'utf8');
      await temporary.sync();
    } finally {
      await temporary.close();
    }

    let action = 'created';
    try {
      await link(temporaryPath, keyringPath);
    } catch (error) {
      if (error?.code !== 'EEXIST') {
        throw error;
      }
      action = 'kept';
    }
    await rm(temporaryPath, { force: true });
    temporaryPath = undefined;
    if (action === 'created') {
      await syncDirectory(directory);
    }

    await readProvisionedKeyring(keyringPath);
    return { action, path: keyringPath };
  } finally {
    if (temporaryPath !== undefined) {
      await rm(temporaryPath, { force: true });
    }
    await releaseKeyringLock(lockPath, lock.owner);
  }
}

async function acquireKeyringLock(lockPath, keyringPath, options) {
  const timeoutMs = options.lockTimeoutMs ?? lockTimeoutMs;
  const retryMs = options.lockRetryMs ?? lockRetryMs;
  const startedAt = Date.now();
  const owner = `${process.pid}:${randomBytes(16).toString('hex')}`;
  while (true) {
    try {
      const lock = await open(lockPath, 'wx', 0o600);
      try {
        await lock.chmod(0o600);
        await lock.writeFile(`${owner}\n`, 'utf8');
        await lock.sync();
      } finally {
        await lock.close();
      }
      return { owner, winnerInstalled: false };
    } catch (error) {
      if (error?.code !== 'EEXIST') {
        throw error;
      }
    }

    const winner = await readProvisionedKeyringIfPresent(keyringPath);
    if (winner !== null) {
      return { owner: null, winnerInstalled: true };
    }
    await removeAbandonedLock(lockPath);
    if (Date.now() - startedAt >= timeoutMs) {
      throw new Error(`timed out waiting to provision local service DB keyring at ${keyringPath}`);
    }
    await delay(retryMs);
  }
}

async function removeAbandonedLock(lockPath) {
  let before;
  let owner;
  try {
    before = await lstat(lockPath);
    owner = (await readFile(lockPath, 'utf8')).trim();
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  const separator = owner.indexOf(':');
  const pid = Number(separator === -1 ? owner : owner.slice(0, separator));
  const incompleteOwner = !Number.isSafeInteger(pid) || pid <= 0;
  if ((incompleteOwner && Date.now() - before.mtimeMs < incompleteLockGraceMs)
      || (!incompleteOwner && processIsAlive(pid))) {
    return;
  }
  const after = await lstat(lockPath).catch((error) => {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  });
  if (after !== null && before.dev === after.dev && before.ino === after.ino) {
    await rm(lockPath, { force: true });
  }
}

async function releaseKeyringLock(lockPath, owner) {
  if (owner === null) {
    return;
  }
  try {
    if ((await readFile(lockPath, 'utf8')).trim() === owner) {
      await rm(lockPath, { force: true });
    }
  } catch (error) {
    if (error?.code !== 'ENOENT') {
      throw error;
    }
  }
}

async function readProvisionedKeyringIfPresent(path) {
  try {
    return await readProvisionedKeyring(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  }
}

async function readProvisionedKeyring(path) {
  const info = await lstat(path);
  if (!info.isFile()) {
    throw new Error(`local service DB keyring ${path} must be a regular file`);
  }
  if (process.platform !== 'win32' && (info.mode & 0o077) !== 0) {
    throw new Error(`local service DB keyring ${path} must not be accessible by group or others`);
  }
  const text = await readFile(path, 'utf8');
  let keyring;
  try {
    keyring = JSON.parse(text);
  } catch {
    throw new Error(`local service DB keyring ${path} is not valid JSON`);
  }
  validateProvisionedKeyring(keyring, path);
  return keyring;
}

function validateProvisionedKeyring(keyring, path) {
  if (!isPlainObject(keyring)
      || Object.keys(keyring).sort().join(',') !== 'activeKeyId,format,keys'
      || keyring.format !== serviceDbKeyringFormat
      || typeof keyring.activeKeyId !== 'string'
      || !isPlainObject(keyring.keys)) {
    throw new Error(`local service DB keyring ${path} is not a valid v1 keyring`);
  }
  const entries = Object.entries(keyring.keys);
  if (entries.length === 0 || !Object.hasOwn(keyring.keys, keyring.activeKeyId)) {
    throw new Error(`local service DB keyring ${path} is not a valid v1 keyring`);
  }
  for (const [keyId, material] of entries) {
    if (!keyIdPattern.test(keyId) || !canonicalBase64Key(material)) {
      throw new Error(`local service DB keyring ${path} is not a valid v1 keyring`);
    }
  }
}

function canonicalBase64Key(value) {
  if (typeof value !== 'string'
      || value.length !== 44
      || !/^[A-Za-z0-9+/]{43}=$/.test(value)) {
    return false;
  }
  const decoded = Buffer.from(value, 'base64');
  return decoded.length === 32 && decoded.toString('base64') === value;
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function processIsAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

async function syncDirectory(path) {
  if (process.platform === 'win32') {
    return;
  }
  const directory = await open(path, 'r');
  try {
    await directory.sync();
  } finally {
    await directory.close();
  }
}
