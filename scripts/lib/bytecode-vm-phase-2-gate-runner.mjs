import { lstat, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

import { captureOwnedCommand } from './owned-command.mjs';
import { assertBytecodeVmGateEnvironment } from './bytecode-vm-gate-environment.mjs';
import {
  assertGitObject,
  assertPhase2LaneCoverage,
  phase2CandidateSpecs,
  phase2WorkloadSpecs,
  snapshotCommandEnvironment,
} from './bytecode-vm-phase-2-contract.mjs';
import { createPhase2EvidenceRoot } from './bytecode-vm-phase-2-evidence-root.mjs';
import {
  checkPhase2Evidence,
  finalizePhase2Evidence,
} from './bytecode-vm-phase-2-evidence.mjs';
import { writePhase2CommandReceipt } from './bytecode-vm-phase-2-receipts.mjs';

const OUTPUT_ENV = 'SKIFF_BYTECODE_VM_PHASE2_EVIDENCE_DIR';
const COMMIT_ENV = 'SKIFF_BYTECODE_VM_PHASE2_CANDIDATE_COMMIT';
const TREE_ENV = 'SKIFF_BYTECODE_VM_PHASE2_CANDIDATE_TREE';

export function parsePhase2GateArgs(args, { env = process.env } = {}) {
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

export async function runPhase2Gate(options, {
  repoRoot,
  capture = captureOwnedCommand,
  signalTarget = process,
  now = () => new Date().toISOString(),
  env = process.env,
} = {}) {
  assertBytecodeVmGateEnvironment(env);
  const input = await validateInput(options, repoRoot);
  const evidenceRoot = await createPhase2EvidenceRoot(input.outputDir);
  const candidateSpecs = phase2CandidateSpecs(input.repoRoot);
  const workloadSpecs = phase2WorkloadSpecs(input.repoRoot);
  assertPhase2LaneCoverage(workloadSpecs);
  const commandEnvironments = new Map(
    [...candidateSpecs, ...workloadSpecs]
      .map((spec) => [spec.id, snapshotCommandEnvironment(env)]),
  );
  const abortController = new AbortController();
  let interruptedBy = null;
  const handlers = new Map(['SIGINT', 'SIGTERM'].map((signal) => [signal, () => {
    interruptedBy ??= signal;
    abortController.abort(new Error(`Phase 2 Gate interrupted by ${signal}`));
  }]));
  for (const [signal, handler] of handlers) signalTarget.on(signal, handler);
  const startedAt = now();
  const outcomes = new Map();
  try {
    for (const spec of candidateSpecs.slice(0, 3)) outcomes.set(spec.id, await execute(spec));
    if (preflightMatches(outcomes, input) && interruptedBy === null) {
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
    for (const [signal, handler] of handlers) signalTarget.off(signal, handler);
  }
  const manifest = await finalizePhase2Evidence({
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
    await checkPhase2Evidence(input.outputDir, {
      ...input,
      directoryIdentities: evidenceRoot.identities(),
      commandEnvironments,
    });
    await evidenceRoot.assertAll();
  } catch (error) {
    checkerError = error instanceof Error ? error.message : String(error);
  }
  return { manifest, checkerError, outputDir: input.outputDir };

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
    await writePhase2CommandReceipt(evidenceRoot, spec, actualEnv, outcome, {
      stdout: outcome.stdout,
      stderr: outcome.stderr,
      startedAt: commandStartedAt,
      finishedAt: now(),
      interruptedBy,
    });
    return outcome;
  }
}

export function phase2GateUsage() {
  return `usage: node scripts/run-bytecode-vm-phase-2-gate.mjs \\
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
  return {
    repoRoot,
    outputDir,
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
