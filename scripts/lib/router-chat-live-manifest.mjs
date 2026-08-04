// Canonical `router-live:chat` service artifact manifest (plan §8).
//
// The gate pins the three repositories (Skiff public, internals private,
// skiff-packages) and every service artifact identity produced from those
// pins, then runs the Agine chat smoke against an isolated Rust Router
// instance. Local verification and the real CI private workflow (owned by the
// internals repository) share exactly this schema.

const SCHEMA_VERSION = 'skiff-router-chat-live-manifest-v1';

const COMMIT_PATTERN = /^[0-9a-f]{40}$/;
const PROFILE_PATTERN = /^[A-Za-z0-9._-]{1,200}$/;
const SERVICE_ID_PATTERN =
  /^[A-Za-z0-9][A-Za-z0-9._-]*(\/[A-Za-z0-9][A-Za-z0-9._-]*)+$/;
const VERSION_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const ASSEMBLY_IDENTITY_PATTERN =
  /^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$/;
const CONFIG_SNAPSHOT_ID_PATTERN =
  /^skiff-runtime-config-snapshot-v1:[0-9a-f]{32}$/;
const DEPLOYMENT_REVISION_PATTERN = /^sha256-[0-9a-f]{64}$/;
const DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN =
  /^skiff-deployment-artifact-v4:sha256:[0-9a-f]{64}$/;
const PACKAGE_BUILD_ID_PATTERN =
  /^skiff-package-build-v10:sha256:[0-9a-f]{64}$/;
const PACKAGE_LOCAL_ABI_IDENTITY_PATTERN =
  /^skiff-package-local-abi-v7:sha256:[0-9a-f]{64}$/;
const STATUS_PATTERN = /^(PASS|FAIL)$/;

export function routerChatLiveManifestSchemaVersion() {
  return SCHEMA_VERSION;
}

/**
 * Strictly validates one router-live:chat manifest and returns a frozen copy.
 * Unknown keys, wrong types, and identity pattern mismatches are rejected so
 * local evidence and CI records cannot drift from the §8 schema.
 */
export function validateRouterChatLiveManifest(value, label = 'manifest') {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  exactKeys(value, [
    'schemaVersion',
    'pinned',
    'profile',
    'generation',
    'assembly',
    'configSnapshot',
    'services',
    'packages',
    'smoke',
  ], label);
  if (value.schemaVersion !== SCHEMA_VERSION) {
    throw new Error(`${label} schemaVersion must be ${SCHEMA_VERSION}`);
  }

  const pinned = validatePinned(value.pinned, `${label}.pinned`);
  const profile = validatePattern(
    value.profile,
    PROFILE_PATTERN,
    `${label}.profile`,
  );
  if (!Number.isSafeInteger(value.generation) || value.generation < 0) {
    throw new Error(`${label}.generation must be a non-negative safe integer`);
  }
  const assembly = exactObject(value.assembly, ['assemblyIdentity'], `${label}.assembly`);
  const assemblyIdentity = validatePattern(
    assembly.assemblyIdentity,
    ASSEMBLY_IDENTITY_PATTERN,
    `${label}.assembly.assemblyIdentity`,
  );
  const configSnapshot = exactObject(
    value.configSnapshot,
    ['snapshotId'],
    `${label}.configSnapshot`,
  );
  const snapshotId = validatePattern(
    configSnapshot.snapshotId,
    CONFIG_SNAPSHOT_ID_PATTERN,
    `${label}.configSnapshot.snapshotId`,
  );
  const services = value.services.map((entry, index) =>
    validateServiceEntry(entry, `${label}.services[${index}]`));
  const packages = value.packages.map((entry, index) =>
    validatePackageEntry(entry, `${label}.packages[${index}]`));
  const smoke = validateSmoke(value.smoke, `${label}.smoke`);

  return deepFreeze({
    schemaVersion: SCHEMA_VERSION,
    pinned,
    profile,
    generation: value.generation,
    assembly: { assemblyIdentity },
    configSnapshot: { snapshotId },
    services,
    packages,
    smoke,
  });
}

function validatePinned(value, label) {
  exactKeys(value, ['skiff', 'internals', 'skiffPackages'], label);
  const entries = {};
  for (const key of ['skiff', 'internals', 'skiffPackages']) {
    const repository = exactKeys(
      value[key],
      ['repository', 'commit'],
      `${label}.${key}`,
    );
    entries[key] = {
      repository: validatePattern(
        repository.repository,
        /^[A-Za-z0-9][A-Za-z0-9._-]*$/,
        `${label}.${key}.repository`,
      ),
      commit: validatePattern(
        repository.commit,
        COMMIT_PATTERN,
        `${label}.${key}.commit`,
      ),
    };
  }
  return entries;
}

function validateServiceEntry(value, label) {
  exactKeys(value, [
    'serviceId',
    'contractVersion',
    'deploymentRevision',
    'deploymentArtifactIdentity',
    'implementationPackageBuildId',
  ], label);
  return deepFreeze({
    serviceId: validatePattern(value.serviceId, SERVICE_ID_PATTERN, `${label}.serviceId`),
    contractVersion: validatePattern(
      value.contractVersion,
      VERSION_PATTERN,
      `${label}.contractVersion`,
    ),
    deploymentRevision: validatePattern(
      value.deploymentRevision,
      DEPLOYMENT_REVISION_PATTERN,
      `${label}.deploymentRevision`,
    ),
    deploymentArtifactIdentity: validatePattern(
      value.deploymentArtifactIdentity,
      DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN,
      `${label}.deploymentArtifactIdentity`,
    ),
    implementationPackageBuildId: validatePattern(
      value.implementationPackageBuildId,
      PACKAGE_BUILD_ID_PATTERN,
      `${label}.implementationPackageBuildId`,
    ),
  });
}

function validatePackageEntry(value, label) {
  exactKeys(value, [
    'packageId',
    'packageVersion',
    'packageBuildId',
    'packageLocalAbiIdentity',
  ], label);
  return deepFreeze({
    packageId: validatePattern(
      value.packageId,
      SERVICE_ID_PATTERN,
      `${label}.packageId`,
    ),
    packageVersion: validatePattern(
      value.packageVersion,
      VERSION_PATTERN,
      `${label}.packageVersion`,
    ),
    packageBuildId: validatePattern(
      value.packageBuildId,
      PACKAGE_BUILD_ID_PATTERN,
      `${label}.packageBuildId`,
    ),
    packageLocalAbiIdentity: validatePattern(
      value.packageLocalAbiIdentity,
      PACKAGE_LOCAL_ABI_IDENTITY_PATTERN,
      `${label}.packageLocalAbiIdentity`,
    ),
  });
}

function validateSmoke(value, label) {
  exactKeys(value, ['command', 'cwd', 'ingressBase', 'status', 'finishedAt'], label);
  return deepFreeze({
    command: validatePattern(
      value.command,
      /^\S+( \S+)*$/,
      `${label}.command`,
    ),
    cwd: validateNonEmptyString(value.cwd, `${label}.cwd`),
    ingressBase: validatePattern(
      value.ingressBase,
      /^https?:\/\/127\.0\.0\.1:[0-9]{2,5}$/,
      `${label}.ingressBase`,
    ),
    status: validatePattern(value.status, STATUS_PATTERN, `${label}.status`),
    finishedAt: validatePattern(
      value.finishedAt,
      /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/,
      `${label}.finishedAt`,
    ),
  });
}

function exactKeys(value, expected, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new Error(`${label} must contain exactly ${expected.join(', ')}`);
  }
  return value;
}

function exactObject(value, keys, label) {
  exactKeys(value, keys, label);
  return value;
}

function validatePattern(value, pattern, label) {
  if (typeof value !== 'string' || !pattern.test(value)) {
    throw new Error(`${label} must match ${pattern}`);
  }
  return value;
}

function validateNonEmptyString(value, label) {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
