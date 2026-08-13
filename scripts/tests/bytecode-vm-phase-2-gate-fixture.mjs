import { mkdir, mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  phase2CandidateSpecs,
  phase2WorkloadSpecs,
  snapshotCommandEnvironment,
} from '../lib/bytecode-vm-phase-2-contract.mjs';
import { finalizePhase2Evidence } from '../lib/bytecode-vm-phase-2-evidence.mjs';
import { createPhase2EvidenceRoot } from '../lib/bytecode-vm-phase-2-evidence-root.mjs';
import { writePhase2CommandReceipt } from '../lib/bytecode-vm-phase-2-receipts.mjs';

export const COMMIT = 'a'.repeat(40);
export const TREE = 'b'.repeat(40);

export async function withPhase2EvidenceBundle(options, assertion) {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase2-gate-test-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    const evidenceRoot = await createPhase2EvidenceRoot(outputDir);
    const candidateSpecs = phase2CandidateSpecs(repoRoot);
    const workloadSpecs = phase2WorkloadSpecs(repoRoot);
    const commandEnvironment = snapshotCommandEnvironment({
      PATH: '/usr/bin:/bin',
      PHASE2_BOUND_ENV: 'before-execution',
    });
    const commandEnvironments = new Map(
      [...candidateSpecs, ...workloadSpecs].map((spec) => [spec.id, commandEnvironment]),
    );
    for (const spec of [...candidateSpecs, ...workloadSpecs]) {
      if (spec.id === options?.missingId) continue;
      const candidate = candidateOutput(spec.id, options);
      const stdout = candidate ?? workloadOutput(spec, options);
      const interrupted = spec.id === options?.interruptedId;
      const actualEnv = spec.id === options?.environmentDriftId
        ? snapshotCommandEnvironment({ ...commandEnvironment, PHASE2_BOUND_ENV: 'drifted' })
        : commandEnvironment;
      await writePhase2CommandReceipt(evidenceRoot, spec, actualEnv, interrupted ? {
        code: null, signal: 'SIGTERM', error: new Error('interrupted'),
      } : { code: 0, signal: null, error: null }, {
        stdout,
        stderr: '',
        startedAt: '2026-08-13T00:00:00.000Z',
        finishedAt: '2026-08-13T00:00:01.000Z',
        interruptedBy: interrupted ? 'SIGTERM' : null,
      });
    }
    const manifest = await finalizePhase2Evidence({
      evidenceRoot,
      repoRoot,
      expectedCommit: COMMIT,
      expectedTree: TREE,
      commandEnvironments,
      startedAt: '2026-08-13T00:00:00.000Z',
      finishedAt: '2026-08-13T00:00:02.000Z',
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

function candidateOutput(id, options) {
  if (id.endsWith('-head')) {
    if (id === 'closure-head' && options?.closureChanged) return `${'c'.repeat(40)}\n`;
    return `${options?.stale ? 'c'.repeat(40) : COMMIT}\n`;
  }
  if (id.endsWith('-tree')) return `${options?.stale ? 'd'.repeat(40) : TREE}\n`;
  if (id.endsWith('-status')) return options?.dirty ? ' M tracked-file\n' : '';
  return null;
}

function workloadOutput(spec, options) {
  if (spec.testFormat === 'node-tap') return tap(options?.nodeCounts);
  if (spec.testFormat === 'rust-exact') return rust({ ...options?.rustCounts, passed: 1 });
  if (spec.testFormat === 'rust-suite') return rust(options?.rustCounts);
  return '';
}

export function tap(overrides = {}) {
  const counts = {
    total: 4,
    passed: 4,
    failed: 0,
    cancelled: 0,
    skipped: 0,
    todo: 0,
    ...overrides,
  };
  return [
    'TAP version 13',
    `1..${counts.total}`,
    `# tests ${counts.total}`,
    `# pass ${counts.passed}`,
    `# fail ${counts.failed}`,
    `# cancelled ${counts.cancelled}`,
    `# skipped ${counts.skipped}`,
    `# todo ${counts.todo}`,
    '',
  ].join('\n');
}

export function rust(overrides = {}) {
  const counts = { passed: 3, failed: 0, ignored: 0, measured: 0, filtered: 72, ...overrides };
  return `test result: ok. ${counts.passed} passed; ${counts.failed} failed; ${counts.ignored} ignored; ${counts.measured} measured; ${counts.filtered} filtered out; finished in 0.01s\n`;
}
