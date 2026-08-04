import { constants as fsConstants } from 'node:fs';
import { lstat, open } from 'node:fs/promises';
import { join } from 'node:path';

const PROFILE_ACTIVATION_STATE_SCHEMA_VERSION =
  'skiff-profile-activation-state-v1';
const BOOTSTRAP_RECEIPT_SCHEMA_VERSION =
  'skiff-package-service-bootstrap-v2';
const CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION =
  'skiff-runtime-config-snapshot-record-v3';
const ASSEMBLY_IDENTITY_PATTERN =
  /^skiff-runtime-assembly-v3:sha256:[a-f0-9]{64}$/;
const CONFIG_SNAPSHOT_ID_PATTERN =
  /^skiff-runtime-config-snapshot-v1:[a-f0-9]{32}$/;
const SECURE_DIRECTORY_MODE = 0o700;
const SECURE_SNAPSHOT_FILE_MODE = 0o600;
const MAX_CONFIG_SNAPSHOT_BYTES = 16 * 1024 * 1024;

export async function buildIsolatedActivationState({
  artifactRoot,
  profile,
  bootstrap,
}) {
  if (typeof artifactRoot !== 'string' || artifactRoot.length === 0) {
    throw new Error('isolated bootstrap requires its exact artifact root');
  }
  if (
    typeof profile !== 'string'
    || profile.length === 0
    || bootstrap?.schemaVersion !== BOOTSTRAP_RECEIPT_SCHEMA_VERSION
    || bootstrap?.profile !== profile
  ) {
    throw new Error(
      'isolated bootstrap profile or receipt schema is invalid',
    );
  }
  const payload = exactObject(
    bootstrap.bootstrap,
    ['assembly', 'configSnapshot', 'generation', 'std'],
    'isolated bootstrap payload',
  );
  const assembly = exactObject(
    payload.assembly,
    ['assemblyIdentity'],
    'isolated bootstrap assembly',
  );
  const configSnapshot = exactObject(
    payload.configSnapshot,
    ['snapshotId'],
    'isolated bootstrap config snapshot',
  );
  if (
    payload.generation !== 0
    || !ASSEMBLY_IDENTITY_PATTERN.test(assembly.assemblyIdentity ?? '')
    || !CONFIG_SNAPSHOT_ID_PATTERN.test(configSnapshot.snapshotId ?? '')
  ) {
    throw new Error(
      'isolated bootstrap cannot initialize the generation-zero activation tuple',
    );
  }
  await validateSecureConfigSnapshotRecord(
    artifactRoot,
    profile,
    configSnapshot,
  );
  return {
    schemaVersion: PROFILE_ACTIVATION_STATE_SCHEMA_VERSION,
    profile,
    committed: {
      generation: payload.generation,
      assembly,
      configSnapshot,
    },
    pending: null,
  };
}

export function isolatedConfigSnapshotRecordPath(artifactRoot, configSnapshot) {
  const snapshotId = configSnapshot?.snapshotId;
  if (!CONFIG_SNAPSHOT_ID_PATTERN.test(snapshotId ?? '')) {
    throw new Error('isolated bootstrap config snapshot reference is invalid');
  }
  return join(
    artifactRoot,
    'runtime-config',
    'snapshots',
    `${snapshotId.slice(snapshotId.lastIndexOf(':') + 1)}.json`,
  );
}

async function validateSecureConfigSnapshotRecord(
  artifactRoot,
  profile,
  configSnapshot,
) {
  const storeRoot = join(artifactRoot, 'runtime-config');
  const snapshotsRoot = join(storeRoot, 'snapshots');
  const recordPath = isolatedConfigSnapshotRecordPath(
    artifactRoot,
    configSnapshot,
  );
  await Promise.all([
    assertSecureDirectory(storeRoot, 'runtime config snapshot store'),
    assertSecureDirectory(snapshotsRoot, 'runtime config snapshot records'),
  ]);
  const file = await open(
    recordPath,
    fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW,
  );
  let record;
  try {
    const metadata = await file.stat();
    if (
      !metadata.isFile()
      || (metadata.mode & 0o777) !== SECURE_SNAPSHOT_FILE_MODE
      || metadata.size > MAX_CONFIG_SNAPSHOT_BYTES
    ) {
      throw new Error(
        'isolated bootstrap config snapshot must be a bounded 0600 regular file',
      );
    }
    record = JSON.parse(await file.readFile('utf8'));
  } finally {
    await file.close();
  }
  exactObject(
    record,
    ['deployments', 'profile', 'schemaVersion', 'snapshot'],
    'isolated bootstrap config snapshot record',
  );
  if (
    record.schemaVersion !== CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION
    || record.profile !== profile
    || !Array.isArray(record.deployments)
    || record.snapshot?.snapshotId !== configSnapshot.snapshotId
  ) {
    throw new Error(
      'isolated bootstrap config snapshot record does not match its exact reference',
    );
  }
}

async function assertSecureDirectory(path, label) {
  const metadata = await lstat(path);
  if (
    metadata.isSymbolicLink()
    || !metadata.isDirectory()
    || (metadata.mode & 0o777) !== SECURE_DIRECTORY_MODE
  ) {
    throw new Error(`${label} must be a 0700 real directory`);
  }
}

function exactObject(value, keys, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (
    actual.length !== expected.length
    || actual.some((key, index) => key !== expected[index])
  ) {
    throw new Error(`${label} must have exact keys`);
  }
  return value;
}
