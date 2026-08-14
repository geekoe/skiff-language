import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  assertPhase5LaneCoverage,
  parsePhase5TestSummary,
  PHASE5_COMMAND_SCHEMA,
  PHASE5_MANIFEST_SCHEMA,
  PHASE5_REQUIRED_LANES,
  phase5CandidateSpecs,
  phase5ScenarioSpecs,
  phase5WorkloadSpecs,
} from '../lib/bytecode-vm-phase-5-contract.mjs';
import {
  PHASE5_DIRECTORY_IDENTITY_FILE,
  PHASE5_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-5-evidence-root.mjs';
import { rust, tap } from './bytecode-vm-phase-5-gate-fixture.mjs';
import { buildVerifyPlan, PUBLIC_SELECTORS } from '../lib/verify-plan.mjs';

const ROOT = '/candidate';
const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

test('r1 schemas cannot accept a receipt from the interrupted Phase 5 epoch', () => {
  assert.equal(PHASE5_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-5-command-r1-v1');
  assert.equal(PHASE5_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-5-gate-r1-v1');
  assert.equal(PHASE5_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-5-directory-identity-r1-v1');
  assert.equal(PHASE5_DIRECTORY_IDENTITY_FILE, 'phase-5-r1-directory-identities.json');
});

test('r1 matrix names all G1-G10 owners and uses only executable commands', () => {
  const scenarios = phase5ScenarioSpecs(ROOT);
  const workloads = phase5WorkloadSpecs(ROOT);
  assert.equal(scenarios.length, 42);
  assert.doesNotThrow(() => assertPhase5LaneCoverage(workloads));
  const observed = new Set(workloads.flatMap(({ lanes }) => lanes));
  for (const lane of PHASE5_REQUIRED_LANES) {
    assert.equal(observed.has(lane), true, `${lane} must be covered`);
  }
  assert.equal(scenarios.every(({ command }) => ['cargo', 'node'].includes(command)), true);
  assert.equal(scenarios.some(({ id }) => id.includes('source-scan')), false);
});

test('six sentinels select six independent integration tests with nonzero accounting', () => {
  const sentinels = phase5ScenarioSpecs(ROOT)
    .filter(({ id }) => /^phase-5-s[1-6]-/.test(id));
  assert.equal(sentinels.length, 6);
  assert.equal(new Set(sentinels.map(({ args }) => args.at(-4))).size, 6);
  for (const sentinel of sentinels) {
    assert.equal(sentinel.command, 'cargo');
    assert.equal(sentinel.testFormat, 'rust-exact');
    assert.deepEqual(sentinel.args.slice(0, 6), [
      'test', '--no-fail-fast', '--manifest-path', 'runtime/host/Cargo.toml',
      '--test', 'bytecode_vm_phase_5',
    ]);
    assert.deepEqual(sentinel.args.slice(-3), ['--', '--exact', '--nocapture']);
  }
});

test('G7 is a Router integration binary rather than a host fake dispatcher', () => {
  const spec = phase5ScenarioSpecs(ROOT)
    .find(({ id }) => id === 'phase-5-router-full-chain-vcp');
  assert.deepEqual(spec, {
    id: 'phase-5-router-full-chain-vcp',
    command: 'cargo',
    args: Object.freeze([
      'test', '--no-fail-fast', '--manifest-path', 'router/Cargo.toml',
      '--test', 'bytecode_vm_phase_5', 'phase_5_router_full_chain_vcp',
      '--', '--exact', '--nocapture',
    ]),
    cwd: ROOT,
    testFormat: 'rust-exact',
    lanes: Object.freeze(['G7', 'G8', 'H5', 'P5G']),
  });
});

test('G7 builds the production Runtime process before the Router integration selector', () => {
  const scenarios = phase5ScenarioSpecs(ROOT);
  const buildIndex = scenarios.findIndex(({ id }) => id === 'phase-5-runtime-process-binary');
  const routerIndex = scenarios.findIndex(({ id }) => id === 'phase-5-router-full-chain-vcp');
  assert.equal(buildIndex >= 0 && buildIndex < routerIndex, true);
  assert.deepEqual(scenarios[buildIndex], {
    id: 'phase-5-runtime-process-binary',
    command: 'cargo',
    args: Object.freeze(['build', '-p', 'runtime', '--bin', 'runtime']),
    cwd: ROOT,
    testFormat: null,
    lanes: Object.freeze(['G7', 'G8', 'H5', 'P5G']),
  });
});

test('G5/G8 include the gated TCP upstream and single-worker canary', () => {
  const byId = Object.fromEntries(phase5ScenarioSpecs(ROOT).map((entry) => [entry.id, entry]));
  assert.equal(byId['phase-5-deterministic-tcp-upstream'].args.includes(
    'tcp_server::deterministic_tcp_server_gates_unary_and_distinguishes_streams'), true);
  assert.equal(byId['phase-5-single-worker-canary'].args.includes(
    'phase_5_single_worker_canary'), true);
  assert.equal(byId['phase-5-lifecycle-race-matrix'].lanes.includes('G8'), true);
});

test('A5/C5/V5 focused joins pin every typed positive and fail-closed companion', () => {
  const byId = Object.fromEntries(phase5ScenarioSpecs(ROOT).map((entry) => [entry.id, entry]));
  assert.equal(byId['a5-exact-executor-registry'].args.includes('executor_identit'), true);
  assert.equal(byId['a5-exact-executor-registry'].expectedTests, 2);
  assert.equal(byId['a5-ordinary-shape-affine-child-rejection'].args.includes(
    'ordinary_shape_cannot_embed_an_affine_resource_field'), true);
  assert.equal(byId['c5-exact-registry-source-emission'].args.includes(
    'exact_registry_executors_flow_from_real_source_to_public_emission'), true);
  assert.equal(byId['c5-affine-body-take-emission'].args.includes(
    'exact_stream_body_flows_from_real_source_to_affine_take_and_recursive_drop'), true);
  assert.equal(byId['c5-unsupported-registry-rows-fail-closed'].args.includes(
    'registry_rows_without_executor_identity_fail_before_value_shape_admission'), true);
  assert.equal(byId['c5-second-body-take-fails-closed'].args.includes(
    'a_second_real_source_body_take_fails_before_emission'), true);
  assert.equal(byId['c5-production-affine-publication'].args.includes(
    'production_authoring_publishes_exact_affine_http_stream_bytecode'), true);
  assert.equal(byId['v5-production-affine-image'].args.includes(
    'production_stream_image_proves_exact_privileged_shape_and_affine_body_take'), true);
  assert.equal(byId['v5-linker-stream-dual-resume'].args.includes(
    'backend_links_stream_next_dual_resume_successors'), true);
  assert.equal(byId['v5-stream-read-resume-certificates'].args.includes('stream_read'), true);
  assert.equal(byId['v5-swapped-resume-target-rejection'].args.includes(
    'swapped_resume_targets_fail_at_exact_hydration_binding'), true);
  assert.equal(byId['v5-verifier-executor-identity-rejections'].expectedTests, 2);
  assert.equal(byId['v5-affine-take-proof'].args.includes('affine_take_tests'), true);
  assert.equal(byId['v5-affine-take-proof'].expectedTests, 6);
  assert.equal(byId['h5-production-bytecode-http-composition'].args.includes(
    'phase_5_bytecode_http'), true);
  assert.equal(byId['h5-production-bytecode-http-composition'].expectedTests, 3);
  assert.equal(byId['h5-server-stream-flush-ack'].args.includes('stream_flush_ack'), true);
  assert.equal(byId['h5-server-stream-flush-ack'].expectedTests, 4);
  assert.equal(byId['k5-request-resource-materialization'].expectedTests, 2);
  assert.equal(byId['k5-scheduler-first-poll-publication'].expectedTests, 1);
  assert.equal(byId['k5-request-first-poll-http-arbitration'].expectedTests, 6);
  assert.equal(phase5ScenarioSpecs(ROOT).some(({ args }) => (
    args.includes('phase_5_admission') || args.includes('stream_resume')
  )), false);
});

test('every Rust proof command is serial-friendly and no-fail-fast', () => {
  const rustCommands = phase5ScenarioSpecs(ROOT)
    .filter(({ command, args }) => command === 'cargo' && args[0] === 'test');
  assert.equal(rustCommands.length > 0, true);
  assert.equal(rustCommands.every(({ args }) => args.includes('--no-fail-fast')), true);
});

test('the accepted Phase 4 matrix is reused verbatim as the Phase 1-4 regression', () => {
  const regression = phase5WorkloadSpecs(ROOT)
    .filter(({ id }) => id.startsWith('phase-4-regression-'));
  assert.equal(regression.length, 55);
  assert.equal(regression.every(({ lanes }) => lanes.includes('phase-4-regression')), true);
  assert.deepEqual(regression.find(({ id }) => id.endsWith('-phase-4-gate-self-tests')).args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-4-gate-*.test.mjs',
  ]);
});

test('candidate closure and command count are frozen by the matrix', () => {
  assert.equal(phase5CandidateSpecs(ROOT).length, 12);
  assert.equal(phase5WorkloadSpecs(ROOT).length, 97);
  assert.equal(phase5CandidateSpecs(ROOT).length + phase5WorkloadSpecs(ROOT).length, 109);
  assert.deepEqual(phase5CandidateSpecs(ROOT).slice(-3).map(({ id }) => id), [
    'fresh-head', 'fresh-tree', 'fresh-status',
  ]);
});

test('public verify selector reaches the exclusive Phase 5 r1 Gate runner', async () => {
  assert.equal(PUBLIC_SELECTORS.includes('bytecode-vm-phase-5-gate'), true);
  const plan = await buildVerifyPlan({
    root: REPOSITORY,
    selectors: ['bytecode-vm-phase-5-gate'],
    catalogRoot: REPOSITORY,
  });
  assert.deepEqual(plan.tasks, [{
    id: 'bytecode-vm-phase-5:gate',
    kind: 'implementation:runtime',
    command: 'node',
    args: ['scripts/run-bytecode-vm-phase-5-gate.mjs'],
    cwd: REPOSITORY,
    exclusive: true,
  }]);
});

test('test summaries reject zero, skip, todo, cancel, ignore, and imprecise exact runs', () => {
  assert.equal(parsePhase5TestSummary('node-tap', tap()).valid, true);
  assert.equal(parsePhase5TestSummary('node-tap', tap({ total: 0, passed: 0 })).valid, false);
  assert.equal(parsePhase5TestSummary('node-tap', tap({ passed: 1, skipped: 1 })).valid, false);
  assert.equal(parsePhase5TestSummary('node-tap', tap({ passed: 1, todo: 1 })).valid, false);
  assert.equal(parsePhase5TestSummary('node-tap', tap({ passed: 1, cancelled: 1 })).valid, false);
  assert.equal(parsePhase5TestSummary('rust-suite', rust({ passed: 3 })).valid, true);
  assert.equal(parsePhase5TestSummary('rust-suite', rust({ passed: 0 })).valid, false);
  assert.equal(parsePhase5TestSummary('rust-suite', rust({ ignored: 1 })).valid, false);
  assert.equal(parsePhase5TestSummary('rust-exact', rust({ passed: 1 })).valid, true);
  assert.equal(parsePhase5TestSummary('rust-exact', rust({ passed: 2 })).valid, false);
  assert.equal(parsePhase5TestSummary(
    'rust-suite', `${rust({ passed: 3 })}${rust({ passed: 4 })}`,
  ).valid, false);
});
