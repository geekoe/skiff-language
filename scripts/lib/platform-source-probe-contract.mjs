import { isAbsolute, relative, resolve, sep } from 'node:path';

import {
  PROBE_TARGETED_CRATES,
  combinedArtifactEvidenceIsComplete,
} from './platform-source-probe-evidence.mjs';
import { probeDigest } from './platform-source-probe-support.mjs';

export const PROBE_LEDGER_SCHEMA = 'skiff-platform-source-shared-target-probe-v5';
export { PROBE_TARGETED_CRATES };

export function validateProbeOptions(options) {
  const mode = options?.mode;
  if (mode !== 'combined' && mode !== 'full') {
    throw new Error('--mode must be exactly combined or full');
  }
  const input = {
    mode,
    integrationRoot: absoluteOption(options.integrationRoot, '--integration-root'),
    candidate: hashOption(options.candidate, '--candidate'),
    expectedTree: hashOption(options.expectedTree, '--expected-tree'),
    expectedLockBlob: hashOption(options.expectedLockBlob, '--expected-lock-blob'),
    expectedPreludeIdentity: textOption(
      options.expectedPreludeIdentity,
      '--expected-prelude-identity',
    ),
    expectedStdPackageBuildId: textOption(
      options.expectedStdPackageBuildId,
      '--expected-std-package-build-id',
    ),
    aWorktree: absoluteOption(options.aWorktree, '--a-worktree'),
    bWorktree: absoluteOption(options.bWorktree, '--b-worktree'),
    json: options.json === true,
  };
  if (!input.json) throw new Error('--json is required');
  if (input.aWorktree === input.bWorktree) throw new Error('A/B worktrees must be distinct');
  if (
    pathsOverlap(input.integrationRoot, input.aWorktree)
    || pathsOverlap(input.integrationRoot, input.bWorktree)
    || pathsOverlap(input.aWorktree, input.bWorktree)
  ) {
    throw new Error('integration and A/B worktree paths must not overlap');
  }
  if (mode === 'combined') {
    input.ledger = absoluteOption(options.ledger, '--ledger');
    if (pathsOverlap(input.ledger, input.aWorktree) || pathsOverlap(input.ledger, input.bWorktree)) {
      throw new Error('combined ledger must not overlap an A/B worktree');
    }
    if (options.combinedLedger !== undefined) {
      throw new Error('--combined-ledger is full-mode only');
    }
  } else {
    input.combinedLedger = absoluteOption(options.combinedLedger, '--combined-ledger');
    if (
      pathsOverlap(input.combinedLedger, input.aWorktree)
      || pathsOverlap(input.combinedLedger, input.bWorktree)
    ) {
      throw new Error('combined ledger must not overlap an A/B worktree');
    }
    if (options.ledger !== undefined) throw new Error('--ledger is combined-mode only');
  }
  return Object.freeze(input);
}

function pathsOverlap(left, right) {
  const fromLeft = relative(left, right);
  const fromRight = relative(right, left);
  const contained = (value) => value === ''
    || (value !== '..' && !value.startsWith(`..${sep}`) && !isAbsolute(value));
  return contained(fromLeft) || contained(fromRight);
}

export function createProbeLedger(input, probeNonce) {
  if (!/^[a-f0-9]{32}$/.test(probeNonce)) {
    throw new Error('probe nonce must be 128 bits of lowercase hexadecimal');
  }
  return {
    schemaVersion: PROBE_LEDGER_SCHEMA,
    mode: input.mode,
    probeNonce,
    status: 'RUNNING',
    candidate: input.candidate,
    tree: input.expectedTree,
    lockBlob: input.expectedLockBlob,
    expectedPreludeIdentity: input.expectedPreludeIdentity,
    expectedStdPackageBuildId: input.expectedStdPackageBuildId,
    targetedCleanCrates: [...PROBE_TARGETED_CRATES],
    capacity: null,
    combinedLedger: null,
    paths: null,
    rounds: [],
    identityProbes: [],
    artifactEvidence: [],
    artifacts: [],
    structure: null,
    registry: null,
    sourceSuite: null,
    fullProbeRuns: 0,
    hostAttempt: null,
    output: null,
    processes: [],
    ports: [],
    primary: null,
    cleanup: null,
    ownership: null,
    firstError: null,
  };
}

export function assertCombinedLedger(ledger, input) {
  if (
    ledger?.schemaVersion !== PROBE_LEDGER_SCHEMA
    || ledger.mode !== 'combined'
    || ledger.status !== 'PASS'
  ) {
    throw new Error('combined ledger is not a PASS ledger for this harness schema');
  }
  const { ledgerDigest, ...body } = ledger;
  if (ledgerDigest !== probeDigest(body)) {
    throw new Error('combined ledger digest mismatch');
  }
  if (
    ledger.candidate !== input.candidate
    || ledger.tree !== input.expectedTree
    || ledger.lockBlob !== input.expectedLockBlob
    || ledger.expectedPreludeIdentity !== input.expectedPreludeIdentity
    || ledger.expectedStdPackageBuildId !== input.expectedStdPackageBuildId
    || ledger.fullProbeRuns !== 0
  ) {
    throw new Error('combined ledger does not match the full-mode candidate and goldens');
  }
  if (
    ledger.paths?.integrationRoot !== input.integrationRoot
    || ledger.output?.combinedLedger !== input.combinedLedger
    || ledger.output?.atomicWrite !== 'PASS'
    || ledger.output?.method !== 'wx+flush+close+hard-link'
    || ledger.output?.temporaryPath !== `${input.combinedLedger}.${ledger.probeNonce}.tmp`
    || ledger.output?.ownedTemporaryAbsent !== true
    || ledger.output?.foreignDestinationPreserved !== true
    || ledger.primary?.status !== 'PASS'
    || ledger.firstError !== null
    || JSON.stringify(ledger.targetedCleanCrates) !== JSON.stringify(PROBE_TARGETED_CRATES)
    || JSON.stringify(ledger.rounds?.map((round) => round.label))
      !== JSON.stringify(['A-origin', 'B-origin', 'final-A-origin'])
    || JSON.stringify(ledger.registry) !== JSON.stringify([{ id: 'std', root: 'std' }])
    || ledger.sourceSuite !== null
    || ledger.hostAttempt !== null
  ) {
    throw new Error('combined ledger matrix metadata is incomplete');
  }
  const worktreeOwnership = ledger.ownership?.worktrees;
  if (
    !/^[a-f0-9]{32}$/.test(ledger.probeNonce)
    || ledger.ownership?.nonce !== ledger.probeNonce
    || !Array.isArray(worktreeOwnership)
    || worktreeOwnership.length !== 2
    || JSON.stringify(worktreeOwnership.map((entry) => entry.label))
      !== JSON.stringify(['A', 'B'])
    || JSON.stringify(worktreeOwnership.map((entry) => entry.path))
      !== JSON.stringify([ledger.paths?.aWorktree, ledger.paths?.bWorktree])
    || worktreeOwnership.some((entry) => (
      entry.claimVerifiedBeforeRemoval !== true
      || entry.pathAbsent !== true
      || entry.registryAbsent !== true
      || entry.registryStorageAbsent !== true
      || entry.error !== null
      || typeof entry.registryIdentity?.entryIdentity !== 'string'
      || entry.registryIdentity?.adminIdentity?.kind !== 'directory'
      || !/^[a-f0-9]{64}$/.test(entry.claimDigest)
    ))
    || ledger.ownership?.taskRoot?.path !== ledger.paths?.taskRoot
    || ledger.ownership?.taskRoot?.markerVerifiedBeforeRemoval !== true
    || ledger.ownership?.taskRoot?.retainedForOwnership !== false
    || ledger.ownership?.taskRoot?.absent !== true
    || ledger.ownership?.foreign?.preserved !== true
    || ledger.ownership?.errors?.length !== 0
  ) {
    throw new Error('combined ledger resource ownership proof is incomplete');
  }
  if (
    !Array.isArray(ledger.identityProbes)
    || ledger.identityProbes.length !== 4
    || ledger.identityProbes.some((probe) => (
      probe.preludeIdentity !== input.expectedPreludeIdentity
      || probe.stdPackageBuildId !== input.expectedStdPackageBuildId
    ))
    || JSON.stringify(ledger.identityProbes.map((probe) => [
      probe.manifestRoot,
      probe.platformRoot,
    ])) !== JSON.stringify([
      [ledger.paths?.aWorktree, ledger.paths?.aWorktree],
      [ledger.paths?.bWorktree, ledger.paths?.bWorktree],
      [ledger.paths?.bWorktree, ledger.paths?.bWorktree],
      [ledger.paths?.aWorktree, ledger.paths?.aWorktree],
    ])
    || !combinedArtifactEvidenceIsComplete(ledger.artifactEvidence)
    || !Array.isArray(ledger.artifacts)
    || ledger.artifacts.length === 0
    || !Array.isArray(ledger.structure?.stringsNoMatch)
    || ledger.structure.stringsNoMatch.length === 0
    || !Array.isArray(ledger.structure?.depInfoNoMatch)
    || ledger.structure.depInfoNoMatch.length === 0
  ) {
    throw new Error('combined ledger identity, Fresh, or structure evidence is incomplete');
  }
  if (
    !ledger.cleanup?.aWorktreeAbsent
    || !ledger.cleanup?.bWorktreeAbsent
    || !ledger.cleanup?.taskRootAbsent
    || !ledger.cleanup?.processGroupsAbsent
    || !ledger.cleanup?.portsAbsent
    || ledger.cleanup?.errors?.length !== 0
    || !Array.isArray(ledger.processes)
    || ledger.processes.some((entry) => entry.absent !== true)
    || !Array.isArray(ledger.ports)
    || ledger.ports.some((entry) => entry.absent !== true)
  ) {
    throw new Error('combined ledger cleanup proof is incomplete');
  }
}

export function parseProbeArgs(rawArgs) {
  const values = new Map();
  let json = false;
  const names = new Map([
    ['--mode', 'mode'],
    ['--integration-root', 'integrationRoot'],
    ['--candidate', 'candidate'],
    ['--expected-tree', 'expectedTree'],
    ['--expected-lock-blob', 'expectedLockBlob'],
    ['--expected-prelude-identity', 'expectedPreludeIdentity'],
    ['--expected-std-package-build-id', 'expectedStdPackageBuildId'],
    ['--a-worktree', 'aWorktree'],
    ['--b-worktree', 'bWorktree'],
    ['--ledger', 'ledger'],
    ['--combined-ledger', 'combinedLedger'],
  ]);
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--json') {
      if (json) throw new Error('--json was provided more than once');
      json = true;
      continue;
    }
    const key = names.get(arg);
    if (key === undefined) throw new Error(`unknown option ${arg}`);
    if (values.has(key)) throw new Error(`${arg} was provided more than once`);
    const value = rawArgs[index + 1];
    if (value === undefined || value.startsWith('--')) {
      throw new Error(`${arg} requires a value`);
    }
    values.set(key, value);
    index += 1;
  }
  return validateProbeOptions({ ...Object.fromEntries(values), json });
}

function absoluteOption(value, name) {
  if (typeof value !== 'string' || !isAbsolute(value)) {
    throw new Error(`${name} requires an absolute path`);
  }
  return resolve(value);
}

function textOption(value, name) {
  if (typeof value !== 'string' || value.trim() !== value || value.length === 0) {
    throw new Error(`${name} requires a non-empty exact value`);
  }
  return value;
}

function hashOption(value, name) {
  const text = textOption(value, name);
  if (!/^[a-f0-9]{40}$/.test(text)) {
    throw new Error(`${name} must be a 40-character Git object id`);
  }
  return text;
}
