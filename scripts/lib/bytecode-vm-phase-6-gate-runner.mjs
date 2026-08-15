import { lstat, mkdir, realpath, rm, rmdir } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

import { captureOwnedCommand } from './owned-command.mjs';
import { assertBytecodeVmGateEnvironment } from './bytecode-vm-gate-environment.mjs';
import {
  assertGitObject,
  assertPhase6LaneCoverage,
  phase6CandidateSpecs,
  phase6WorkloadSpecs,
  snapshotCommandEnvironment,
} from './bytecode-vm-phase-6-contract.mjs';
import { createPhase6EvidenceRoot } from './bytecode-vm-phase-6-evidence-root.mjs';
import { assertNoUnsafeHttpBypassEnvironment } from './http_live_process.mjs';
import {
  PHASE5_CARRIER_ENV,
  PHASE5_RUNTIME_BIN_ENV,
} from './bytecode-vm-phase-5-gate-runner.mjs';
import {
  checkPhase6Evidence,
  finalizePhase6Evidence,
} from './bytecode-vm-phase-6-evidence.mjs';
import { writePhase6CommandReceipt } from './bytecode-vm-phase-6-receipts.mjs';

const OUTPUT_ENV = 'SKIFF_BYTECODE_VM_PHASE6_EVIDENCE_DIR';
const COMMIT_ENV = 'SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT';
const TREE_ENV = 'SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_TREE';
export const PHASE6_CARRIER_ENV = 'SKIFF_BYTECODE_VM_PHASE6_CARRIER_ROOT';
export const PHASE6_RUNTIME_BIN_ENV = 'SKIFF_BYTECODE_VM_PHASE6_RUNTIME_BIN';
export const PHASE6_CARGO_LEASE_DIR = '/tmp/skiff-bcvm-p6-r1-cargo.lockdir';
export const PHASE6_CARGO_TARGET_DIR = '/Users/geek/workspace/.skiff-cargo-target';

export function parsePhase6GateArgs(args, { env = process.env } = {}) {
  const fields = new Map([
    ['--output-dir', 'outputDir'],
    ['--candidate', 'expectedCommit'],
    ['--tree', 'expectedTree'],
  ]);
  const values = new Map();
  let help = false;
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === '--help' || argument === '-h') {
      help = true;
      continue;
    }
    const key = fields.get(argument);
    if (key === undefined) throw new Error(`unknown option ${argument}`);
    if (values.has(key)) throw new Error(`${argument} was provided more than once`);
    const value = args[index + 1];
    if (value === undefined || value.startsWith('--')) throw new Error(`${argument} requires a value`);
    values.set(key, value);
    index += 1;
  }
  if (help) return { help: true };
  return {
    help: false,
    outputDir: values.get('outputDir') ?? env[OUTPUT_ENV],
    expectedCommit: values.get('expectedCommit') ?? env[COMMIT_ENV],
    expectedTree: values.get('expectedTree') ?? env[TREE_ENV],
  };
}

export async function runPhase6Gate(options, {
  repoRoot,
  capture = captureOwnedCommand,
  signalTarget = process,
  now = () => new Date().toISOString(),
  env = process.env,
  acquireCargoLease = acquirePhase6CargoLease,
} = {}) {
  assertNoUnsafeHttpBypassEnvironment(env);
  assertBytecodeVmGateEnvironment(env);
  const input = await validateInput(options, repoRoot);
  const evidenceRoot = await createPhase6EvidenceRoot(input.outputDir);
  const candidateSpecs = phase6CandidateSpecs(input.repoRoot);
  const workloadSpecs = phase6WorkloadSpecs(input.repoRoot);
  assertPhase6LaneCoverage(workloadSpecs);
  const childEnvironment = {
    ...env,
    CARGO_TARGET_DIR: PHASE6_CARGO_TARGET_DIR,
    [PHASE6_CARRIER_ENV]: input.carrierRoot,
    [PHASE6_RUNTIME_BIN_ENV]: join(PHASE6_CARGO_TARGET_DIR, 'debug', 'runtime'),
    [PHASE5_CARRIER_ENV]: input.carrierRoot,
    [PHASE5_RUNTIME_BIN_ENV]: join(PHASE6_CARGO_TARGET_DIR, 'debug', 'runtime'),
  };
  const commandEnvironments = new Map(
    [...candidateSpecs, ...workloadSpecs]
      .map((spec) => [spec.id, snapshotCommandEnvironment(childEnvironment)]),
  );
  const abortController = new AbortController();
  let interruptedBy = null;
  const handlers = new Map(['SIGINT', 'SIGTERM'].map((signal) => [signal, () => {
    interruptedBy ??= signal;
    abortController.abort(new Error(`Phase 6 r1 Gate interrupted by ${signal}`));
  }]));
  for (const [signal, handler] of handlers) signalTarget.on(signal, handler);
  const startedAt = now();
  const outcomes = new Map();
  let releaseCargoLease = null;
  try {
    for (const spec of candidateSpecs.slice(0, 3)) outcomes.set(spec.id, await execute(spec));
    if (preflightMatches(outcomes, input) && interruptedBy === null) {
      releaseCargoLease = await acquireCargoLease(PHASE6_CARGO_LEASE_DIR);
      for (const spec of workloadSpecs) {
        if (interruptedBy !== null) break;
        outcomes.set(spec.id, await execute(spec));
      }
      for (const spec of candidateSpecs.slice(3)) {
        if (interruptedBy !== null) break;
        outcomes.set(spec.id, await execute(spec));
      }
    }
  } finally {
    if (releaseCargoLease !== null) await releaseCargoLease();
    for (const [signal, handler] of handlers) signalTarget.off(signal, handler);
  }
  try {
    const manifest = await finalizePhase6Evidence({
      evidenceRoot,
      repoRoot: input.repoRoot,
      expectedCommit: input.expectedCommit,
      expectedTree: input.expectedTree,
      commandEnvironments,
      startedAt,
      finishedAt: now(),
    });
    let checkerError = null;
    try {
      await checkPhase6Evidence(input.outputDir, {
        ...input,
        directoryIdentities: evidenceRoot.identities(),
        commandEnvironments,
      });
      await evidenceRoot.assertAll();
    } catch (error) {
      checkerError = error instanceof Error ? error.message : String(error);
    }
    return { manifest, checkerError, outputDir: input.outputDir };
  } finally {
    await rm(input.carrierRoot, { recursive: true, force: true });
  }

  async function execute(spec) {
    await evidenceRoot.assertAll();
    const commandStartedAt = now();
    const actualEnv = commandEnvironments.get(spec.id);
    const outcome = await capture(spec.command, [...spec.args], {
      cwd: spec.cwd,
      env: actualEnv,
      signal: abortController.signal,
    });
    await evidenceRoot.assertAll();
    await writePhase6CommandReceipt(evidenceRoot, spec, actualEnv, outcome, {
      stdout: outcome.stdout,
      stderr: outcome.stderr,
      startedAt: commandStartedAt,
      finishedAt: now(),
      interruptedBy,
    });
    return outcome;
  }
}

export async function acquirePhase6CargoLease(
  leaseDir = PHASE6_CARGO_LEASE_DIR,
  { makeDirectory = mkdir, removeDirectory = rmdir } = {},
) {
  try {
    await makeDirectory(leaseDir);
  } catch (error) {
    if (error?.code === 'EEXIST') {
      throw new Error(`Phase 6 r1 Cargo lease is already held: ${leaseDir}`);
    }
    throw error;
  }
  let released = false;
  return async () => {
    if (released) return;
    released = true;
    await removeDirectory(leaseDir);
  };
}

export function phase6GateUsage() {
  return `usage: node scripts/run-bytecode-vm-phase-6-gate.mjs \\
  --output-dir <absolute-new-directory> \\
  --candidate <40-hex-commit> \\
  --tree <40-hex-tree>\n\nThe output directory must be absent, canonical, and outside the candidate repository.`;
}

async function validateInput(options, repoRoot) {
  const exactRoot = await realpath(repoRoot);
  if (exactRoot !== repoRoot) throw new Error('Gate repository root must be canonical');
  const outputDir = options?.outputDir;
  if (typeof outputDir !== 'string' || !isAbsolute(outputDir) || resolve(outputDir) !== outputDir) {
    throw new Error('--output-dir must be a canonical absolute path');
  }
  if (contains(repoRoot, outputDir) || contains(outputDir, repoRoot)) {
    throw new Error('--output-dir must not overlap the candidate repository');
  }
  try {
    await lstat(outputDir);
    throw new Error('--output-dir must not already exist');
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
  }
  const parent = dirname(outputDir);
  if (join(await realpath(parent), outputDir.slice(parent.length + 1)) !== outputDir) {
    throw new Error('--output-dir parent must not use a symlink');
  }
  const carrierRoot = `${outputDir}.carrier`;
  if (contains(repoRoot, carrierRoot) || contains(carrierRoot, repoRoot)) {
    throw new Error('Phase 6 carrier root must not overlap the candidate repository');
  }
  try {
    await lstat(carrierRoot);
    throw new Error('Phase 6 carrier root must not already exist');
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
  }
  return {
    repoRoot,
    outputDir,
    carrierRoot,
    expectedCommit: assertGitObject(options?.expectedCommit, '--candidate'),
    expectedTree: assertGitObject(options?.expectedTree, '--tree'),
  };
}

function contains(parent, child) {
  const value = relative(parent, child);
  return value === '' || (value !== '..' && !value.startsWith(`..${sep}`) && !isAbsolute(value));
}

function preflightMatches(outcomes, input) {
  return successfulText(outcomes.get('preflight-head'))?.trim() === input.expectedCommit
    && successfulText(outcomes.get('preflight-tree'))?.trim() === input.expectedTree
    && successfulText(outcomes.get('preflight-status')) === '';
}

function successfulText(outcome) {
  return outcome?.code === 0 && outcome?.signal === null && outcome?.error == null
    ? outcome.stdout
    : null;
}
