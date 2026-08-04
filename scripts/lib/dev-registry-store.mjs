import { randomUUID } from 'node:crypto';
import {
  mkdir,
  open,
  readFile,
  rename,
  rm,
} from 'node:fs/promises';
import { dirname, isAbsolute, resolve } from 'node:path';

export const devRegistrySchemaVersion = 'skiff-package-service-dev-registry-v2';

export function emptyDevRegistry() {
  return {
    schemaVersion: devRegistrySchemaVersion,
    profile: 'dev',
    roots: [],
  };
}

export async function readStoredDevRegistry(
  path,
  {
    allowMissing = false,
  } = {},
) {
  const registryPath = resolve(path);
  let value;
  try {
    value = JSON.parse(await readFile(registryPath, 'utf8'));
  } catch (error) {
    if (allowMissing && error?.code === 'ENOENT') {
      return emptyDevRegistry();
    }
    throw new Error(
      `failed to read dev registry ${registryPath}: ${formatError(error)}`,
      { cause: error },
    );
  }
  return normalizeStoredDevRegistry(value, registryPath);
}

export async function writeStoredDevRegistry(path, registry) {
  const registryPath = resolve(path);
  const normalized = normalizeStoredDevRegistry(registry, registryPath);
  const parent = dirname(registryPath);
  await mkdir(parent, { recursive: true });
  const temporaryPath = `${registryPath}.tmp-${process.pid}-${randomUUID()}`;
  let temporary;
  try {
    temporary = await open(temporaryPath, 'wx', 0o600);
    await temporary.writeFile(`${JSON.stringify(normalized, null, 2)}\n`, 'utf8');
    await temporary.sync();
    await temporary.close();
    temporary = undefined;
    await rename(temporaryPath, registryPath);
    await syncDirectory(parent);
  } catch (error) {
    await temporary?.close().catch(() => {});
    await rm(temporaryPath, { force: true }).catch(() => {});
    throw error;
  }
  return normalized;
}

export function normalizeStoredDevRegistry(value, label = 'dev registry') {
  if (!isPlainObject(value)) {
    throw new Error(`${label} must contain a JSON object`);
  }
  exactFields(value, ['profile', 'roots', 'schemaVersion'], label);
  if (value.schemaVersion !== devRegistrySchemaVersion) {
    throw new Error(`${label} schemaVersion must be ${devRegistrySchemaVersion}`);
  }
  assertProfile(value.profile);
  if (!Array.isArray(value.roots)) {
    throw new Error(`${label} roots must be an array`);
  }
  const roots = value.roots.map((entry, index) =>
    normalizeStoredEntry(entry, `${label} root ${index}`));
  roots.sort((left, right) =>
    left.kind.localeCompare(right.kind)
    || left.root.localeCompare(right.root)
    || (left.serviceId ?? '').localeCompare(right.serviceId ?? ''));
  rejectDuplicateValues(roots, 'root', label);
  rejectDuplicateValues(
    roots.filter((entry) => entry.serviceId !== undefined),
    'serviceId',
    label,
  );
  return {
    schemaVersion: devRegistrySchemaVersion,
    profile: value.profile,
    roots,
  };
}

export function assertProfile(value) {
  if (typeof value !== 'string' || !/^(?!\.{1,2}$)[A-Za-z0-9._-]{1,200}$/.test(value)) {
    throw new Error('profile must use only letters, digits, dot, dash, or underscore');
  }
}

export function assertServiceId(value, label = 'serviceId') {
  if (
    typeof value !== 'string'
    || value.length === 0
    || value.length > 200
    || value.trim() !== value
    || !/^[A-Za-z0-9](?:[A-Za-z0-9._/-]*[A-Za-z0-9])?$/.test(value)
    || value.includes('//')
    || value.includes('/./')
    || value.includes('/../')
  ) {
    throw new Error(`${label} must be a canonical service ID`);
  }
}

function normalizeStoredEntry(value, label) {
  if (!isPlainObject(value)) {
    throw new Error(`${label} must be an object`);
  }
  const allowed = value.serviceId === undefined
    ? ['kind', 'root']
    : ['kind', 'root', 'serviceId'];
  exactFields(value, allowed, label);
  if (value.kind !== 'package' && value.kind !== 'service') {
    throw new Error(`${label}.kind must be package or service`);
  }
  if (
    typeof value.root !== 'string'
    || !isAbsolute(value.root)
    || resolve(value.root) !== value.root
  ) {
    throw new Error(`${label}.root must be a canonical absolute path`);
  }
  if (value.kind === 'service' && value.serviceId === undefined) {
    throw new Error(`${label}.serviceId is required for a service root`);
  }
  if (value.kind === 'package' && value.serviceId !== undefined) {
    throw new Error(`${label}.serviceId is allowed only for a service root`);
  }
  if (value.serviceId !== undefined) {
    assertServiceId(value.serviceId, `${label}.serviceId`);
  }
  return {
    kind: value.kind,
    root: value.root,
    ...(value.serviceId === undefined ? {} : { serviceId: value.serviceId }),
  };
}

function rejectDuplicateValues(entries, field, label) {
  const seen = new Set();
  for (const entry of entries) {
    if (seen.has(entry[field])) {
      throw new Error(`${label} contains duplicate ${field} ${entry[field]}`);
    }
    seen.add(entry[field]);
  }
}

function exactFields(value, expected, label) {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  if (
    actual.length !== sortedExpected.length
    || actual.some((field, index) => field !== sortedExpected[index])
  ) {
    throw new Error(`${label} fields must be exactly ${sortedExpected.join(', ')}`);
  }
}

async function syncDirectory(path) {
  if (process.platform === 'win32') {
    return;
  }
  let directory;
  try {
    directory = await open(path, 'r');
    await directory.sync();
  } finally {
    await directory?.close().catch(() => {});
  }
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function formatError(error) {
  return error instanceof Error ? error.message : String(error);
}
