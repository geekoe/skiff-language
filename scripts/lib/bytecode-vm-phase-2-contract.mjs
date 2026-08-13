import {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary,
  phase1CandidateSpecs,
  phase1WorkloadSpecs,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
} from './bytecode-vm-phase-1-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary as parsePhase2TestSummary,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE2_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-2-command-v1';
export const PHASE2_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-2-gate-v1';

const HOST_PHASE2_VCP =
  'host::request_entry::phase_2_vcp_tests::phase_2_vcp_production_composition';
const HOST_PHASE2_NEGATIVE =
  'host::request_entry::phase_2_vcp_tests::phase_2_missing_plan_negative';

// Every Phase 2 scenario lane plus the Phase 1 full-regression lane. A scenario
// may be carried by one command and one command may serve several lanes; the
// coverage assertion only requires each lane to appear at least once.
export const PHASE2_REQUIRED_LANES = Object.freeze([
  'VCP',
  'NEG',
  'K2',
  'C2',
  'P2G',
  'phase-1-regression',
]);

// Phase 2 scenario commands. K2/C2 focused commands are P2G-chosen join
// contracts: each lane must land tests matching these exact cargo filters in
// the same join that flips its scenario green. They stay expected-red until
// then, so the real Gate correctly reports FAIL for the unjoined scenarios.
export function phase2ScenarioSpecs(root) {
  return Object.freeze([
    spec(root, 'phase-2-gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-2-gate-*.test.mjs',
    ], 'node-tap', ['P2G']),
    spec(root, 'phase-2-vcp-production-composition', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE2_VCP,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['VCP', 'K2', 'C2']),
    spec(root, 'phase-2-missing-plan-negative', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE2_NEGATIVE,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'C2']),
    spec(root, 'k2-lifecycle-executor', 'cargo', [
      'test', '-p', 'skiff-runtime-vm', '--lib', 'lifecycle',
    ], 'rust-suite', ['K2']),
    spec(root, 'k2-model-writable-path', 'cargo', [
      'test', '-p', 'skiff-runtime-model', '--lib', 'vm_heap',
    ], 'rust-suite', ['K2']),
    spec(root, 'k2-request-heap-cow', 'cargo', [
      'test', '-p', 'skiff-runtime-request', '--lib', 'vm_heap',
    ], 'rust-suite', ['K2']),
    spec(root, 'k2-linker-record-array-admission', 'cargo', [
      'test', '-p', 'skiff-runtime-linker', '--lib', 'capability',
    ], 'rust-suite', ['K2']),
    spec(root, 'c2-pipeline-exact-facts', 'cargo', [
      'test', '-p', 'skiff-compiler', '--lib', 'phase_2_bytecode_admission',
    ], 'rust-suite', ['C2']),
    spec(root, 'c2-emission-exact-plan', 'cargo', [
      'test', '-p', 'skiff-compiler-emission', '--lib', 'phase_2_bytecode_admission',
    ], 'rust-suite', ['C2']),
  ]);
}

// Phase 1 full regression: the accepted Phase 1 workload specs verbatim (the
// twelve commands including the Phase 1 Gate self-test), re-id'd under the
// Phase 2 epoch with the regression lane appended. The command args and test
// formats are reused, never re-derived.
export function phase2RegressionSpecs(root) {
  return Object.freeze(phase1WorkloadSpecs(root).map((entry) => spec(
    entry.cwd,
    `phase-1-regression-${entry.id}`,
    entry.command,
    [...entry.args],
    entry.testFormat,
    [...entry.lanes, 'phase-1-regression'],
  )));
}

export function phase2WorkloadSpecs(root) {
  return Object.freeze([
    ...phase2ScenarioSpecs(root),
    ...phase2RegressionSpecs(root),
  ]);
}

export function phase2CandidateSpecs(root) {
  return phase1CandidateSpecs(root);
}

export function assertPhase2LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = PHASE2_REQUIRED_LANES.filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 2 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
  }
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
