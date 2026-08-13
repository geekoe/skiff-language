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
import { phase3WorkloadSpecs } from './bytecode-vm-phase-3-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary as parsePhase4TestSummary,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE4_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-4-command-v1';
export const PHASE4_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-4-gate-v1';

const HOST_PHASE4_VCP =
  'host::request_entry::phase_4_vcp_tests::phase_4_vcp_production_composition';
const HOST_PHASE4_SENTINEL_ADMISSION =
  'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_source_to_admission';
const HOST_PHASE4_SENTINEL_EMISSION =
  'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_admission_to_emission';
const HOST_PHASE4_SENTINEL_LINK =
  'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_emission_to_link';
const HOST_PHASE4_SENTINEL_VERIFY =
  'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_link_to_verify';
const HOST_PHASE4_SENTINEL_SCHEDULER =
  'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_verify_to_scheduler';
const HOST_PHASE4_SENTINEL_RESPONSE =
  'host::request_entry::phase_4_vcp_tests::phase_4_stage_sentinel_scheduler_to_request_response';
const HOST_PHASE4_NEGATIVE_CANCEL =
  'host::request_entry::phase_4_vcp_tests::phase_4_negative_cancel_before_complete';
const HOST_PHASE4_NEGATIVE_DEADLINE =
  'host::request_entry::phase_4_vcp_tests::phase_4_negative_deadline_race';
const HOST_PHASE4_NEGATIVE_DUPLICATE =
  'host::request_entry::phase_4_vcp_tests::phase_4_negative_duplicate_wake_drop';
const HOST_PHASE4_NEGATIVE_DISCONNECT =
  'host::request_entry::phase_4_vcp_tests::phase_4_negative_session_disconnect';

// Every Phase 4 scenario lane plus the full Phase 1/2/3 regression lane. A
// scenario may be carried by one command and one command may serve several
// lanes; the coverage assertion only requires each lane to appear at least
// once.
export const PHASE4_REQUIRED_LANES = Object.freeze([
  'VCP',
  'NEG',
  'SENTINEL',
  'K4',
  'V4',
  'C4',
  'P4G',
  'phase-3-regression',
]);

// Phase 4 scenario commands. The K4/V4/C4 focused commands are join contracts
// pinned to the exact test-name filter words each lane must land:
//
// - K4 scheduler: the accepted Phase 2/3 pending-cell machinery already owns
//   `enqueues_once`, `park`, `duplicate`, and `concurrent_terminal_race`
//   tests in `skiff-runtime-scheduler`; those four filters are the
//   publish/wake/claim-once, park/resume, duplicate-drop, and terminal-race
//   authority and must keep matching exactly those invariants after the
//   Phase 4 kernel lands.
// - V4 linker/verifier: both crates must land focused tests whose names
//   contain `host_effect` (typed pinned-registry entry; ActualWithResume
//   HostEffect pending contract).
// - C4 emission: `skiff-compiler-emission` must land focused admission tests
//   whose names contain `phase_4_admission` (mirroring the accepted
//   `phase_3_admission` convention) admitting `std.time.sleep` alone.
//
// A zero-hit filter is rejected by the real Gate (rust summary is not exact
// and complete), which is the honest expected-red until that lane joins.
export function phase4ScenarioSpecs(root) {
  return Object.freeze([
    spec(root, 'phase-4-gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-4-gate-*.test.mjs',
    ], 'node-tap', ['P4G']),
    spec(root, 'phase-4-vcp-production-composition', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_VCP,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['VCP', 'K4', 'V4', 'C4']),
    spec(root, 'phase-4-stage-sentinel-source-to-admission', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_SENTINEL_ADMISSION,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['SENTINEL', 'C4']),
    spec(root, 'phase-4-stage-sentinel-admission-to-emission', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_SENTINEL_EMISSION,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['SENTINEL', 'C4']),
    spec(root, 'phase-4-stage-sentinel-emission-to-link', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_SENTINEL_LINK,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['SENTINEL', 'V4']),
    spec(root, 'phase-4-stage-sentinel-link-to-verify', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_SENTINEL_VERIFY,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['SENTINEL', 'V4']),
    spec(root, 'phase-4-stage-sentinel-verify-to-scheduler', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_SENTINEL_SCHEDULER,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['SENTINEL', 'K4']),
    spec(root, 'phase-4-stage-sentinel-scheduler-to-request-response', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_SENTINEL_RESPONSE,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['SENTINEL', 'K4']),
    spec(root, 'phase-4-negative-cancel-before-complete', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_NEGATIVE_CANCEL,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'K4']),
    spec(root, 'phase-4-negative-deadline-race', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_NEGATIVE_DEADLINE,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'K4']),
    spec(root, 'phase-4-negative-duplicate-wake-drop', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_NEGATIVE_DUPLICATE,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'K4']),
    spec(root, 'phase-4-negative-session-disconnect', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE4_NEGATIVE_DISCONNECT,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['NEG', 'K4']),
    spec(root, 'k4-scheduler-pending-publish-claim', 'cargo', [
      'test', '-p', 'skiff-runtime-scheduler', '--lib', 'enqueues_once',
    ], 'rust-suite', ['K4']),
    spec(root, 'k4-scheduler-park-resume', 'cargo', [
      'test', '-p', 'skiff-runtime-scheduler', '--lib', 'park',
    ], 'rust-suite', ['K4']),
    spec(root, 'k4-scheduler-duplicate-wake', 'cargo', [
      'test', '-p', 'skiff-runtime-scheduler', '--lib', 'duplicate',
    ], 'rust-suite', ['K4']),
    spec(root, 'k4-scheduler-terminal-race', 'cargo', [
      'test', '-p', 'skiff-runtime-scheduler', '--lib', 'concurrent_terminal_race',
    ], 'rust-suite', ['K4']),
    spec(root, 'v4-linker-typed-host-entry', 'cargo', [
      'test', '-p', 'skiff-runtime-linker', '--lib', 'host_effect',
    ], 'rust-suite', ['V4']),
    spec(root, 'v4-verifier-pending-contract', 'cargo', [
      'test', '-p', 'skiff-runtime-bytecode-verifier', '--lib', 'host_effect',
    ], 'rust-suite', ['V4']),
    spec(root, 'c4-emission-host-effect-admission', 'cargo', [
      'test', '-p', 'skiff-compiler-emission', '--lib', 'phase_4_admission',
    ], 'rust-suite', ['C4']),
    spec(root, 'phase-4-fmt-check', 'cargo', [
      'fmt', '--all', '--', '--check',
    ], null, ['P4G']),
    spec(root, 'phase-4-clippy-check', 'cargo', [
      'clippy', '--workspace',
    ], null, ['P4G']),
  ]);
}

// Phase 1/2/3 full regression: the accepted Phase 3 workload matrix verbatim
// (thirty-four commands, itself carrying the Phase 1 and Phase 2 full
// regression), re-id'd under the Phase 4 epoch with the Phase 3 regression
// lane appended. The command args and test formats are reused, never
// re-derived.
function phase3RegressionSpecs(root) {
  return Object.freeze(phase3WorkloadSpecs(root).map((entry) => spec(
    entry.cwd,
    `phase-3-regression-${entry.id}`,
    entry.command,
    [...entry.args],
    entry.testFormat,
    [...entry.lanes, 'phase-3-regression'],
  )));
}

export function phase4WorkloadSpecs(root) {
  return Object.freeze([
    ...phase4ScenarioSpecs(root),
    ...phase3RegressionSpecs(root),
  ]);
}

export function phase4CandidateSpecs(root) {
  return phase1CandidateSpecs(root);
}

export function assertPhase4LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = PHASE4_REQUIRED_LANES.filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 4 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
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
