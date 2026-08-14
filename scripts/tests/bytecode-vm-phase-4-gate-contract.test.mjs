import assert from 'node:assert/strict';
import test from 'node:test';

import {
  assertPhase4LaneCoverage,
  parsePhase4TestSummary,
  PHASE4_COMMAND_SCHEMA,
  PHASE4_MANIFEST_SCHEMA,
  PHASE4_REQUIRED_LANES,
  phase4CandidateSpecs,
  phase4ScenarioSpecs,
  phase4WorkloadSpecs,
} from '../lib/bytecode-vm-phase-4-contract.mjs';
import {
  PHASE4_DIRECTORY_IDENTITY_FILE,
  PHASE4_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-4-evidence-root.mjs';
import { rust, tap } from './bytecode-vm-phase-4-gate-fixture.mjs';

const ROOT = '/candidate';

test('Phase 4 schemas are independent from the accepted Phase 0/1/2/3 epochs', () => {
  assert.equal(PHASE4_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-4-command-v1');
  assert.equal(PHASE4_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-4-gate-v1');
  assert.doesNotMatch(PHASE4_MANIFEST_SCHEMA, /phase-[0123]/);
  assert.equal(PHASE4_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-4-directory-identity-v1');
  assert.equal(PHASE4_DIRECTORY_IDENTITY_FILE, 'phase-4-directory-identities.json');
});

test('retired-stage matrix contains twenty Phase 4 scenarios and every required lane', () => {
  const specs = phase4ScenarioSpecs(ROOT);
  assert.equal(specs.length, 20);
  assert.doesNotThrow(() => assertPhase4LaneCoverage(phase4WorkloadSpecs(ROOT)));
  const observed = new Set(phase4WorkloadSpecs(ROOT).flatMap(({ lanes }) => lanes));
  for (const lane of PHASE4_REQUIRED_LANES) {
    assert.equal(observed.has(lane), true, `${lane} must be covered`);
  }
  const byId = Object.fromEntries(specs.map((entry) => [entry.id, entry]));
  assert.deepEqual(byId['phase-4-vcp-production-composition'], {
    id: 'phase-4-vcp-production-composition',
    command: 'cargo',
    args: Object.freeze([
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib',
      'host::request_entry::phase_4_vcp_tests::phase_4_vcp_production_composition',
      '--', '--exact', '--nocapture',
    ]),
    cwd: ROOT,
    testFormat: 'rust-exact',
    lanes: Object.freeze(['VCP', 'K4', 'V4', 'C4']),
  });
  assert.equal(specs.filter(({ lanes }) => lanes.includes('SENTINEL')).length, 6);
  assert.deepEqual(
    specs
      .filter(({ lanes }) => lanes.includes('SENTINEL'))
      .map(({ args }) => args.at(-4)),
    [
      'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_source_to_admission',
      'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_admission_to_emission',
      'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_emission_to_atomic_link_input',
      'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_atomic_link_to_image',
      'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_image_to_scheduler',
      'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_scheduler_to_request_response',
    ],
  );
  assert.equal(specs.filter(({ lanes }) => lanes.includes('NEG')).length, 4);
  assert.equal(specs.filter(({ lanes }) => lanes.includes('K4')).length >= 10, true);
  assert.equal(specs.filter(({ lanes }) => lanes.includes('V4')).length >= 4, true);
  assert.equal(specs.filter(({ lanes }) => lanes.includes('C4')).length >= 3, true);
  assert.deepEqual(byId['phase-4-gate-self-tests'].args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-4-gate-*.test.mjs',
  ]);
  // The focused join-contract filters must match the exact test-name words
  // each lane lands: zero-hit filters are rejected by the real Gate.
  assert.deepEqual(byId['k4-scheduler-pending-publish-claim'], {
    id: 'k4-scheduler-pending-publish-claim',
    command: 'cargo',
    args: Object.freeze([
      'test', '-p', 'skiff-runtime-scheduler', '--lib', 'enqueues_once',
    ]),
    cwd: ROOT,
    testFormat: 'rust-suite',
    lanes: Object.freeze(['K4']),
  });
  assert.deepEqual(byId['v4-linker-typed-host-entry'].args, [
    'test', '-p', 'skiff-runtime-linker', '--lib', 'host_effect',
  ]);
  assert.deepEqual(
    [...new Set(specs
      .filter(({ id }) => id.startsWith('v4-'))
      .map(({ args }) => args[args.indexOf('-p') + 1]))],
    ['skiff-runtime-linker'],
  );
  assert.deepEqual(byId['c4-emission-host-effect-admission'].args, [
    'test', '-p', 'skiff-compiler-emission', '--lib', 'phase_4_admission',
  ]);
  assert.deepEqual(byId['phase-4-fmt-check'], {
    id: 'phase-4-fmt-check',
    command: 'cargo',
    args: Object.freeze(['fmt', '--all', '--', '--check']),
    cwd: ROOT,
    testFormat: null,
    lanes: Object.freeze(['P4G']),
  });
  assert.deepEqual(byId['phase-4-clippy-check'], {
    id: 'phase-4-clippy-check',
    command: 'cargo',
    args: Object.freeze(['clippy', '--workspace']),
    cwd: ROOT,
    testFormat: null,
    lanes: Object.freeze(['P4G']),
  });
});

test('the accepted Phase 3 matrix is reused verbatim as the full Phase 1/2/3 regression', () => {
  const workloads = phase4WorkloadSpecs(ROOT);
  const regression = workloads.filter(({ id }) => id.startsWith('phase-3-regression-'));
  assert.equal(regression.length, 34);
  assert.equal(
    regression.every(({ lanes }) => lanes.includes('phase-3-regression')),
    true,
  );
  const p3Gate = regression.find(({ id }) => id.endsWith('-gate-self-tests'));
  assert.deepEqual(p3Gate.args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-3-gate-*.test.mjs',
  ]);
  const p1Regression = regression.filter(({ id }) => id.includes('phase-1-regression-'));
  assert.equal(p1Regression.length, 12);
  const p2Regression = regression.filter(({ id }) => id.includes('phase-2-regression-'));
  assert.equal(p2Regression.length, 9);
});

test('workload count is fifty-four on top of the twelve candidate probes', () => {
  assert.equal(phase4CandidateSpecs(ROOT).length, 12);
  assert.equal(phase4WorkloadSpecs(ROOT).length, 54);
});

test('candidate closure fixes four receipt-backed identity snapshots', () => {
  const specs = phase4CandidateSpecs(ROOT);
  assert.equal(specs.length, 12);
  assert.deepEqual(specs.slice(-3).map(({ id }) => id), [
    'fresh-head', 'fresh-tree', 'fresh-status',
  ]);
});

test('test summaries reject zero, skip, todo, cancel, ignore, and imprecise exact runs', () => {
  assert.equal(parsePhase4TestSummary('node-tap', tap()).valid, true);
  assert.equal(parsePhase4TestSummary('node-tap', tap({ total: 0, passed: 0 })).valid, false);
  assert.equal(parsePhase4TestSummary('node-tap', tap({ passed: 1, skipped: 1 })).valid, false);
  assert.equal(parsePhase4TestSummary('node-tap', tap({ passed: 1, todo: 1 })).valid, false);
  assert.equal(parsePhase4TestSummary('node-tap', tap({ passed: 1, cancelled: 1 })).valid, false);
  assert.equal(parsePhase4TestSummary('rust-suite', rust({ passed: 3 })).valid, true);
  assert.equal(parsePhase4TestSummary('rust-suite', rust({ passed: 0 })).valid, false);
  assert.equal(parsePhase4TestSummary('rust-suite', rust({ ignored: 1 })).valid, false);
  assert.equal(parsePhase4TestSummary('rust-exact', rust({ passed: 1 })).valid, true);
  assert.equal(parsePhase4TestSummary('rust-exact', rust({ passed: 2 })).valid, false);
  assert.equal(
    parsePhase4TestSummary('rust-suite', `${rust({ passed: 3 })}${rust({ passed: 4 })}`).valid,
    false,
  );
});
