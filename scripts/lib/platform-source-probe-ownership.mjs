import { isAbsolute, join } from 'node:path';

import {
  errorMessage,
  probeDigest,
  samePathIdentity,
} from './platform-source-probe-support.mjs';

const OWNER_SCHEMA = 'skiff-platform-source-probe-owner-v1';

export function parseWorktreeRegistry(output) {
  return output.split('\0\0').filter(Boolean).map((record) => {
    const fields = record.split('\0');
    const path = fields.shift()?.replace(/^worktree /, '');
    const head = fields.find((field) => field.startsWith('HEAD '))?.slice(5);
    const branch = fields.find((field) => field.startsWith('branch '))?.slice(7) ?? null;
    const detached = fields.includes('detached');
    const identity = probeDigest({ path, head, branch, detached });
    return { path, head, branch, detached, identity };
  });
}

export async function readWorktreeRegistry(input, deps, checked) {
  const outcome = await checked(deps, 'git', [
    '-C', input.integrationRoot, 'worktree', 'list', '--porcelain', '-z',
  ], { cwd: input.integrationRoot });
  return parseWorktreeRegistry(outcome.stdout);
}

export async function assertProbeWorktreesUnregistered(input, deps, checked) {
  const registry = await readWorktreeRegistry(input, deps, checked);
  for (const path of [input.aWorktree, input.bWorktree]) {
    if (registry.some((entry) => entry.path === path)) {
      throw new Error(`worktree is already registered: ${path}`);
    }
  }
}

export async function createProbeOwnership({ input, deps, ledger, taskRoot }) {
  const markerPath = join(taskRoot, '.skiff-platform-source-probe-owner.json');
  const marker = `${JSON.stringify({
    schemaVersion: OWNER_SCHEMA,
    nonce: ledger.probeNonce,
    taskRoot,
    candidate: input.candidate,
  })}\n`;
  const taskRootIdentity = await deps.pathIdentity(taskRoot);
  if (taskRootIdentity?.kind !== 'directory') {
    throw new Error('probe task root is not an owned directory');
  }
  const markerIdentity = await deps.writeExclusiveDurable(markerPath, marker);
  return {
    input,
    deps,
    ledger,
    taskRoot,
    taskRootIdentity,
    markerPath,
    marker,
    markerIdentity,
    claims: [],
    worktrees: [],
    foreignPaths: [],
    foreignRegistries: [],
  };
}

export async function addOwnedWorktree(owner, label, path, checked, signal) {
  const claim = await createClaim(owner, label, path);
  let registry = await readWorktreeRegistry(owner.input, owner.deps, checked);
  const pathBefore = await owner.deps.pathIdentity(path);
  const registryBefore = registry.find((entry) => entry.path === path) ?? null;
  if (pathBefore !== null || registryBefore !== null) {
    rememberForeign(owner, path, pathBefore, registryBefore, 'before-add');
    throw new Error(`${label} worktree target changed after preflight`);
  }

  let addError;
  try {
    await checked(owner.deps, 'git', [
      '-C', owner.input.integrationRoot,
      'worktree', 'add', '--detach', path, owner.input.candidate,
    ], { cwd: owner.input.integrationRoot, signal });
  } catch (error) {
    addError = error;
  }

  let captureError;
  let observedPathIdentity = null;
  let observedRegistryEntry = null;
  try {
    registry = await readWorktreeRegistry(owner.input, owner.deps, checked);
    observedPathIdentity = await owner.deps.pathIdentity(path);
    observedRegistryEntry = registry.find((entry) => entry.path === path) ?? null;
    const claimVerified = await verifyClaim(owner, claim);
    const registryIdentity = observedPathIdentity?.kind === 'directory'
      && observedRegistryEntry !== null
      ? await captureRegistryIdentity(owner, path, observedRegistryEntry)
      : null;
    if (
      registryIdentity !== null
      && observedRegistryEntry?.head === owner.input.candidate
      && observedRegistryEntry.detached === true
      && claimVerified
    ) {
      owner.worktrees.push({
        label,
        path,
        pathIdentity: observedPathIdentity,
        registryIdentity,
        claim,
      });
    } else {
      rememberForeign(
        owner,
        path,
        observedPathIdentity,
        observedRegistryEntry,
        'incomplete-add',
      );
      captureError = new Error(
        `${label} worktree add did not establish complete ownership `
        + `(path=${observedPathIdentity?.kind ?? 'absent'}, `
        + `registry=${observedRegistryEntry?.head ?? 'absent'}, `
        + `detached=${observedRegistryEntry?.detached === true}, claim=${claimVerified})`,
      );
    }
  } catch (error) {
    rememberForeign(
      owner,
      path,
      observedPathIdentity,
      observedRegistryEntry,
      'add-inspection-failure',
    );
    captureError = error;
  }
  if (addError !== undefined) throw addError;
  if (captureError !== undefined) throw captureError;
}

export async function cleanupProbeOwnership(owner, checked) {
  const errors = [];
  const worktrees = [];
  for (const resource of [...owner.worktrees].reverse()) {
    const proof = {
      label: resource.label,
      path: resource.path,
      pathIdentity: resource.pathIdentity,
      registryIdentity: resource.registryIdentity,
      claimPath: resource.claim.path,
      claimIdentity: resource.claim.identity,
      claimDigest: resource.claim.digest,
      claimVerifiedBeforeRemoval: false,
      pathAbsent: false,
      registryAbsent: false,
      registryStorageAbsent: false,
      error: null,
    };
    try {
      proof.claimVerifiedBeforeRemoval = await verifyClaim(owner, resource.claim);
      const pathMatches = samePathIdentity(
        await owner.deps.pathIdentity(resource.path),
        resource.pathIdentity,
      );
      const registry = await readWorktreeRegistry(owner.input, owner.deps, checked);
      const current = registry.find((entry) => entry.path === resource.path) ?? null;
      const currentIdentity = pathMatches && current !== null
        ? await captureRegistryIdentity(owner, resource.path, current)
        : null;
      const registryMatches = sameRegistryIdentity(currentIdentity, resource.registryIdentity);
      if (!proof.claimVerifiedBeforeRemoval || !pathMatches || !registryMatches) {
        rememberForeign(
          owner,
          resource.path,
          await owner.deps.pathIdentity(resource.path),
          current,
          'cleanup-replacement',
        );
        throw new Error(`${resource.label} ownership changed before cleanup`);
      }
      await checked(owner.deps, 'git', [
        '-C', owner.input.integrationRoot, 'worktree', 'remove', resource.path,
      ], { cwd: owner.input.integrationRoot });
    } catch (error) {
      proof.error = errorMessage(error);
      errors.push(error);
    }
    try {
      proof.pathAbsent = await owner.deps.pathIdentity(resource.path) === null;
      const registry = await readWorktreeRegistry(owner.input, owner.deps, checked);
      proof.registryAbsent = !registry.some((entry) => entry.path === resource.path);
      proof.registryStorageAbsent = await owner.deps.pathIdentity(
        resource.registryIdentity.adminPath,
      ) === null;
    } catch (error) {
      proof.error ??= errorMessage(error);
      errors.push(error);
    }
    if (!proof.pathAbsent || !proof.registryAbsent || !proof.registryStorageAbsent) {
      const message = `${resource.label} path/registry cleanup proof is incomplete`;
      proof.error ??= message;
      errors.push(new Error(message));
    }
    worktrees.push(proof);
  }

  const worktreeCleanupComplete = worktrees.every((entry) => (
    entry.pathAbsent && entry.registryAbsent && entry.registryStorageAbsent
  ));
  const taskRoot = await cleanupTaskRoot(
    owner,
    errors,
    worktreeCleanupComplete && owner.foreignRegistries.length === 0,
  );
  const foreign = await verifyForeignResources(owner, checked, errors);
  return {
    nonce: owner.ledger.probeNonce,
    worktrees: worktrees.reverse(),
    taskRoot,
    foreign,
    errors: errors.map(errorMessage),
  };
}

async function createClaim(owner, label, path) {
  const claimPath = join(owner.taskRoot, `.skiff-platform-source-${label.toLowerCase()}.claim`);
  const contents = `${JSON.stringify({
    schemaVersion: OWNER_SCHEMA,
    nonce: owner.ledger.probeNonce,
    label,
    path,
    candidate: owner.input.candidate,
  })}\n`;
  const identity = await owner.deps.writeExclusiveDurable(claimPath, contents);
  const claim = {
    label,
    path: claimPath,
    contents,
    identity,
    digest: probeDigest(contents),
  };
  owner.claims.push(claim);
  return claim;
}

async function verifyTaskRoot(owner) {
  if (!samePathIdentity(await owner.deps.pathIdentity(owner.taskRoot), owner.taskRootIdentity)) {
    return false;
  }
  if (!samePathIdentity(await owner.deps.pathIdentity(owner.markerPath), owner.markerIdentity)) {
    return false;
  }
  return await owner.deps.readOwnershipText(owner.markerPath) === owner.marker;
}

async function verifyClaim(owner, claim) {
  if (!await verifyTaskRoot(owner)) return false;
  if (!samePathIdentity(await owner.deps.pathIdentity(claim.path), claim.identity)) return false;
  return await owner.deps.readOwnershipText(claim.path) === claim.contents;
}

async function captureRegistryIdentity(owner, path, entry) {
  const dotGitPath = join(path, '.git');
  const dotGitIdentity = await owner.deps.pathIdentity(dotGitPath);
  if (dotGitIdentity?.kind !== 'file') return null;
  const contents = await owner.deps.readOwnershipText(dotGitPath);
  const match = /^gitdir: (.+)\r?\n?$/.exec(contents);
  if (match === null || !isAbsolute(match[1])) return null;
  const adminPath = match[1];
  const adminIdentity = await owner.deps.pathIdentity(adminPath);
  if (adminIdentity?.kind !== 'directory') return null;
  if (await owner.deps.canonicalPath(adminPath) !== adminPath) return null;
  return {
    entryIdentity: entry.identity,
    dotGitPath,
    dotGitIdentity,
    dotGitDigest: probeDigest(contents),
    adminPath,
    adminIdentity,
  };
}

function sameRegistryIdentity(left, right) {
  return left !== null
    && right !== null
    && left.entryIdentity === right.entryIdentity
    && left.dotGitPath === right.dotGitPath
    && samePathIdentity(left.dotGitIdentity, right.dotGitIdentity)
    && left.dotGitDigest === right.dotGitDigest
    && left.adminPath === right.adminPath
    && samePathIdentity(left.adminIdentity, right.adminIdentity);
}

async function cleanupTaskRoot(owner, errors, allowRemoval) {
  const proof = {
    path: owner.taskRoot,
    pathIdentity: owner.taskRootIdentity,
    markerPath: owner.markerPath,
    markerIdentity: owner.markerIdentity,
    markerVerifiedBeforeRemoval: false,
    retainedForOwnership: false,
    absent: false,
    error: null,
  };
  try {
    proof.markerVerifiedBeforeRemoval = await verifyTaskRoot(owner);
    if (!proof.markerVerifiedBeforeRemoval) {
      rememberForeign(
        owner,
        owner.taskRoot,
        await owner.deps.pathIdentity(owner.taskRoot),
        null,
        'task-root-replacement',
      );
      throw new Error('task root marker or inode changed before cleanup');
    }
    if (!allowRemoval) {
      proof.retainedForOwnership = true;
      throw new Error('task root retained because worktree ownership cleanup is incomplete');
    }
    await owner.deps.removeOwnedTree(owner.taskRoot);
  } catch (error) {
    proof.error = errorMessage(error);
    errors.push(error);
  }
  proof.absent = await owner.deps.pathIdentity(owner.taskRoot) === null;
  if (!proof.absent && proof.error === null) {
    proof.error = 'task root cleanup proof is not ABSENT';
    errors.push(new Error(proof.error));
  }
  return proof;
}

function rememberForeign(owner, path, pathIdentity, registryEntry, reason) {
  if (pathIdentity !== null && !owner.foreignPaths.some((entry) => entry.path === path)) {
    owner.foreignPaths.push({ path, identity: pathIdentity, reason });
  }
  if (registryEntry !== null && !owner.foreignRegistries.some((entry) => entry.path === path)) {
    owner.foreignRegistries.push({
      path,
      identity: registryEntry.identity,
      reason,
    });
  }
}

async function verifyForeignResources(owner, checked, errors) {
  let registry = [];
  try {
    registry = await readWorktreeRegistry(owner.input, owner.deps, checked);
  } catch (error) {
    errors.push(error);
  }
  const paths = [];
  for (const entry of owner.foreignPaths) {
    const preserved = samePathIdentity(await owner.deps.pathIdentity(entry.path), entry.identity);
    paths.push({ ...entry, preserved });
    if (!preserved) errors.push(new Error(`foreign path changed during probe: ${entry.path}`));
  }
  const registries = owner.foreignRegistries.map((entry) => {
    const preserved = registry.some((current) => (
      current.path === entry.path && current.identity === entry.identity
    ));
    if (!preserved) errors.push(new Error(`foreign registry changed during probe: ${entry.path}`));
    return { ...entry, preserved };
  });
  return {
    paths,
    registries,
    preserved: paths.every((entry) => entry.preserved)
      && registries.every((entry) => entry.preserved),
  };
}
