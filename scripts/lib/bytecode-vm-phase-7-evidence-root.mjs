import {
  lstat,
  mkdir,
  realpath,
} from 'node:fs/promises';
import { join } from 'node:path';

import { Phase0EvidenceRoot } from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE7_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-7-directory-identity-r1-v1';
export const PHASE7_DIRECTORY_IDENTITY_FILE =
  'phase-7-r1-v1-directory-identities.json';

export async function createPhase7EvidenceRoot(outputDir) {
  await mkdir(outputDir, { mode: 0o700 });
  await mkdir(join(outputDir, 'commands'), { mode: 0o700 });
  await mkdir(join(outputDir, 'observations'), { mode: 0o700 });
  const rootIdentity = await captureDirectoryIdentity(outputDir, outputDir);
  const commandsIdentity = await captureDirectoryIdentity(
    join(outputDir, 'commands'),
    join(outputDir, 'commands'),
  );
  const observationsIdentity = await captureDirectoryIdentity(
    join(outputDir, 'observations'),
    join(outputDir, 'observations'),
  );
  const evidenceRoot = new Phase0EvidenceRoot(
    outputDir,
    rootIdentity,
    new Map([
      ['commands', Object.freeze(commandsIdentity)],
      ['observations', Object.freeze(observationsIdentity)],
    ]),
    PHASE7_DIRECTORY_IDENTITY_SCHEMA,
    PHASE7_DIRECTORY_IDENTITY_FILE,
  );
  await evidenceRoot.writeExclusive(
    PHASE7_DIRECTORY_IDENTITY_FILE,
    `${JSON.stringify(evidenceRoot.identities(), null, 2)}\n`,
  );
  await evidenceRoot.assertAll();
  return evidenceRoot;
}

export async function openPhase7EvidenceRoot(outputDir, expectedIdentities) {
  const identities = validateIdentityRecord(expectedIdentities, outputDir);
  const evidenceRoot = new Phase0EvidenceRoot(
    outputDir,
    identities.root,
    new Map(Object.entries(identities.directories)),
    PHASE7_DIRECTORY_IDENTITY_SCHEMA,
    PHASE7_DIRECTORY_IDENTITY_FILE,
  );
  await evidenceRoot.assertAll();
  const stored = JSON.parse(await evidenceRoot.readFile(PHASE7_DIRECTORY_IDENTITY_FILE, 'utf8'));
  if (JSON.stringify(stored) !== JSON.stringify(identities)) {
    throw new Error('evidence directory identity record drifted');
  }
  await evidenceRoot.assertAll();
  return evidenceRoot;
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

function validateIdentityRecord(value, outputDir) {
  if (value?.schemaVersion !== PHASE7_DIRECTORY_IDENTITY_SCHEMA
    || !validIdentity(value.root, outputDir)
    || value.directories === null
    || typeof value.directories !== 'object'
    || Array.isArray(value.directories)
    || Object.keys(value.directories).length !== 2
    || !validIdentity(value.directories.commands, join(outputDir, 'commands'))
    || !validIdentity(value.directories.observations, join(outputDir, 'observations'))) {
    throw new Error('invalid evidence directory identity record');
  }
  return value;
}

function validIdentity(value, canonicalPath) {
  return value?.canonicalPath === canonicalPath
    && typeof value.device === 'string' && /^\d+$/.test(value.device)
    && typeof value.inode === 'string' && /^\d+$/.test(value.inode);
}

function fileIdentity(metadata) {
  return { device: metadata.dev.toString(), inode: metadata.ino.toString() };
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino;
}