import { mkdir, mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  phase0CandidateSpecs,
  phase0WorkloadSpecs,
  snapshotCommandEnvironment,
} from '../lib/bytecode-vm-phase-0-contract.mjs';
import { finalizePhase0Evidence } from '../lib/bytecode-vm-phase-0-evidence.mjs';
import { createPhase0EvidenceRoot } from '../lib/bytecode-vm-phase-0-evidence-root.mjs';
import { writePhase0CommandReceipt } from '../lib/bytecode-vm-phase-0-receipts.mjs';

export const COMMIT = 'a'.repeat(40);
export const TREE = 'b'.repeat(40);

export async function withEvidenceBundle(options, assertion) {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase0-gate-test-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    const evidenceRoot = await createPhase0EvidenceRoot(outputDir);
    const candidateSpecs = phase0CandidateSpecs(repoRoot);
    const workloadSpecs = phase0WorkloadSpecs(repoRoot);
    const commandEnvironment = snapshotCommandEnvironment({
      PATH: '/usr/bin:/bin',
      P0_UNRECORDED_ENV: 'bound-before-execution',
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
        ? snapshotCommandEnvironment({ ...commandEnvironment, P0_UNRECORDED_ENV: 'drifted' })
        : commandEnvironment;
      await writePhase0CommandReceipt(evidenceRoot, spec, actualEnv, interrupted ? {
        code: null, signal: 'SIGTERM', error: new Error('interrupted'),
      } : { code: 0, signal: null, error: null }, {
        stdout,
        stderr: '',
        startedAt: '2026-08-13T00:00:00.000Z',
        finishedAt: '2026-08-13T00:00:01.000Z',
        interruptedBy: interrupted ? 'SIGTERM' : null,
      });
    }
    const manifest = await finalizePhase0Evidence({
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
  if (spec.testFormat === 'rust-exact') return rust(options?.rustCounts);
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

function rust(overrides = {}) {
  const counts = { passed: 1, failed: 0, ignored: 0, measured: 0, filtered: 72, ...overrides };
  return `test result: ok. ${counts.passed} passed; ${counts.failed} failed; ${counts.ignored} ignored; ${counts.measured} measured; ${counts.filtered} filtered out; finished in 0.01s\n`;
}
