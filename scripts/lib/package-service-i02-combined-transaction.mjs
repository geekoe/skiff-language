import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import {
  lstat,
  readFile,
  realpath,
  rename,
  writeFile,
} from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';

import { assertIsolatedTestWorkspaceOwned } from './isolated-test-runtime-workspace.mjs';

const WITHDRAWN_ROOT_SUFFIX = '.p5-i02-withdrawn';

export function createI02TransactionDeadline({ timeoutMs, parentSignalTarget }) {
  assert.ok(
    Number.isSafeInteger(timeoutMs) && timeoutMs > 0,
    'I02 transaction deadline must be a positive safe integer',
  );
  const signalTarget = new EventEmitter();
  const forwarders = new Map(
    ['SIGINT', 'SIGTERM'].map((signal) => [
      signal,
      () => signalTarget.emit(signal),
    ]),
  );
  for (const [signal, forward] of forwarders) parentSignalTarget.on(signal, forward);
  const timeout = setTimeout(() => signalTarget.emit('SIGTERM'), timeoutMs);
  return {
    signalTarget,
    dispose: () => {
      clearTimeout(timeout);
      for (const [signal, forward] of forwarders) {
        parentSignalTarget.off(signal, forward);
      }
    },
  };
}

export async function withI02ArtifactRootWithdrawn(
  stack,
  operation,
  operations = {},
) {
  const ops = {
    assertOwned: assertIsolatedTestWorkspaceOwned,
    lstat,
    realpath,
    rename,
    ...operations,
  };
  await assertExactOwnedArtifactRoot(stack, ops);
  const artifactRoot = resolve(stack.artifactRoot);
  const withdrawnRoot = `${artifactRoot}${WITHDRAWN_ROOT_SUFFIX}`;
  assertStrictlyContained(stack.ownershipReceipt.root.path, withdrawnRoot, 'withdrawn root');
  await assertPathMissing(withdrawnRoot, ops.lstat);
  const original = await directoryIdentity(artifactRoot, ops);
  await ops.rename(artifactRoot, withdrawnRoot);
  let value;
  let primaryError;
  try {
    value = await operation();
  } catch (error) {
    primaryError = error;
  }
  let restoreError;
  try {
    await ops.assertOwned(stack.ownershipReceipt, { requireConfig: true });
    await assertPathMissing(artifactRoot, ops.lstat);
    await assertSameDirectory(withdrawnRoot, original, ops);
    await ops.rename(withdrawnRoot, artifactRoot);
    await assertSameDirectory(artifactRoot, original, ops);
  } catch (error) {
    restoreError = error;
  }
  if (primaryError !== undefined && restoreError !== undefined) {
    throw new AggregateError(
      [primaryError, restoreError],
      'I02 unary failed and exact artifact-root restoration also failed',
    );
  }
  if (primaryError !== undefined) throw primaryError;
  if (restoreError !== undefined) throw restoreError;
  return {
    value,
    evidence: Object.freeze({
      exactPath: artifactRoot,
      withdrawnPath: withdrawnRoot,
      workspaceNonce: stack.ownershipReceipt.nonce,
      restored: true,
      requestArtifactIo: 0,
    }),
  };
}

export async function withTamperedI02PackageRecord(
  stack,
  transitive,
  operation,
  operations = {},
) {
  const ops = {
    assertOwned: assertIsolatedTestWorkspaceOwned,
    readFile,
    writeFile,
    ...operations,
  };
  await assertExactOwnedArtifactRoot(stack, {
    ...ops,
    lstat,
    realpath,
  });
  const recordPath = resolveI02OwnedArtifactPath(stack, transitive.relativePath);
  const original = await ops.readFile(recordPath);
  const record = JSON.parse(original.toString('utf8'));
  assert.equal(record.packageId, transitive.artifact.packageId);
  assert.equal(record.packageVersion, transitive.artifact.packageVersion);
  assert.equal(record.packageBuildId, transitive.artifact.packageBuildId);
  const tampered = {
    ...record,
    packageId: 'test.skiff/i02-tampered-transitive-package',
  };
  await ops.writeFile(recordPath, `${JSON.stringify(tampered)}\n`, { flag: 'w' });
  const observed = JSON.parse(await ops.readFile(recordPath, 'utf8'));
  assert.equal(observed.packageId, tampered.packageId);
  let value;
  let primaryError;
  try {
    value = await operation();
  } catch (error) {
    primaryError = error;
  }
  let restoreError;
  try {
    await ops.assertOwned(stack.ownershipReceipt, { requireConfig: true });
    await ops.writeFile(recordPath, original, { flag: 'w' });
    assert.deepEqual(await ops.readFile(recordPath), original);
  } catch (error) {
    restoreError = error;
  }
  if (primaryError !== undefined && restoreError !== undefined) {
    throw new AggregateError(
      [primaryError, restoreError],
      'I02 activation failed and transitive-record restoration also failed',
    );
  }
  if (primaryError !== undefined) throw primaryError;
  if (restoreError !== undefined) throw restoreError;
  return {
    value,
    evidence: Object.freeze({
      recordPath: transitive.relativePath,
      workspaceNonce: stack.ownershipReceipt.nonce,
      candidatePatched: false,
      candidateResigned: false,
      recordRestored: true,
    }),
  };
}

export function resolveI02OwnedArtifactPath(stack, relativePath) {
  assert.equal(isAbsolute(relativePath), false, 'I02 artifact record path must be relative');
  const absolute = resolve(stack.artifactRoot, relativePath);
  assertStrictlyContained(stack.artifactRoot, absolute, 'artifact record');
  assertStrictlyContained(
    stack.ownershipReceipt.root.path,
    absolute,
    'owned artifact record',
  );
  return absolute;
}

async function assertExactOwnedArtifactRoot(stack, ops) {
  assert.ok(stack?.ownershipReceipt, 'I02 requires an isolated workspace ownership marker');
  await ops.assertOwned(stack.ownershipReceipt, { requireConfig: true });
  const artifactRoot = resolve(stack.artifactRoot);
  assert.equal(
    artifactRoot,
    resolve(stack.devHome, 'artifacts'),
    'I02 may move only the isolated runtime canonical artifact root',
  );
  assertStrictlyContained(
    stack.ownershipReceipt.root.path,
    artifactRoot,
    'canonical artifact root',
  );
  const resolvedRoot = await ops.realpath(artifactRoot);
  assertStrictlyContained(
    stack.ownershipReceipt.root.realPath,
    resolvedRoot,
    'resolved canonical artifact root',
  );
  await directoryIdentity(artifactRoot, ops);
}

function assertStrictlyContained(root, candidate, label) {
  const relativePath = relative(resolve(root), resolve(candidate));
  if (
    relativePath.length === 0
    || relativePath === '..'
    || relativePath.startsWith(`..${sep}`)
    || isAbsolute(relativePath)
  ) {
    throw new Error(`${label} must be strictly inside ${root}: ${candidate}`);
  }
}

async function directoryIdentity(path, ops) {
  const status = await ops.lstat(path, { bigint: true });
  assert.equal(status.isDirectory(), true, `I02 path must be a directory: ${path}`);
  return Object.freeze({
    dev: status.dev.toString(),
    ino: status.ino.toString(),
  });
}

async function assertSameDirectory(path, expected, ops) {
  assert.deepEqual(await directoryIdentity(path, ops), expected);
}

async function assertPathMissing(path, inspect) {
  try {
    await inspect(path);
  } catch (error) {
    if (error?.code === 'ENOENT') return;
    throw error;
  }
  throw new Error(`I02 exact move destination already exists: ${path}`);
}

export const packageServiceI02CombinedTransactionConstants = Object.freeze({
  withdrawnRootSuffix: WITHDRAWN_ROOT_SUFFIX,
});
