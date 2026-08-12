import { constants } from 'node:fs';
import {
  lstat,
  mkdir,
  open,
  readdir,
  realpath,
} from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

export const PHASE0_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-0-directory-identity-v1';
export const PHASE0_DIRECTORY_IDENTITY_FILE = 'directory-identities.json';

export async function createPhase0EvidenceRoot(outputDir) {
  await mkdir(outputDir, { mode: 0o700 });
  const root = await captureDirectoryIdentity(outputDir, outputDir);
  const evidenceRoot = new Phase0EvidenceRoot(outputDir, root, new Map());
  await evidenceRoot.createDirectory('commands');
  await evidenceRoot.writeExclusive(
    PHASE0_DIRECTORY_IDENTITY_FILE,
    `${JSON.stringify(evidenceRoot.identities(), null, 2)}\n`,
  );
  await evidenceRoot.assertAll();
  return evidenceRoot;
}

export async function openPhase0EvidenceRoot(outputDir, expectedIdentities) {
  const identities = validateIdentityRecord(expectedIdentities, outputDir);
  const evidenceRoot = new Phase0EvidenceRoot(
    outputDir,
    identities.root,
    new Map(Object.entries(identities.directories)),
  );
  await evidenceRoot.assertAll();
  const stored = JSON.parse(await evidenceRoot.readFile(PHASE0_DIRECTORY_IDENTITY_FILE, 'utf8'));
  if (JSON.stringify(stored) !== JSON.stringify(identities)) {
    throw new Error('evidence directory identity record drifted');
  }
  await evidenceRoot.assertAll();
  return evidenceRoot;
}

export class Phase0EvidenceRoot {
  constructor(outputDir, rootIdentity, directories) {
    this.outputDir = outputDir;
    this.rootIdentity = Object.freeze({ ...rootIdentity });
    this.directories = directories;
  }

  identities() {
    return {
      schemaVersion: PHASE0_DIRECTORY_IDENTITY_SCHEMA,
      root: { ...this.rootIdentity },
      directories: Object.fromEntries(
        [...this.directories.entries()].map(([path, identity]) => [path, { ...identity }]),
      ),
    };
  }

  async createDirectory(relativePath) {
    const normalized = normalizeRelative(relativePath);
    if (normalized.includes('/')) throw new Error('evidence subdirectories must be direct children');
    await this.assertAll();
    const absolute = this.resolve(normalized);
    await mkdir(absolute, { mode: 0o700 });
    const identity = await captureDirectoryIdentity(absolute, absolute);
    await this.assertOne('', this.rootIdentity);
    this.directories.set(normalized, Object.freeze(identity));
    await this.assertAll();
  }

  async assertAll() {
    await this.assertOne('', this.rootIdentity);
    for (const [path, identity] of this.directories) await this.assertOne(path, identity);
    await this.assertOne('', this.rootIdentity);
  }

  async writeExclusive(relativePath, value) {
    const { absolute, parentPath, parentIdentity } = await this.prepareFile(relativePath);
    const handle = await open(
      absolute,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o600,
    );
    let openedIdentity;
    try {
      await handle.writeFile(value, 'utf8');
      const metadata = await handle.stat({ bigint: true });
      if (!metadata.isFile()) throw new Error(`evidence target is not a regular file ${absolute}`);
      openedIdentity = fileIdentity(metadata);
    } finally {
      await handle.close();
    }
    await this.assertFileParent(parentPath, parentIdentity);
    await assertRegularPathIdentity(absolute, openedIdentity);
  }

  async readFile(relativePath, encoding = null) {
    const { absolute, parentPath, parentIdentity } = await this.prepareFile(relativePath);
    const handle = await open(absolute, constants.O_RDONLY | constants.O_NOFOLLOW);
    let openedIdentity;
    let contents;
    try {
      const metadata = await handle.stat({ bigint: true });
      if (!metadata.isFile()) throw new Error(`evidence target is not a regular file ${absolute}`);
      openedIdentity = fileIdentity(metadata);
      contents = await handle.readFile(encoding === null ? undefined : { encoding });
    } finally {
      await handle.close();
    }
    await this.assertFileParent(parentPath, parentIdentity);
    await assertRegularPathIdentity(absolute, openedIdentity);
    return contents;
  }

  async snapshotFiles() {
    await this.assertAll();
    const files = [];
    await this.walk('', files);
    await this.assertAll();
    return files.sort((left, right) => left.path.localeCompare(right.path));
  }

  async walk(relativeDirectory, files) {
    const identity = relativeDirectory === ''
      ? this.rootIdentity
      : this.directories.get(relativeDirectory);
    if (identity === undefined) throw new Error(`unregistered evidence directory ${relativeDirectory}`);
    await this.assertOne(relativeDirectory, identity);
    const entries = await readdir(this.resolve(relativeDirectory), { withFileTypes: true });
    await this.assertOne(relativeDirectory, identity);
    for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
      const path = relativeDirectory === '' ? entry.name : `${relativeDirectory}/${entry.name}`;
      const absolute = this.resolve(path);
      const metadata = await lstat(absolute, { bigint: true });
      if (metadata.isSymbolicLink()) throw new Error(`evidence contains symlink ${absolute}`);
      if (metadata.isDirectory()) {
        if (!this.directories.has(path)) throw new Error(`unregistered evidence directory ${absolute}`);
        await this.walk(path, files);
      } else if (metadata.isFile()) {
        const bytes = await this.readFile(path);
        files.push({ path, bytes });
      } else {
        throw new Error(`evidence contains non-regular entry ${absolute}`);
      }
    }
    await this.assertOne(relativeDirectory, identity);
  }

  resolve(relativePath) {
    const normalized = relativePath === '' ? '' : normalizeRelative(relativePath);
    return normalized === '' ? this.outputDir : join(this.outputDir, ...normalized.split('/'));
  }

  async prepareFile(relativePath) {
    const normalized = normalizeRelative(relativePath);
    const absolute = this.resolve(normalized);
    const parent = dirname(absolute);
    const parentPath = slashPath(relative(this.outputDir, parent));
    const parentIdentity = parentPath === ''
      ? this.rootIdentity
      : this.directories.get(parentPath);
    if (parentIdentity === undefined) throw new Error(`unregistered evidence parent ${parent}`);
    await this.assertFileParent(parentPath, parentIdentity);
    return { absolute, parentPath, parentIdentity };
  }

  async assertFileParent(parentPath, parentIdentity) {
    await this.assertOne('', this.rootIdentity);
    if (parentPath !== '') await this.assertOne(parentPath, parentIdentity);
    await this.assertOne('', this.rootIdentity);
  }

  async assertOne(relativePath, expected) {
    const actual = await captureDirectoryIdentity(this.resolve(relativePath), expected.canonicalPath);
    if (actual.device !== expected.device || actual.inode !== expected.inode) {
      throw new Error(`evidence directory identity changed ${actual.canonicalPath}`);
    }
  }
}

async function captureDirectoryIdentity(path, expectedCanonical) {
  const before = await lstat(path, { bigint: true });
  if (before.isSymbolicLink() || !before.isDirectory()) {
    throw new Error(`evidence directory is not an original regular directory ${path}`);
  }
  const canonicalPath = await realpath(path);
  if (canonicalPath !== expectedCanonical) {
    throw new Error(`evidence directory canonical path changed ${path}`);
  }
  const after = await lstat(path, { bigint: true });
  if (!sameFile(before, after) || after.isSymbolicLink() || !after.isDirectory()) {
    throw new Error(`evidence directory changed while checking ${path}`);
  }
  return { canonicalPath, ...fileIdentity(after) };
}

async function assertRegularPathIdentity(path, expected = null) {
  const metadata = await lstat(path, { bigint: true });
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error(`evidence target is not a regular file ${path}`);
  }
  if (expected !== null && !sameIdentity(fileIdentity(metadata), expected)) {
    throw new Error(`evidence target identity changed ${path}`);
  }
}

function validateIdentityRecord(value, outputDir) {
  if (value?.schemaVersion !== PHASE0_DIRECTORY_IDENTITY_SCHEMA
    || !validIdentity(value.root, outputDir)
    || value.directories === null
    || typeof value.directories !== 'object'
    || Array.isArray(value.directories)
    || Object.keys(value.directories).length !== 1
    || !validIdentity(value.directories.commands, join(outputDir, 'commands'))) {
    throw new Error('invalid evidence directory identity record');
  }
  return value;
}

function validIdentity(value, canonicalPath) {
  return value?.canonicalPath === canonicalPath
    && typeof value.device === 'string' && /^\d+$/.test(value.device)
    && typeof value.inode === 'string' && /^\d+$/.test(value.inode);
}

function normalizeRelative(value) {
  if (typeof value !== 'string' || value === '' || isAbsolute(value) || resolve('/', value) === '/') {
    throw new Error(`invalid evidence relative path ${String(value)}`);
  }
  const parts = value.split(/[\\/]/u);
  if (parts.some((part) => part === '' || part === '.' || part === '..')) {
    throw new Error(`invalid evidence relative path ${value}`);
  }
  return parts.join('/');
}

function fileIdentity(metadata) {
  return { device: metadata.dev.toString(), inode: metadata.ino.toString() };
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}

function sameIdentity(left, right) {
  return left.device === right.device && left.inode === right.inode;
}

function slashPath(value) {
  return value.split(sep).join('/');
}
