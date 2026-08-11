import { createHash, randomUUID } from 'node:crypto';
import { createReadStream as createPathReadStream } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import {
  chmod,
  copyFile,
  mkdir,
  rename,
  rm,
  stat as statPath,
} from 'node:fs/promises';

export async function binaryIdentity(path, options = {}) {
  const stat = options.stat ?? statPath;
  const createReadStream = options.createReadStream ?? createPathReadStream;
  const info = await stat(path, { bigint: true });
  if (!info.isFile()) {
    throw new Error(`${path} must be a file`);
  }
  const file = fileIdentity(info);
  if (fileIdentitiesEqual(options.knownIdentity?.file, file)) {
    return options.knownIdentity;
  }
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk);
  }
  return {
    algorithm: 'sha256',
    digest: hash.digest('hex'),
    size: Number(info.size),
    file,
  };
}

function fileIdentity(info) {
  return {
    device: String(info.dev),
    inode: String(info.ino),
    size: String(info.size),
    modifiedNs: String(info.mtimeNs),
    changedNs: String(info.ctimeNs),
  };
}

function fileIdentitiesEqual(left, right) {
  return left?.device === right?.device
    && left.inode === right.inode
    && left.size === right.size
    && left.modifiedNs === right.modifiedNs
    && left.changedNs === right.changedNs;
}

export function binaryIdentitiesEqual(left, right) {
  return left?.algorithm === 'sha256'
    && right?.algorithm === 'sha256'
    && left.digest === right.digest
    && left.size === right.size;
}

export async function installManagedBinary(source, destination, options = {}) {
  const sourceInfo = await statPath(source);
  if (!sourceInfo.isFile()) {
    throw new Error(`${source} must be a file`);
  }
  await mkdir(dirname(destination), { recursive: true });
  if (resolve(source) === resolve(destination)) {
    if (process.platform !== 'win32') {
      await chmod(destination, options.mode ?? 0o755);
    }
    return binaryIdentity(destination);
  }

  const temporary = join(
    dirname(destination),
    `.${basename(destination)}.install-${process.pid}-${randomUUID()}`,
  );
  try {
    await copyFile(source, temporary);
    if (process.platform !== 'win32') {
      await chmod(temporary, options.mode ?? 0o755);
    }
    const candidateIdentity = await binaryIdentity(temporary);
    try {
      const installedIdentity = await binaryIdentity(destination);
      if (binaryIdentitiesEqual(candidateIdentity, installedIdentity)) {
        if (
          process.platform === 'win32'
          || ((await statPath(destination)).mode & 0o7777) === (options.mode ?? 0o755)
        ) {
          return installedIdentity;
        }
      }
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
    }
    if (process.platform === 'win32') {
      await rm(destination, { force: true });
    }
    await rename(temporary, destination);
    return candidateIdentity;
  } finally {
    await rm(temporary, { force: true });
  }
}
