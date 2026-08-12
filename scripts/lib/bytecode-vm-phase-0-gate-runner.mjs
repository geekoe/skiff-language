import { lstat, mkdir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

import { captureOwnedCommand } from './owned-command.mjs';
import {
  assertGitObject,
  phase0CandidateSpecs,
  phase0FreshCandidateSpecs,
  phase0WorkloadSpecs,
} from './bytecode-vm-phase-0-contract.mjs';
import {
  checkPhase0Evidence,
  finalizePhase0Evidence,
} from './bytecode-vm-phase-0-evidence.mjs';
import { writePhase0CommandReceipt } from './bytecode-vm-phase-0-receipts.mjs';

const OUTPUT_ENV = 'SKIFF_BYTECODE_VM_PHASE0_EVIDENCE_DIR';
const COMMIT_ENV = 'SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_COMMIT';
const TREE_ENV = 'SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_TREE';

export function parsePhase0GateArgs(args, { env = process.env } = {}) {
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

export async function runPhase0Gate(options, {
  repoRoot,
  capture = captureOwnedCommand,
  signalTarget = process,
  now = () => new Date().toISOString(),
} = {}) {
  const input = await validateInput(options, repoRoot);
  await mkdir(input.outputDir, { mode: 0o700 });
  const transcriptDir = join(input.outputDir, 'transcripts');
  await mkdir(transcriptDir, { mode: 0o700 });
  const transcriptPaths = {
    success: join(transcriptDir, 'success.jsonl'),
    negative: join(transcriptDir, 'negative.jsonl'),
  };
  const candidateSpecs = phase0CandidateSpecs(input.repoRoot);
  const workloadSpecs = phase0WorkloadSpecs(input.repoRoot, transcriptPaths);
  const abortController = new AbortController();
  let interruptedBy = null;
  const handlers = new Map(['SIGINT', 'SIGTERM'].map((signal) => [signal, () => {
    interruptedBy ??= signal;
    abortController.abort(new Error(`Phase 0 Gate interrupted by ${signal}`));
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
      for (const spec of candidateSpecs.slice(3, 9)) {
        if (interruptedBy !== null) break;
        outcomes.set(spec.id, await execute(spec));
      }
    }
  } finally {
    for (const [signal, handler] of handlers) signalTarget.off(signal, handler);
  }
  const manifest = await finalizePhase0Evidence({
    outputDir: input.outputDir,
    repoRoot: input.repoRoot,
    expectedCommit: input.expectedCommit,
    expectedTree: input.expectedTree,
    transcriptPaths,
    startedAt,
    finishedAt: now(),
  });
  let checkerError = null;
  try {
    await checkPhase0Evidence(input.outputDir, { ...input, transcriptPaths });
    const fresh = await captureFreshCandidate(input.repoRoot, capture);
    if (fresh.commit !== input.expectedCommit
      || fresh.tree !== input.expectedTree
      || fresh.status !== '') {
      throw new Error('fresh live candidate is dirty or stale after evidence checking');
    }
  } catch (error) {
    checkerError = error instanceof Error ? error.message : String(error);
  }
  return { manifest, checkerError, outputDir: input.outputDir };

  async function execute(spec) {
    const commandStartedAt = now();
    const outcome = await capture(spec.command, [...spec.args], {
      cwd: spec.cwd,
      env: { ...process.env, ...spec.evidenceEnv },
      signal: abortController.signal,
    });
    await writePhase0CommandReceipt(input.outputDir, spec, outcome, {
      stdout: outcome.stdout,
      stderr: outcome.stderr,
      startedAt: commandStartedAt,
      finishedAt: now(),
      interruptedBy,
    });
    return outcome;
  }
}

export function phase0GateUsage() {
  return `usage: node scripts/run-bytecode-vm-phase-0-gate.mjs \\
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

async function captureFreshCandidate(repoRoot, capture) {
  const values = [];
  for (const spec of phase0FreshCandidateSpecs(repoRoot)) {
    const outcome = await capture(spec.command, [...spec.args], { cwd: repoRoot, env: process.env });
    values.push(outcome.code === 0 && outcome.signal === null && outcome.error == null
      ? outcome.stdout
      : null);
  }
  return {
    commit: values[0]?.trim() ?? null,
    tree: values[1]?.trim() ?? null,
    status: values[2] ?? null,
  };
}
