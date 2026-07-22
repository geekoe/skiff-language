import { randomBytes } from 'node:crypto';
import { lstat, open, readFile, realpath, rm } from 'node:fs/promises';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';

const RECEIPT_SCHEMA = 'skiff-isolated-test-workspace-owner-v1';
const MARKER_NAME = '.skiff-isolated-workspace-owner.json';

export async function claimIsolatedTestWorkspace(rootPath) {
  const root = resolve(rootPath);
  const rootOwner = await captureDirectoryOwner(root, 'workspace root');
  const nonce = randomBytes(16).toString('hex');
  const markerPath = join(root, MARKER_NAME);
  const markerContents = serializeMarker({ nonce, root });
  const markerHandle = await open(markerPath, 'wx', 0o600);
  try {
    await markerHandle.writeFile(markerContents, 'utf8');
    await markerHandle.sync();
  } finally {
    await markerHandle.close();
  }
  const markerOwner = await captureFileOwner(markerPath, 'workspace marker');
  return freezeReceipt({
    schemaVersion: RECEIPT_SCHEMA,
    nonce,
    root: { path: root, ...rootOwner },
    marker: { path: markerPath, ...markerOwner },
  });
}

export async function captureIsolatedTestConfig(receipt, configPath) {
  await assertIsolatedTestWorkspaceOwned(receipt);
  if (receipt.config !== undefined) {
    throw ownershipError(receipt, 'instance config ownership was already captured');
  }
  const absoluteConfigPath = resolve(configPath);
  assertContainedPath(receipt.root.path, absoluteConfigPath, 'instance config');
  const configOwner = await captureFileOwner(absoluteConfigPath, 'instance config');
  assertContainedPath(receipt.root.realPath, configOwner.realPath, 'resolved instance config');
  return freezeReceipt({
    ...receipt,
    config: { path: absoluteConfigPath, ...configOwner },
  });
}

export async function assertIsolatedTestWorkspaceOwned(receipt, {
  requireConfig = false,
} = {}) {
  assertReceiptShape(receipt);
  const expectedMarkerPath = join(receipt.root.path, MARKER_NAME);
  if (receipt.marker.path !== expectedMarkerPath) {
    throw ownershipError(receipt, 'workspace marker path is not canonical');
  }
  try {
    await assertOwnedPath(
      receipt.root,
      (status) => status.isDirectory(),
      'workspace root',
    );
    await assertOwnedPath(
      receipt.marker,
      (status) => status.isFile(),
      'workspace marker',
    );
    const actualMarker = await readFile(receipt.marker.path, 'utf8');
    if (actualMarker !== serializeMarker(receipt)) {
      throw ownershipError(receipt, 'workspace marker contents changed');
    }
    if (requireConfig && receipt.config === undefined) {
      throw ownershipError(receipt, 'instance config ownership was not captured');
    }
    if (receipt.config !== undefined) {
      assertContainedPath(receipt.root.path, receipt.config.path, 'instance config');
      assertContainedPath(receipt.root.realPath, receipt.config.realPath, 'resolved instance config');
      await assertOwnedPath(
        receipt.config,
        (status) => status.isFile(),
        'instance config',
      );
    }
  } catch (error) {
    if (error instanceof IsolatedTestWorkspaceOwnershipError) {
      throw error;
    }
    throw ownershipError(receipt, errorMessage(error), error);
  }
  return receipt;
}

export async function removeOwnedIsolatedTestWorkspace(receipt) {
  await assertIsolatedTestWorkspaceOwned(receipt);
  await rm(receipt.root.path, { recursive: true });
}

class IsolatedTestWorkspaceOwnershipError extends Error {
  constructor(message, { cause, receipt } = {}) {
    super(message, { cause });
    this.name = 'IsolatedTestWorkspaceOwnershipError';
    this.receipt = receipt;
  }
}

function freezeReceipt(receipt) {
  const frozen = {
    ...receipt,
    root: freezeOwnedPath(receipt.root),
    marker: freezeOwnedPath(receipt.marker),
    ...(receipt.config === undefined ? {} : { config: freezeOwnedPath(receipt.config) }),
  };
  return Object.freeze(frozen);
}

function freezeOwnedPath(ownedPath) {
  return Object.freeze({
    ...ownedPath,
    identity: Object.freeze({ ...ownedPath.identity }),
  });
}

function assertReceiptShape(receipt) {
  if (
    receipt?.schemaVersion !== RECEIPT_SCHEMA
    || !/^[0-9a-f]{32}$/.test(receipt?.nonce ?? '')
    || !validOwnedPath(receipt?.root)
    || !validOwnedPath(receipt?.marker)
    || (receipt?.config !== undefined && !validOwnedPath(receipt.config))
    || resolve(receipt.root.path) !== receipt.root.path
    || resolve(receipt.marker.path) !== receipt.marker.path
  ) {
    throw ownershipError(receipt, 'workspace ownership receipt is invalid');
  }
}

function validOwnedPath(ownedPath) {
  return typeof ownedPath?.path === 'string'
    && typeof ownedPath?.realPath === 'string'
    && typeof ownedPath?.identity?.dev === 'string'
    && typeof ownedPath?.identity?.ino === 'string'
    && resolve(ownedPath.path) === ownedPath.path
    && resolve(ownedPath.realPath) === ownedPath.realPath;
}

async function captureDirectoryOwner(path, label) {
  const status = await lstat(path, { bigint: true });
  if (!status.isDirectory()) {
    throw new Error(`${label} is not a directory: ${path}`);
  }
  return { identity: fileIdentity(status), realPath: await realpath(path) };
}

async function captureFileOwner(path, label) {
  const status = await lstat(path, { bigint: true });
  if (!status.isFile()) {
    throw new Error(`${label} is not a regular file: ${path}`);
  }
  return { identity: fileIdentity(status), realPath: await realpath(path) };
}

async function assertOwnedPath(ownedPath, matchesType, label) {
  const status = await lstat(ownedPath.path, { bigint: true });
  const actual = fileIdentity(status);
  if (
    !matchesType(status)
    || actual.dev !== ownedPath.identity?.dev
    || actual.ino !== ownedPath.identity?.ino
  ) {
    throw new Error(
      `${label} identity changed at ${ownedPath.path}; `
      + `expected dev=${ownedPath.identity?.dev} ino=${ownedPath.identity?.ino}, `
      + `actual dev=${actual.dev} ino=${actual.ino}`,
    );
  }
  const actualRealPath = await realpath(ownedPath.path);
  if (actualRealPath !== ownedPath.realPath) {
    throw new Error(
      `${label} resolved path changed at ${ownedPath.path}; `
      + `expected ${ownedPath.realPath}, actual ${actualRealPath}`,
    );
  }
}

function fileIdentity(status) {
  return Object.freeze({
    dev: status.dev.toString(),
    ino: status.ino.toString(),
  });
}

function assertContainedPath(root, candidate, label) {
  const relativePath = relative(root, candidate);
  if (
    relativePath.length === 0
    || relativePath === '..'
    || relativePath.startsWith(`..${sep}`)
    || isAbsolute(relativePath)
  ) {
    throw new Error(`${label} must be strictly inside owned workspace ${root}: ${candidate}`);
  }
}

function serializeMarker({ nonce, root }) {
  const rootPath = typeof root === 'string' ? root : root.path;
  return `${JSON.stringify({ schemaVersion: RECEIPT_SCHEMA, nonce, root: rootPath })}\n`;
}

function ownershipError(receipt, detail, cause) {
  const nonce = typeof receipt?.nonce === 'string' ? receipt.nonce : '<invalid>';
  const root = typeof receipt?.root?.path === 'string' ? receipt.root.path : '<invalid>';
  return new IsolatedTestWorkspaceOwnershipError(
    `isolated workspace ownership mismatch for ${root} (nonce ${nonce}): ${detail}`,
    { cause, receipt },
  );
}

function errorMessage(error) {
  return error?.message || String(error);
}
