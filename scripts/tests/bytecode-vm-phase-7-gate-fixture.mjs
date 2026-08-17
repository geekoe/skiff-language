import { mkdir, mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  phase7CandidateSpecs,
  phase7EffectiveTestCount,
  phase7ExecutionOrder,
  phase7SpecCatalogDigest,
  phase7WorkloadSpecs,
  snapshotCommandEnvironment,
} from '../lib/bytecode-vm-phase-7-contract.mjs';
import { finalizePhase7Evidence } from '../lib/bytecode-vm-phase-7-evidence.mjs';
import { createPhase7EvidenceRoot } from '../lib/bytecode-vm-phase-7-evidence-root.mjs';
import {
  receiptDigest,
  writePhase7BlockedReceipt,
  writePhase7CommandReceipt,
  writePhase7GenesisReceipt,
} from '../lib/bytecode-vm-phase-7-receipts.mjs';
import { tap as phase6Tap, rust as phase6Rust } from './bytecode-vm-phase-6-gate-fixture.mjs';

export const COMMIT = 'a'.repeat(40);
export const TREE = 'b'.repeat(40);
export const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

export const PRODUCER_ID = 'phase-7-whole-system-producer';
export const CONSUMER_ID = 'phase-7-whole-system-consumer';

export function tap(overrides = {}) {
  return phase6Tap(overrides);
}

export function rust(overrides = {}) {
  return phase6Rust(overrides);
}

export async function withPhase7EvidenceBundle(options, assertion) {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-gate-test-'));
  const temp = await realpath(created);
  const repoRoot = REPOSITORY;
  const outputDir = join(temp, 'evidence');
  try {
    const evidenceRoot = await createPhase7EvidenceRoot(outputDir);
    const commandEnvironment = snapshotCommandEnvironment({
      PATH: '/usr/bin:/bin',
      PHASE7_BOUND_ENV: 'before-execution',
    });
    const commandEnvironments = new Map(
      [...phase7CandidateSpecs(repoRoot), ...phase7WorkloadSpecs(repoRoot)]
        .map((spec) => [spec.id, commandEnvironment]),
    );
    await writeAllReceipts({
      evidenceRoot,
      repoRoot,
      commandEnvironments,
      options,
    });
    const manifest = await finalizePhase7Evidence({
      evidenceRoot,
      repoRoot,
      expectedCommit: COMMIT,
      expectedTree: TREE,
      commandEnvironments,
      startedAt: '2026-08-17T00:00:00.000Z',
      finishedAt: '2026-08-17T00:00:02.000Z',
    });
    await assertion({
      temp,
      repoRoot,
      outputDir,
      directoryIdentities: evidenceRoot.identities(),
      commandEnvironments,
      manifest,
    });
  } finally {
    await rm(created, { recursive: true, force: true });
  }
}

async function writeAllReceipts({
  evidenceRoot,
  repoRoot,
  commandEnvironments,
  options,
}) {
  const genesis = await writePhase7GenesisReceipt(evidenceRoot, {
    expectedCommit: COMMIT,
    expectedTree: TREE,
    specCatalogDigest: options?.crossEpoch
      ? '0'.repeat(64)
      : phase7SpecCatalogDigest(repoRoot),
  });
  let previousDigest = receiptDigest(genesis);
  const specById = new Map(
    [...phase7CandidateSpecs(repoRoot), ...phase7WorkloadSpecs(repoRoot)]
      .map((spec) => [spec.id, spec]),
  );
  let sequence = 0;
  for (const id of phase7ExecutionOrder(repoRoot)) {
    if (id === options?.missingId) continue;
    sequence += 1;
    const spec = specById.get(id);
    const actualEnv = id === options?.environmentDriftId
      ? snapshotCommandEnvironment({
        PATH: '/usr/bin:/bin',
        PHASE7_BOUND_ENV: 'drifted',
      })
      : commandEnvironments.get(id);
    if (id === options?.blockedConsumerId) {
      const receipt = await writePhase7BlockedReceipt(
        evidenceRoot,
        spec,
        actualEnv,
        {
          sequence,
          priorReceiptDigest: previousDigest,
          blockedBy: [options?.failedProducerId ?? PRODUCER_ID],
          startedAt: '2026-08-17T00:00:00.000Z',
          finishedAt: '2026-08-17T00:00:01.000Z',
        },
      );
      previousDigest = receiptDigest(receipt);
      continue;
    }
    const failed = id === options?.failedProducerId;
    const candidate = candidateOutput(id, options);
    const stdout = failed ? failedOutput(spec, options) : specOutput(spec, options);
    const receipt = await writePhase7CommandReceipt(
      evidenceRoot,
      spec,
      actualEnv,
      failed ? { code: 101, signal: null, error: null } : { code: 0, signal: null, error: null },
      {
        sequence,
        priorReceiptDigest: previousDigest,
        stdout: candidate ?? stdout,
        stderr: failed ? 'Phase 7 fixture producer failure\n' : '',
        startedAt: '2026-08-17T00:00:00.000Z',
        finishedAt: '2026-08-17T00:00:01.000Z',
      },
    );
    previousDigest = receiptDigest(receipt);
  }
}

function candidateOutput(id, options) {
  if (id.endsWith('-head')) {
    if (id === 'closure-head' && options?.closureChanged) return `${'c'.repeat(40)}\n`;
    return `${options?.stale ? 'c'.repeat(40) : COMMIT}\n`;
  }
  if (id.endsWith('-tree')) return `${options?.stale ? 'd'.repeat(40) : TREE}\n`;
  if (id.endsWith('-status')) return options?.dirty ? ' M tracked-file\n' : '';
  return null;
}

function specOutput(spec, options) {
  const effective = phase7EffectiveTestCount(spec);
  if (spec.testFormat === 'node-tap') {
    const total = effective ?? 14;
    return tap({
      ...options?.nodeCounts,
      total,
      passed: options?.nodeCounts?.passed ?? total,
    });
  }
  if (spec.testFormat === 'rust-exact') {
    return rust({ ...options?.rustCounts, passed: 1 });
  }
  if (spec.testFormat === 'rust-suite') {
    const passed = options?.rustCounts?.passed ?? effective ?? 3;
    return rust({ ...options?.rustCounts, passed });
  }
  return '';
}

function failedOutput(spec, options) {
  const effective = phase7EffectiveTestCount(spec);
  if (spec.testFormat === 'node-tap') {
    const total = effective ?? 14;
    return tap({ ...options?.nodeCounts, total, passed: 0, failed: total });
  }
  if (spec.testFormat !== null) {
    return rust({ ...options?.rustCounts, passed: 0, failed: 1 });
  }
  return '';
}