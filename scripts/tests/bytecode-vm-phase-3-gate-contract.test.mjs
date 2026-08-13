import assert from 'node:assert/strict';
import test from 'node:test';

import {
  assertPhase3LaneCoverage,
  parsePhase3TestSummary,
  PHASE3_COMMAND_SCHEMA,
  PHASE3_MANIFEST_SCHEMA,
  PHASE3_REQUIRED_LANES,
  phase3CandidateSpecs,
  phase3ScenarioSpecs,
  phase3WorkloadSpecs,
} from '../lib/bytecode-vm-phase-3-contract.mjs';
import {
  PHASE3_DIRECTORY_IDENTITY_FILE,
  PHASE3_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-3-evidence-root.mjs';
import { rust, tap } from './bytecode-vm-phase-3-gate-fixture.mjs';

const ROOT = '/candidate';

test('Phase 3 schemas are independent from the accepted Phase 0/1/2 epochs', () => {
  assert.equal(PHASE3_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-3-command-v1');
  assert.equal(PHASE3_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-3-gate-v1');
  assert.doesNotMatch(PHASE3_MANIFEST_SCHEMA, /phase-[012]/);
  assert.equal(PHASE3_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-3-directory-identity-v1');
  assert.equal(PHASE3_DIRECTORY_IDENTITY_FILE, 'phase-3-directory-identities.json');
});

test('day-one matrix contains thirteen Phase 3 scenarios and every required lane', () => {
  const specs = phase3ScenarioSpecs(ROOT);
  assert.equal(specs.length, 13);
  assert.doesNotThrow(() => assertPhase3LaneCoverage(phase3WorkloadSpecs(ROOT)));
  const observed = new Set(phase3WorkloadSpecs(ROOT).flatMap(({ lanes }) => lanes));
  for (const lane of PHASE3_REQUIRED_LANES) {
    assert.equal(observed.has(lane), true, `${lane} must be covered`);
  }
  const byId = Object.fromEntries(specs.map((entry) => [entry.id, entry]));
  assert.deepEqual(byId['phase-3-vcp-production-composition'], {
    id: 'phase-3-vcp-production-composition',
    command: 'cargo',
    args: Object.freeze([
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib',
      'host::request_entry::phase_3_vcp_tests::phase_3_vcp_production_composition',
      '--', '--exact', '--nocapture',
    ]),
    cwd: ROOT,
    testFormat: 'rust-exact',
    lanes: Object.freeze(['VCP', 'K3', 'C3']),
  });
  assert.deepEqual(byId['phase-3-negative-host-pending-throw'], {
    id: 'phase-3-negative-host-pending-throw',
    command: 'cargo',
    args: Object.freeze([
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib',
      'host::request_entry::phase_3_vcp_tests::phase_3_negative_host_pending_throw',
      '--', '--exact', '--nocapture',
    ]),
    cwd: ROOT,
    testFormat: 'rust-exact',
    lanes: Object.freeze(['NEG', 'C3']),
  });
  assert.equal(specs.filter(({ lanes }) => lanes.includes('K3')).length >= 6, true);
  assert.equal(specs.filter(({ lanes }) => lanes.includes('C3')).length >= 4, true);
  assert.deepEqual(byId['phase-3-gate-self-tests'].args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-3-gate-*.test.mjs',
  ]);
  // The three focused join-contract filters must match the exact test names
  // K3 and C3 landed: zero-hit filters are rejected by the real Gate.
  assert.deepEqual(byId['k3-vm-throw-unwind'], {
    id: 'k3-vm-throw-unwind',
    command: 'cargo',
    args: Object.freeze([
      'test', '-p', 'skiff-runtime-vm', '--lib', 'catch',
    ]),
    cwd: ROOT,
    testFormat: 'rust-suite',
    lanes: Object.freeze(['K3']),
  });
  assert.deepEqual(byId['c3-emission-throw-admission'], {
    id: 'c3-emission-throw-admission',
    command: 'cargo',
    args: Object.freeze([
      'test', '-p', 'skiff-compiler-emission', '--lib', 'phase_3_admission',
    ]),
    cwd: ROOT,
    testFormat: 'rust-suite',
    lanes: Object.freeze(['C3']),
  });
  assert.deepEqual(byId['c3-emission-throw-emission'], {
    id: 'c3-emission-throw-emission',
    command: 'cargo',
    args: Object.freeze([
      'test', '-p', 'skiff-compiler-emission', '--lib', 'throw',
    ]),
    cwd: ROOT,
    testFormat: 'rust-suite',
    lanes: Object.freeze(['C3']),
  });
});

test('Phase 1 and Phase 2 full regression are reused verbatim under the Phase 3 epoch', () => {
  const workloads = phase3WorkloadSpecs(ROOT);
  const phase1 = workloads.filter(({ id }) => id.startsWith('phase-1-regression-'));
  const phase2 = workloads.filter(({ id }) => id.startsWith('phase-2-regression-'));
  assert.equal(phase1.length, 12);
  assert.equal(phase2.length, 9);
  assert.equal(
    phase1.every(({ lanes }) => lanes.includes('phase-1-regression')),
    true,
  );
  assert.equal(
    phase2.every(({ lanes }) => lanes.includes('phase-2-regression')),
    true,
  );
  const p1Gate = phase1.find(({ id }) => id.endsWith('-gate-self-tests'));
  assert.deepEqual(p1Gate.args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-0-gate-*.test.mjs',
    'scripts/tests/bytecode-vm-phase-1-gate-*.test.mjs',
  ]);
  const p2Gate = phase2.find(({ id }) => id.endsWith('-gate-self-tests'));
  assert.deepEqual(p2Gate.args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-2-gate-*.test.mjs',
  ]);
  const p2Vcp = phase2.find(({ id }) => id.endsWith('-vcp-production-composition'));
  assert.equal(p2Vcp.testFormat, 'rust-exact');
});

test('workload count is thirty-four on top of the twelve candidate probes', () => {
  assert.equal(phase3CandidateSpecs(ROOT).length, 12);
  assert.equal(phase3WorkloadSpecs(ROOT).length, 34);
});

test('candidate closure fixes four receipt-backed identity snapshots', () => {
  const specs = phase3CandidateSpecs(ROOT);
  assert.equal(specs.length, 12);
  assert.deepEqual(specs.slice(-3).map(({ id }) => id), [
    'fresh-head', 'fresh-tree', 'fresh-status',
  ]);
});

test('test summaries reject zero, skip, todo, cancel, ignore, and imprecise exact runs', () => {
  assert.equal(parsePhase3TestSummary('node-tap', tap()).valid, true);
  assert.equal(parsePhase3TestSummary('node-tap', tap({ total: 0, passed: 0 })).valid, false);
  assert.equal(parsePhase3TestSummary('node-tap', tap({ passed: 1, skipped: 1 })).valid, false);
  assert.equal(parsePhase3TestSummary('node-tap', tap({ passed: 1, todo: 1 })).valid, false);
  assert.equal(parsePhase3TestSummary('node-tap', tap({ passed: 1, cancelled: 1 })).valid, false);
  assert.equal(parsePhase3TestSummary('rust-suite', rust({ passed: 3 })).valid, true);
  assert.equal(parsePhase3TestSummary('rust-suite', rust({ passed: 0 })).valid, false);
  assert.equal(parsePhase3TestSummary('rust-suite', rust({ ignored: 1 })).valid, false);
  assert.equal(parsePhase3TestSummary('rust-exact', rust({ passed: 1 })).valid, true);
  assert.equal(parsePhase3TestSummary('rust-exact', rust({ passed: 2 })).valid, false);
  assert.equal(
    parsePhase3TestSummary('rust-suite', `${rust({ passed: 3 })}${rust({ passed: 4 })}`).valid,
    false,
  );
});
