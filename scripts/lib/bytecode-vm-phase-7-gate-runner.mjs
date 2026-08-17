import {
  access,
  lstat,
  mkdir,
  realpath,
  rm,
  rmdir,
} from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { captureOwnedCommand } from './owned-command.mjs';
import { assertBytecodeVmGateEnvironment } from './bytecode-vm-gate-environment.mjs';
import {
  assertGitObject,
  assertPhase7Catalog,
  phase7CandidateSpecs,
  phase7ExecutionOrder,
  phase7SpecCatalogDigest,
  phase7WorkloadSpecs,
  sha256,
  snapshotCommandEnvironment,
} from './bytecode-vm-phase-7-contract.mjs';
import { createPhase7EvidenceRoot } from './bytecode-vm-phase-7-evidence-root.mjs';
import {
  checkPhase7Evidence,
  finalizePhase7Evidence,
} from './bytecode-vm-phase-7-evidence.mjs';
import {
  receiptDigest,
  writePhase7BlockedReceipt,
  writePhase7CommandReceipt,
  writePhase7GenesisReceipt,
} from './bytecode-vm-phase-7-receipts.mjs';
import { PHASE7_CARRIER_ENV } from './bytecode-vm-phase-7-whole-system-harness.mjs';
import { assertNoUnsafeHttpBypassEnvironment } from './http_live_process.mjs';
import {
  PHASE5_CARRIER_ENV,
  PHASE5_RUNTIME_BIN_ENV,
} from './bytecode-vm-phase-5-gate-runner.mjs';
import {
  PHASE6_CARRIER_ENV,
  PHASE6_RUNTIME_BIN_ENV,
} from './bytecode-vm-phase-6-gate-runner.mjs';

const OUTPUT_ENV = 'SKIFF_BYTECODE_VM_PHASE7_EVIDENCE_DIR';
const COMMIT_ENV = 'SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_COMMIT';
const TREE_ENV = 'SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_TREE';
export const PHASE7_CARGO_LEASE_DIR = '/tmp/skiff-bcvm-p7-r1-cargo.lockdir';
export const PHASE7_CARGO_TARGET_DIR = '/Users/geek/workspace/.skiff-cargo-target';
export const PHASE7_CARGO_LEASE_WAIT_MS = 30 * 60 * 1000;

export function parsePhase7GateArgs(args, { env = process.env } = {}) {
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

export async function runPhase7Gate(options, {
  repoRoot,
  capture = captureOwnedCommand,
  signalTarget = process,
  now = () => new Date().toISOString(),
  env = process.env,
  assertCargoLeaseFree = assertPhase7CargoLeaseFree,
  acquireCargoLease = acquirePhase7CargoLease,
} = {}) {
  assertNoUnsafeHttpBypassEnvironment(env);
  assertBytecodeVmGateEnvironment(env);
  const input = await validateInput(options, repoRoot);
  const evidenceRoot = await createPhase7EvidenceRoot(input.outputDir);
  const candidateSpecs = phase7CandidateSpecs(input.repoRoot);
  const workloadSpecs = phase7WorkloadSpecs(input.repoRoot);
  assertPhase7Catalog(input.repoRoot);
  const specCatalogDigest = phase7SpecCatalogDigest(input.repoRoot);
  const childEnvironment = {
    ...env,
    CARGO_TARGET_DIR: PHASE7_CARGO_TARGET_DIR,
    [PHASE7_CARRIER_ENV]: input.carrierRoot,
    [PHASE6_CARRIER_ENV]: input.carrierRoot,
    [PHASE6_RUNTIME_BIN_ENV]: join(PHASE7_CARGO_TARGET_DIR, 'debug', 'runtime'),
    [PHASE5_CARRIER_ENV]: input.carrierRoot,
    [PHASE5_RUNTIME_BIN_ENV]: join(PHASE7_CARGO_TARGET_DIR, 'debug', 'runtime'),
  };
  const specById = new Map(
    [...candidateSpecs, ...workloadSpecs].map((spec) => [spec.id, spec]),
  );
  const commandEnvironments = new Map(
    [...specById.keys()].map((id) => [id, snapshotCommandEnvironment(childEnvironment)]),
  );
  const order = phase7ExecutionOrder(input.repoRoot);
  const abortController = new AbortController();
  let interruptedBy = null;
  const handlers = new Map(['SIGINT', 'SIGTERM'].map((signal) => [signal, () => {
    interruptedBy ??= signal;
    abortController.abort(new Error(`Phase 7 r1 Gate interrupted by ${signal}`));
  }]));
  for (const [signal, handler] of handlers) signalTarget.on(signal, handler);
  const startedAt = now();
  const outcomes = new Map();
  let previousDigest = null;
  let sequence = 0;
  let releaseCargoLease = null;
  const writeGenesis = async () => {
    const genesis = await writePhase7GenesisReceipt(evidenceRoot, {
      expectedCommit: input.expectedCommit,
      expectedTree: input.expectedTree,
      specCatalogDigest,
    });
    previousDigest = receiptDigest(genesis);
  };
  try {
    await writeGenesis();
    for (const id of ['preflight-head', 'preflight-tree', 'preflight-status']) {
      outcomes.set(id, await execute(id));
    }
    if (preflightMatches(outcomes, input) && interruptedBy === null) {
      await assertCargoLeaseFree(PHASE7_CARGO_LEASE_DIR);
      releaseCargoLease = await acquireCargoLease(PHASE7_CARGO_LEASE_DIR);
      for (const id of order.slice(3, 3 + workloadSpecs.length)) {
        if (interruptedBy !== null) break;
        await executeWorkload(id);
      }
      for (const id of order.slice(3 + workloadSpecs.length)) {
        if (interruptedBy !== null) break;
        outcomes.set(id, await execute(id));
      }
    }
  } finally {
    if (releaseCargoLease !== null) await releaseCargoLease();
    for (const [signal, handler] of handlers) signalTarget.off(signal, handler);
  }
  try {
    const manifest = await finalizePhase7Evidence({
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
      const checked = await checkPhase7Evidence(input.outputDir, {
        ...input,
        directoryIdentities: evidenceRoot.identities(),
        commandEnvironments,
      });
      await evidenceRoot.assertAll();
      if (checked.verdict !== manifest.verdict
        || JSON.stringify(checked.failures) !== JSON.stringify(manifest.failures)) {
        checkerError = 'Phase 7 checker did not agree with the stored assessment';
      }
    } catch (error) {
      checkerError = error instanceof Error ? error.message : String(error);
    }
    return {
      manifest,
      checkerError,
      outputDir: input.outputDir,
      manifestSha256: sha256(JSON.stringify(manifest)),
    };
  } finally {
    await rm(input.carrierRoot, { recursive: true, force: true });
  }

  async function execute(id) {
    const spec = specById.get(id);
    await evidenceRoot.assertAll();
    const commandStartedAt = now();
    const actualEnv = commandEnvironments.get(id);
    const outcome = await capture(spec.command, [...spec.args], {
      cwd: spec.cwd,
      env: actualEnv,
      signal: abortController.signal,
    });
    await evidenceRoot.assertAll();
    sequence += 1;
    const receipt = await writePhase7CommandReceipt(evidenceRoot, spec, actualEnv, outcome, {
      sequence,
      priorReceiptDigest: previousDigest,
      stdout: outcome.stdout,
      stderr: outcome.stderr,
      startedAt: commandStartedAt,
      finishedAt: now(),
      interruptedBy,
    });
    previousDigest = receiptDigest(receipt);
    return outcome;
  }

  async function executeWorkload(id) {
    const spec = specById.get(id);
    const dependencies = spec.dependsOn ?? [];
    const failedProducers = dependencies.filter((dependency) =>
      !successfulOutcome(outcomes.get(dependency)));
    if (failedProducers.length === 0) {
      outcomes.set(id, await execute(id));
      return;
    }
    await evidenceRoot.assertAll();
    const startedAt = now();
    sequence += 1;
    const receipt = await writePhase7BlockedReceipt(
      evidenceRoot,
      spec,
      commandEnvironments.get(id),
      {
        sequence,
        priorReceiptDigest: previousDigest,
        blockedBy: failedProducers,
        startedAt,
        finishedAt: now(),
      },
    );
    previousDigest = receiptDigest(receipt);
  }
}

export async function assertPhase7CargoLeaseFree(
  leaseDir = PHASE7_CARGO_LEASE_DIR,
  { accessPath = access } = {},
) {
  try {
    await accessPath(leaseDir);
  } catch (error) {
    if (error?.code === 'ENOENT') return;
    throw error;
  }
  throw new Error(`Phase 7 r1 Cargo lease is already held: ${leaseDir}`);
}

export async function acquirePhase7CargoLease(
  leaseDir = PHASE7_CARGO_LEASE_DIR,
  {
    makeDirectory = mkdir,
    removeDirectory = rmdir,
    delayMs = 500,
    timeoutMs = PHASE7_CARGO_LEASE_WAIT_MS,
    now = Date.now,
  } = {},
) {
  const deadline = now() + timeoutMs;
  for (;;) {
    try {
      await makeDirectory(leaseDir);
      break;
    } catch (error) {
      if (error?.code !== 'EEXIST') throw error;
      if (now() >= deadline) {
        throw new Error(`Phase 7 r1 Cargo lease stayed held past the wait budget: ${leaseDir}`);
      }
      await delay(delayMs);
    }
  }
  let released = false;
  return async () => {
    if (released) return;
    released = true;
    await removeDirectory(leaseDir);
  };
}

export async function removeStalePhase7CargoLease(
  leaseDir = PHASE7_CARGO_LEASE_DIR,
  { owningProcessAlive = false, interruptedEvidencePath = null, removeDirectory = rmdir } = {},
) {
  if (owningProcessAlive) {
    throw new Error(`refusing unsafe stale-lease removal while an owning Cargo/rustc/Gate process remains: ${leaseDir}`);
  }
  if (typeof interruptedEvidencePath !== 'string' || interruptedEvidencePath.length === 0) {
    throw new Error('stale-lease removal must record the interrupted evidence path first');
  }
  await removeDirectory(leaseDir);
  return { leaseDir, interruptedEvidencePath };
}

export function phase7GateUsage() {
  return `usage: node scripts/run-bytecode-vm-phase-7-gate.mjs \\
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
    throw new Error('Phase 7 carrier root must not overlap the candidate repository');
  }
  try {
    await lstat(carrierRoot);
    throw new Error('Phase 7 carrier root must not already exist');
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

function successfulOutcome(outcome) {
  return outcome?.code === 0 && outcome?.signal === null && outcome?.error == null;
}

function successfulText(outcome) {
  return successfulOutcome(outcome) ? outcome.stdout : null;
}