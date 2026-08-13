import {
  phase1CandidateSpecs,
  phase1WorkloadSpecs,
  parsePhase1TestSummary,
} from './bytecode-vm-phase-1-contract.mjs';
import {
  assertGitObject,
  commandEnvironmentIdentity,
  phase2ScenarioSpecs,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
} from './bytecode-vm-phase-2-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary as parsePhase3TestSummary,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE3_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-3-command-v1';
export const PHASE3_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-3-gate-v1';

const HOST_PHASE3_VCP =
  'host::request_entry::phase_3_vcp_tests::phase_3_vcp_production_composition';
const HOST_PHASE3_MISMATCH =
  'host::request_entry::phase_3_vcp_tests::phase_3_negative_catch_mismatch';
const HOST_PHASE3_UNCAUGHT =
  'host::request_entry::phase_3_vcp_tests::phase_3_negative_uncaught_throw';
const HOST_PHASE3_HOST_PENDING =
  'host::request_entry::phase_3_vcp_tests::phase_3_negative_host_pending_throw';
const HOST_PHASE3_RESUME =
  'host::request_entry::phase_3_vcp_tests::phase_3_controlled_resume_harness';

// Every Phase 3 scenario lane plus the Phase 1 and Phase 2 full-regression
// lanes. A scenario may be carried by one command and one command may serve
// several lanes; the coverage assertion only requires each lane to appear at
// least once.
export const PHASE3_REQUIRED_LANES = Object.freeze([
  'VCP',
  'NEG',
  'K3',
  'C3',
  'P3G',
  'phase-1-regression',
  'phase-2-regression',
]);

// Phase 3 scenario commands. The K3/C3 focused commands are join contracts
// pinned to the exact test names each lane landed: K3's VM coverage lives
// under `fiber::tests::catch*` and C3's admission/emission coverage lives in
// `skiff-compiler-emission` (`phase_3_admission*`, `throw*`). Each filter
// matches at least one test and yields exactly one `test result:` line.
export function phase3ScenarioSpecs(root) {
  return Object.freeze([
    spec(root, 'phase-3-gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-3-gate-*.test.mjs',
    ], 'node-tap', ['P3G']),
    spec(root, 'phase-3-vcp-production-composition', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE3_VCP,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['VCP', 'K3', 'C3']),
    spec(root, 'phase-3-negative-catch-mismatch', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE3_MISMATCH,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'K3', 'C3']),
    spec(root, 'phase-3-negative-uncaught-throw', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE3_UNCAUGHT,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'K3']),
    spec(root, 'phase-3-negative-host-pending-throw', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE3_HOST_PENDING,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'C3']),
    spec(root, 'phase-3-controlled-resume-harness', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE3_RESUME,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['VCP', 'K3']),
    spec(root, 'k3-vm-throw-unwind', 'cargo', [
      'test', '-p', 'skiff-runtime-vm', '--lib', 'catch',
    ], 'rust-suite', ['K3']),
    spec(root, 'k3-model-service-error-envelope', 'cargo', [
      'test', '-p', 'skiff-runtime-model', '--lib', 'service_error',
    ], 'rust-suite', ['K3']),
    spec(root, 'k3-scheduler-resume-throw', 'cargo', [
      'test', '-p', 'skiff-runtime-scheduler', '--lib', 'resume',
    ], 'rust-suite', ['K3']),
    spec(root, 'k3-request-user-error', 'cargo', [
      'test', '-p', 'skiff-runtime-request', '--lib', 'throw',
    ], 'rust-suite', ['K3']),
    spec(root, 'k3-linker-throw-admission', 'cargo', [
      'test', '-p', 'skiff-runtime-linker', '--lib', 'capability',
    ], 'rust-suite', ['K3']),
    spec(root, 'c3-emission-throw-admission', 'cargo', [
      'test', '-p', 'skiff-compiler-emission', '--lib', 'phase_3_admission',
    ], 'rust-suite', ['C3']),
    spec(root, 'c3-emission-throw-emission', 'cargo', [
      'test', '-p', 'skiff-compiler-emission', '--lib', 'throw',
    ], 'rust-suite', ['C3']),
  ]);
}

// Phase 1 full regression: the accepted Phase 1 workload specs verbatim (the
// twelve commands including the Phase 1 Gate self-test), re-id'd under the
// Phase 3 epoch with the Phase 1 regression lane appended. The command args
// and test formats are reused, never re-derived.
function phase1RegressionSpecs(root) {
  return Object.freeze(phase1WorkloadSpecs(root).map((entry) => spec(
    entry.cwd,
    `phase-1-regression-${entry.id}`,
    entry.command,
    [...entry.args],
    entry.testFormat,
    [...entry.lanes, 'phase-1-regression'],
  )));
}

// Phase 2 full regression: the nine Phase 2 scenario commands verbatim,
// re-id'd under the Phase 3 epoch with the Phase 2 regression lane appended.
// (The Phase 2 Gate's own nested Phase 1 regression is not duplicated: the
// Phase 1 lane above already covers it once.)
function phase2RegressionSpecs(root) {
  return Object.freeze(phase2ScenarioSpecs(root).map((entry) => spec(
    entry.cwd,
    `phase-2-regression-${entry.id}`,
    entry.command,
    [...entry.args],
    entry.testFormat,
    [...entry.lanes, 'phase-2-regression'],
  )));
}

export function phase3WorkloadSpecs(root) {
  return Object.freeze([
    ...phase3ScenarioSpecs(root),
    ...phase1RegressionSpecs(root),
    ...phase2RegressionSpecs(root),
  ]);
}

export function phase3CandidateSpecs(root) {
  return phase1CandidateSpecs(root);
}

export function assertPhase3LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = PHASE3_REQUIRED_LANES.filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 3 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
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
