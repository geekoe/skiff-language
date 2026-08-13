import assert from 'node:assert/strict';
import test from 'node:test';

import {
  assertPhase2LaneCoverage,
  parsePhase2TestSummary,
  PHASE2_COMMAND_SCHEMA,
  PHASE2_MANIFEST_SCHEMA,
  PHASE2_REQUIRED_LANES,
  phase2CandidateSpecs,
  phase2RegressionSpecs,
  phase2ScenarioSpecs,
  phase2WorkloadSpecs,
} from '../lib/bytecode-vm-phase-2-contract.mjs';
import {
  PHASE2_DIRECTORY_IDENTITY_FILE,
  PHASE2_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-2-evidence-root.mjs';
import { rust, tap } from './bytecode-vm-phase-2-gate-fixture.mjs';

const ROOT = '/candidate';

test('Phase 2 schemas are independent from the accepted Phase 0/1 epochs', () => {
  assert.equal(PHASE2_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-2-command-v1');
  assert.equal(PHASE2_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-2-gate-v1');
  assert.doesNotMatch(PHASE2_MANIFEST_SCHEMA, /phase-[01]/);
  assert.equal(PHASE2_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-2-directory-identity-v1');
  assert.equal(PHASE2_DIRECTORY_IDENTITY_FILE, 'phase-2-directory-identities.json');
});

test('day-one matrix contains nine Phase 2 scenarios and every required lane', () => {
  const specs = phase2ScenarioSpecs(ROOT);
  assert.equal(specs.length, 9);
  assert.doesNotThrow(() => assertPhase2LaneCoverage([...specs, ...phase2RegressionSpecs(ROOT)]));
  const observed = new Set(phase2WorkloadSpecs(ROOT).flatMap(({ lanes }) => lanes));
  for (const lane of PHASE2_REQUIRED_LANES) {
    assert.equal(observed.has(lane), true, `${lane} must be covered`);
  }
  const byId = Object.fromEntries(specs.map((entry) => [entry.id, entry]));
  assert.deepEqual(byId['phase-2-vcp-production-composition'], {
    id: 'phase-2-vcp-production-composition',
    command: 'cargo',
    args: Object.freeze([
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib',
      'host::request_entry::phase_2_vcp_tests::phase_2_vcp_production_composition',
      '--', '--exact', '--nocapture',
    ]),
    cwd: ROOT,
    testFormat: 'rust-exact',
    lanes: Object.freeze(['VCP', 'K2', 'C2']),
  });
  assert.deepEqual(byId['phase-2-missing-plan-negative'], {
    id: 'phase-2-missing-plan-negative',
    command: 'cargo',
    args: Object.freeze([
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib',
      'host::request_entry::phase_2_vcp_tests::phase_2_missing_plan_negative',
      '--', '--exact', '--nocapture',
    ]),
    cwd: ROOT,
    testFormat: 'rust-exact',
    lanes: Object.freeze(['NEG', 'C2']),
  });
  assert.equal(specs.filter(({ lanes }) => lanes.includes('K2')).length >= 4, true);
  assert.equal(specs.filter(({ lanes }) => lanes.includes('C2')).length >= 3, true);
  assert.deepEqual(byId['phase-2-gate-self-tests'].args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-2-gate-*.test.mjs',
  ]);
});

test('Phase 1 full regression is reused verbatim under the Phase 2 epoch', () => {
  const specs = phase2RegressionSpecs(ROOT);
  assert.equal(specs.length, 12);
  assert.equal(specs.every(({ id }) => id.startsWith('phase-1-regression-')), true);
  assert.equal(
    specs.every(({ lanes }) => lanes.includes('phase-1-regression')),
    true,
  );
  const gate = specs.find(({ id }) => id.endsWith('-gate-self-tests'));
  assert.deepEqual(gate.args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-0-gate-*.test.mjs',
    'scripts/tests/bytecode-vm-phase-1-gate-*.test.mjs',
  ]);
  const k0a = specs.filter(({ id }) => id.endsWith('-k0a-compiler-admission')
    || id.endsWith('-k0a-emission-admission'));
  assert.equal(k0a.length, 2);
  assert.equal(k0a.every(({ testFormat }) => testFormat === 'rust-suite'), true);
});

test('candidate closure fixes four receipt-backed identity snapshots', () => {
  const specs = phase2CandidateSpecs(ROOT);
  assert.equal(specs.length, 12);
  assert.deepEqual(specs.slice(-3).map(({ id }) => id), [
    'fresh-head', 'fresh-tree', 'fresh-status',
  ]);
});

test('test summaries reject zero, skip, todo, cancel, ignore, and imprecise exact runs', () => {
  assert.equal(parsePhase2TestSummary('node-tap', tap()).valid, true);
  assert.equal(parsePhase2TestSummary('node-tap', tap({ total: 0, passed: 0 })).valid, false);
  assert.equal(parsePhase2TestSummary('node-tap', tap({ passed: 1, skipped: 1 })).valid, false);
  assert.equal(parsePhase2TestSummary('node-tap', tap({ passed: 1, todo: 1 })).valid, false);
  assert.equal(parsePhase2TestSummary('node-tap', tap({ passed: 1, cancelled: 1 })).valid, false);
  assert.equal(parsePhase2TestSummary('rust-suite', rust({ passed: 3 })).valid, true);
  assert.equal(parsePhase2TestSummary('rust-suite', rust({ passed: 0 })).valid, false);
  assert.equal(parsePhase2TestSummary('rust-suite', rust({ ignored: 1 })).valid, false);
  assert.equal(parsePhase2TestSummary('rust-exact', rust({ passed: 1 })).valid, true);
  assert.equal(parsePhase2TestSummary('rust-exact', rust({ passed: 2 })).valid, false);
  assert.equal(
    parsePhase2TestSummary('rust-suite', `${rust({ passed: 3 })}${rust({ passed: 4 })}`).valid,
    false,
  );
});
