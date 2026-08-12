import {
  assertGitObject,
  commandEnvironmentIdentity,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
} from './bytecode-vm-phase-0-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE1_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-1-command-v1';
export const PHASE1_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-1-gate-v1';

const HOST_SUCCESS =
  'host::request_entry::phase_0_vcp_tests::phase_0_vcp_production_composition';
const HOST_NEGATIVE =
  'host::request_entry::phase_0_negative_tests::phase_0_negative_production_boundaries';
const HOST_K0C =
  'host::request_entry::bytecode_http_tests::phase_1_request_lane_containment';
const HOST_PHASE1_PROOF =
  'host::request_entry::phase_1_runtime_proof_tests::phase_1_runtime_vcp_and_expected_red_obligations';

export const PHASE1_REQUIRED_LANES = Object.freeze([
  'K0A',
  'K0B',
  'K0C',
  'T-C',
  'T-R',
  'V1',
  'phase-0-regression',
]);

export function phase1WorkloadSpecs(root) {
  // Phase 0 regression is deliberately three accepted exact Rust proofs inside
  // this Gate's own receipts and hash closure. Nesting the Phase 0 selector
  // would require a second caller-designated absent durable root and would leave
  // that bundle outside the Phase 1 manifest closure.
  return Object.freeze([
    spec(root, 'gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-0-gate-*.test.mjs',
      'scripts/tests/bytecode-vm-phase-1-gate-*.test.mjs',
    ], 'node-tap', ['G1', 'phase-0-regression']),
    spec(root, 'k0a-compiler-admission', 'cargo', [
      'test', '-p', 'skiff-compiler', '--lib',
      'phase_1_bytecode_admission',
    ], 'rust-suite', ['K0A']),
    spec(root, 'k0a-emission-admission', 'cargo', [
      'test', '-p', 'skiff-compiler-emission', '--lib',
      'phase_1_bytecode_admission',
    ], 'rust-suite', ['K0A']),
    spec(root, 'k0b-tc-production-contract', 'cargo', [
      'test', '--manifest-path', 'runtime/linker/Cargo.toml', '--test',
      'phase_1_contract',
    ], 'rust-suite', ['K0B', 'T-C']),
    spec(root, 'k0c-request-containment', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_K0C,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['K0C']),
    spec(root, 'tr-v1-production-proof', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_PHASE1_PROOF,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['T-R', 'V1']),
    spec(root, 'phase0-production-composition-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_SUCCESS,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['phase-0-regression']),
    spec(root, 'phase0-production-boundaries-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_NEGATIVE,
      '--', '--exact', '--nocapture',
    ], 'rust-exact', ['phase-0-regression']),
    spec(root, 'phase0-request-scalar-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/request/Cargo.toml', '--test',
      'bytecode_request', 'tests::request_heap_scalar_returns_payload', '--', '--exact',
    ], 'rust-exact', ['phase-0-regression']),
  ]);
}

export function phase1CandidateSpecs(root) {
  return Object.freeze(['preflight', 'postflight', 'closure', 'fresh'].flatMap((phase) => [
    spec(root, `${phase}-head`, 'git', ['rev-parse', 'HEAD']),
    spec(root, `${phase}-tree`, 'git', ['rev-parse', 'HEAD^{tree}']),
    spec(root, `${phase}-status`, 'git', [
      'status', '--porcelain=v1', '--untracked-files=all',
    ]),
  ]));
}

export function assertPhase1LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = PHASE1_REQUIRED_LANES.filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 1 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
  }
}

export function parsePhase1TestSummary(format, output) {
  if (format === 'node-tap') return parseNodeTap(output);
  if (format === 'rust-exact') return parseRust(output, true);
  if (format === 'rust-suite') return parseRust(output, false);
  return null;
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

function parseNodeTap(output) {
  const field = (name) => uniqueInteger(output, new RegExp(`^# ${name} (\\d+)\\s*$`, 'gm'));
  const plans = [...output.matchAll(/^1\.\.(\d+)\s*$/gm)].map((match) => Number(match[1]));
  const counts = {
    total: field('tests'),
    passed: field('pass'),
    failed: field('fail'),
    cancelled: field('cancelled'),
    skipped: field('skipped'),
    todo: field('todo'),
  };
  const valid = plans.length === 1
    && Object.values(counts).every(Number.isSafeInteger)
    && counts.total > 0
    && plans[0] === counts.total
    && counts.passed === counts.total
    && counts.failed === 0
    && counts.cancelled === 0
    && counts.skipped === 0
    && counts.todo === 0;
  return { format: 'node-tap', declared: plans.length === 1 ? plans[0] : null, ...counts, valid };
}

function parseRust(output, exact) {
  const pattern = /^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out(?:;.*)?$/gm;
  const matches = [...output.matchAll(pattern)];
  if (matches.length !== 1) return rustSummary(null, null, null, null, null, false);
  const [, disposition, passedText, failedText, ignoredText, measuredText, filteredText] = matches[0];
  const [passed, failed, ignored, measured, filtered] = [
    passedText, failedText, ignoredText, measuredText, filteredText,
  ].map(Number);
  const valid = disposition === 'ok'
    && passed > 0
    && (!exact || passed === 1)
    && failed === 0
    && ignored === 0
    && measured === 0;
  return rustSummary(passed, failed, ignored, measured, filtered, valid);
}

function rustSummary(passed, failed, ignored, measured, filtered, valid) {
  return {
    format: 'rust', total: passed, passed, failed, ignored, measured, filtered, valid,
  };
}

function uniqueInteger(output, pattern) {
  const matches = [...output.matchAll(pattern)];
  return matches.length === 1 ? Number(matches[0][1]) : null;
}
