import { basename, dirname, isAbsolute, join, relative, sep } from 'node:path';

import { assertCombinedLedger } from './platform-source-probe-contract.mjs';
import { assertProbeWorktreesUnregistered } from './platform-source-probe-ownership.mjs';

const GIB = 1024 ** 3;

export async function preflightPlatformSourceProbe(input, deps, checked) {
  await deps.assertExecutables(
    ['git', 'node', 'cargo', 'strings', 'rg'],
    process.env,
    input.integrationRoot,
  );
  const canonical = await deps.canonicalPath(input.integrationRoot);
  if (canonical !== input.integrationRoot) {
    throw new Error('integration root must be an exact canonical absolute path');
  }
  if (await deps.exists(input.aWorktree) || await deps.exists(input.bWorktree)) {
    throw new Error('A/B worktree paths must be absent before the probe');
  }
  if (input.mode === 'combined' && await deps.exists(input.ledger)) {
    throw new Error('combined ledger path must be absent before the probe');
  }
  if (input.mode === 'full' && !await deps.exists(input.combinedLedger)) {
    throw new Error('full mode requires an existing combined ledger');
  }
  for (const path of [
    input.aWorktree,
    input.bWorktree,
    input.mode === 'combined' ? input.ledger : input.combinedLedger,
  ]) {
    const parent = dirname(path);
    if (join(await deps.canonicalPath(parent), basename(path)) !== path) {
      throw new Error(`probe path parent is not canonical: ${path}`);
    }
  }
  if (
    input.mode === 'full'
    && await deps.canonicalPath(input.combinedLedger) !== input.combinedLedger
  ) {
    throw new Error('combined ledger path must not be a symlink');
  }

  const head = (await checked(deps, 'git', [
    '-C', input.integrationRoot, 'rev-parse', 'HEAD',
  ])).stdout.trim();
  const tree = (await checked(deps, 'git', [
    '-C', input.integrationRoot, 'rev-parse', 'HEAD^{tree}',
  ])).stdout.trim();
  const lock = (await checked(deps, 'git', [
    '-C', input.integrationRoot, 'rev-parse', 'HEAD:Cargo.lock',
  ])).stdout.trim();
  if (head !== input.candidate || tree !== input.expectedTree || lock !== input.expectedLockBlob) {
    throw new Error('candidate commit/tree/Cargo.lock blob does not match the requested probe');
  }
  const status = (await checked(deps, 'git', [
    '-C', input.integrationRoot, 'status', '--porcelain', '--untracked-files=all',
  ])).stdout.trim();
  if (!statusIsClean(status, input)) {
    throw new Error(`integration candidate is not clean: ${status}`);
  }
  await assertProbeWorktreesUnregistered(input, deps, checked);

  let combinedLedger = null;
  if (input.mode === 'full') {
    combinedLedger = await deps.readLedger(input.combinedLedger);
    assertCombinedLedger(combinedLedger, input);
  }
  const existingTarget = join(input.integrationRoot, 'build', 'cargo-target');
  const allocated = await deps.allocatedBytes(existingTarget);
  const requiredBytes = allocated === undefined ? 8 * GIB : allocated + (2 * GIB);
  const availableBytes = await deps.availableBytes(dirname(input.integrationRoot));
  if (availableBytes < requiredBytes) {
    throw new Error(`insufficient capacity: need ${requiredBytes}, have ${availableBytes}`);
  }
  return {
    combinedLedger,
    capacity: {
      existingTarget,
      existingAllocatedBytes: allocated ?? null,
      requiredBytes,
      availableBytes,
    },
  };
}

function statusIsClean(status, input) {
  if (status === '') return true;
  if (input.mode !== 'full') return false;
  const fromRoot = relative(input.integrationRoot, input.combinedLedger);
  if (fromRoot === '..' || fromRoot.startsWith(`..${sep}`) || isAbsolute(fromRoot)) return false;
  return status.split(/\r?\n/).every((line) => line === `?? ${fromRoot}`);
}
