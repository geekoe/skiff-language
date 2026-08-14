import {
  phase1CandidateSpecs,
  parsePhase1TestSummary,
} from './bytecode-vm-phase-1-contract.mjs';
import {
  assertGitObject,
  commandEnvironmentIdentity,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
} from './bytecode-vm-phase-2-contract.mjs';
import { phase4WorkloadSpecs } from './bytecode-vm-phase-4-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary as parsePhase5TestSummary,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE5_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-5-command-r1-v1';
export const PHASE5_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-5-gate-r1-v1';

export const PHASE5_REQUIRED_LANES = Object.freeze([
  'G1',
  'G2',
  'G3',
  'G4',
  'G5',
  'G6',
  'G7',
  'G8',
  'G9',
  'G10',
  'A5',
  'C5',
  'V5',
  'K5',
  'H5',
  'P5G',
  'phase-4-regression',
]);

const HOST_TEST = Object.freeze({
  tcp: 'tcp_server::deterministic_tcp_server_gates_unary_and_distinguishes_streams',
  s1: 'phase_5_stage_sentinel_source_to_admission',
  s2: 'stages::phase_5_stage_sentinel_admission_to_emission',
  s3: 'stages::phase_5_stage_sentinel_emission_to_link',
  s4: 'stages::phase_5_stage_sentinel_link_to_verify',
  s5: 'phase_5_stage_sentinel_verify_to_scheduler',
  s6: 'phase_5_stage_sentinel_scheduler_to_request_response',
  vcp: 'phase_5_vcp_production_composition',
  lifecycle: 'phase_5_lifecycle_race_matrix',
  canary: 'phase_5_single_worker_canary',
  structure: 'phase_5_structural_no_bypass',
});

const ROUTER_VCP = 'phase_5_router_full_chain_vcp';

export function phase5ScenarioSpecs(root) {
  return Object.freeze([
    spec(root, 'phase-5-gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-5-gate-*.test.mjs',
    ], 'node-tap', ['G10', 'P5G']),
    hostExact(root, 'phase-5-deterministic-tcp-upstream', HOST_TEST.tcp, ['G5', 'G8', 'P5G']),
    hostExact(root, 'phase-5-s1-source-to-admission', HOST_TEST.s1, ['G1', 'C5', 'P5G']),
    hostExact(root, 'phase-5-s2-admission-to-emission', HOST_TEST.s2, ['G2', 'A5', 'C5', 'P5G']),
    hostExact(root, 'phase-5-s3-emission-to-link', HOST_TEST.s3, ['G3', 'V5', 'P5G']),
    hostExact(root, 'phase-5-s4-link-to-verify', HOST_TEST.s4, ['G4', 'V5', 'P5G']),
    hostExact(root, 'phase-5-s5-verify-to-scheduler', HOST_TEST.s5, ['G5', 'K5', 'H5', 'P5G']),
    hostExact(root, 'phase-5-s6-request-response', HOST_TEST.s6, ['G6', 'K5', 'H5', 'P5G']),
    hostExact(root, 'phase-5-vcp-production-composition', HOST_TEST.vcp,
      ['G5', 'G6', 'G8', 'K5', 'H5', 'P5G']),
    hostExact(root, 'phase-5-lifecycle-race-matrix', HOST_TEST.lifecycle,
      ['G8', 'K5', 'H5', 'P5G']),
    hostExact(root, 'phase-5-single-worker-canary', HOST_TEST.canary,
      ['G5', 'G8', 'K5', 'H5', 'P5G']),
    hostExact(root, 'phase-5-structural-no-bypass', HOST_TEST.structure,
      ['G9', 'A5', 'V5', 'K5', 'H5', 'P5G']),
    spec(root, 'phase-5-runtime-process-binary', 'cargo', [
      'build', '-p', 'runtime', '--bin', 'runtime',
    ], null, ['G7', 'G8', 'H5', 'P5G']),
    routerExact(root, 'phase-5-router-full-chain-vcp', ROUTER_VCP, ['G7', 'G8', 'H5', 'P5G']),
    rustSuite(root, 'a5-exact-executor-registry', 'skiff-artifact-model',
      'executor_identit', ['G3', 'G9', 'A5'], 2),
    rustSuite(root, 'a5-privileged-http-stream-composite', 'skiff-artifact-model',
      'privileged_http_stream_composite', ['G2', 'G4', 'G9', 'A5'], 2),
    rustSuite(root, 'a5-affine-take-opcode', 'skiff-artifact-model',
      'take_dense_field_requires_exact_privileged_affine_field', ['G2', 'G4', 'G9', 'A5'], 1),
    rustSuite(root, 'a5-ordinary-shape-affine-child-rejection', 'skiff-artifact-model',
      'ordinary_shape_cannot_embed_an_affine_resource_field', ['G2', 'G4', 'G9', 'A5'], 1),
    rustSuite(root, 'c5-exact-registry-source-emission', 'skiff-compiler-emission',
      'exact_registry_executors_flow_from_real_source_to_public_emission',
      ['G1', 'G2', 'G9', 'C5'], 1),
    rustSuite(root, 'c5-affine-body-take-emission', 'skiff-compiler-emission',
      'exact_stream_body_flows_from_real_source_to_affine_take_and_recursive_drop',
      ['G1', 'G2', 'G9', 'C5'], 1),
    rustSuite(root, 'c5-unsupported-registry-rows-fail-closed', 'skiff-compiler-emission',
      'registry_rows_without_executor_identity_fail_before_value_shape_admission',
      ['G1', 'G9', 'C5'], 1),
    rustSuite(root, 'c5-second-body-take-fails-closed', 'skiff-compiler-emission',
      'a_second_real_source_body_take_fails_before_emission', ['G1', 'G2', 'G9', 'C5'], 1),
    rustSuite(root, 'c5-production-affine-publication', 'skiff-compiler',
      'production_authoring_publishes_exact_affine_http_stream_bytecode',
      ['G1', 'G2', 'G9', 'C5'], 1),
    rustSuite(root, 'v5-production-affine-image', 'skiff-runtime-linker',
      'production_stream_image_proves_exact_privileged_shape_and_affine_body_take',
      ['G3', 'G4', 'G9', 'V5'], 1),
    rustSuite(root, 'v5-indexed-typed-executor-target', 'skiff-runtime-linker',
      'production_sleep_image_exposes_only_the_indexed_typed_executor_target',
      ['G3', 'G9', 'V5'], 1),
    rustSuite(root, 'v5-registry-executor-identity-closure', 'skiff-runtime-linker',
      'executor_identity', ['G3', 'G9', 'V5'], 5),
    rustSuite(root, 'v5-host-signature-drift-rejections', 'skiff-runtime-linker',
      'wrong_sleep_', ['G3', 'G9', 'V5'], 4),
    rustSuite(root, 'v5-host-binding-key-rejections', 'skiff-runtime-linker',
      'binding_key_fails_closed', ['G3', 'G9', 'V5'], 2),
    rustSuite(root, 'v5-verifier-executor-identity-rejections',
      'skiff-runtime-bytecode-verifier', 'executor_identity', ['G4', 'G9', 'V5'], 2),
    rustSuite(root, 'v5-linker-stream-dual-resume', 'skiff-runtime-linker',
      'backend_links_stream_next_dual_resume_successors', ['G3', 'G4', 'G9', 'V5'], 1),
    rustSuite(root, 'v5-stream-read-resume-certificates', 'skiff-runtime-bytecode-verifier',
      'stream_read', ['G4', 'G9', 'V5'], 2),
    rustSuite(root, 'v5-swapped-resume-target-rejection', 'skiff-runtime-bytecode-verifier',
      'swapped_resume_targets_fail_at_exact_hydration_binding', ['G4', 'G9', 'V5'], 1),
    rustSuite(root, 'v5-affine-take-proof', 'skiff-runtime-bytecode-verifier',
      'affine_take_tests', ['G4', 'G9', 'V5'], 6),
    rustSuite(root, 'v5-privileged-sibling-read-verifier-rejections',
      'skiff-runtime-bytecode-verifier', 'get_dense_field_cannot_read_privileged_',
      ['G4', 'G9', 'V5'], 2),
    rustSuite(root, 'v5-privileged-sibling-read-linker-rejection', 'skiff-runtime-linker',
      'privileged_headers_and_status_dense_reads_fail_closed', ['G3', 'G4', 'G9', 'V5'], 1),
    rustSuite(root, 'k5-scheduler-resource-authority', 'skiff-runtime-scheduler',
      'phase_5_resource', ['G5', 'G6', 'G8', 'G9', 'K5']),
    rustSuite(root, 'k5-request-resource-materialization', 'skiff-runtime-request',
      'phase_5_resource', ['G5', 'G6', 'G8', 'G9', 'K5'], 2),
    rustSuite(root, 'k5-scheduler-first-poll-publication', 'skiff-runtime-scheduler',
      'phase_5_first_poll', ['G5', 'G8', 'K5'], 1),
    rustSuite(root, 'k5-request-first-poll-http-arbitration', 'skiff-runtime-request',
      'phase_5_first_poll', ['G5', 'G8', 'K5'], 6),
    rustSuite(root, 'k5-capacity-one-stream-lifecycle', 'skiff-runtime-scheduler',
      'phase_5_stream', ['G5', 'G6', 'G8', 'K5']),
    rustSuite(root, 'h5-production-bytecode-http-composition', 'skiff-runtime-host',
      'phase_5_bytecode_http', ['G5', 'G6', 'G8', 'G9', 'H5'], 3),
    rustSuite(root, 'h5-server-stream-flush-ack', 'skiff-runtime-host',
      'stream_flush_ack', ['G6', 'G7', 'G8', 'H5'], 4),
    spec(root, 'phase-5-fmt-check', 'cargo', [
      'fmt', '--all', '--', '--check',
    ], null, ['G10', 'P5G']),
    spec(root, 'phase-5-clippy-check', 'cargo', [
      'clippy', '--workspace', '--all-targets', '--all-features',
    ], null, ['G10', 'P5G']),
  ]);
}

export function phase5RegressionSpecs(root) {
  return Object.freeze(phase4WorkloadSpecs(root).map((entry) => spec(
    entry.cwd,
    `phase-4-regression-${entry.id}`,
    entry.command,
    [...entry.args],
    entry.testFormat,
    [...entry.lanes, 'phase-4-regression'],
  )));
}

export function phase5WorkloadSpecs(root) {
  return Object.freeze([
    ...phase5ScenarioSpecs(root),
    ...phase5RegressionSpecs(root),
  ]);
}

export function phase5CandidateSpecs(root) {
  return phase1CandidateSpecs(root);
}

export function assertPhase5LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = PHASE5_REQUIRED_LANES.filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 5 r1 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
  }
}

function hostExact(cwd, id, testName, lanes) {
  return spec(cwd, id, 'cargo', [
    'test', '--no-fail-fast', '--manifest-path', 'runtime/host/Cargo.toml',
    '--test', 'bytecode_vm_phase_5', testName, '--', '--exact', '--nocapture',
  ], 'rust-exact', lanes);
}

function routerExact(cwd, id, testName, lanes) {
  return spec(cwd, id, 'cargo', [
    'test', '--no-fail-fast', '--manifest-path', 'router/Cargo.toml',
    '--test', 'bytecode_vm_phase_5', testName, '--', '--exact', '--nocapture',
  ], 'rust-exact', lanes);
}

function rustSuite(cwd, id, packageName, filter, lanes, expectedTests = null) {
  const entry = spec(cwd, id, 'cargo', [
    'test', '--no-fail-fast', '-p', packageName, '--lib', filter, '--', '--nocapture',
  ], 'rust-suite', lanes);
  return Object.freeze({ ...entry, expectedTests });
}

function spec(cwd, id, command, args, testFormat = null, lanes = []) {
  return Object.freeze({
    id,
    command,
    args: Object.freeze(args),
    cwd,
    testFormat,
    lanes: Object.freeze(lanes),
  });
}
