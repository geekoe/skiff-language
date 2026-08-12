import { createHash } from 'node:crypto';

export const PHASE0_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-0-command-v3';
export const PHASE0_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-0-gate-v4';

const GIT_OBJECT = /^[a-f0-9]{40}$/;
const HOST_SUCCESS =
  'host::request_entry::phase_0_vcp_tests::phase_0_vcp_production_composition';
const HOST_NEGATIVE =
  'host::request_entry::phase_0_negative_tests::phase_0_negative_production_boundaries';

export function phase0WorkloadSpecs(root) {
  return Object.freeze([
    spec(root, 'gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-0-gate-*.test.mjs',
    ], 'node-tap'),
    spec(root, 'host-production-composition', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_SUCCESS,
      '--', '--exact', '--nocapture',
    ], 'rust-exact'),
    spec(root, 'host-production-boundaries', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib', HOST_NEGATIVE,
      '--', '--exact', '--nocapture',
    ], 'rust-exact'),
    spec(root, 'request-scalar-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/request/Cargo.toml', '--test',
      'bytecode_request', 'tests::request_heap_scalar_returns_payload', '--', '--exact',
    ], 'rust-exact'),
    spec(root, 'request-typed-json-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/request/Cargo.toml', '--test',
      'bytecode_request', 'tests::typed_json_number_body_materializes_against_pinned_entry',
      '--', '--exact',
    ], 'rust-exact'),
    spec(root, 'request-raw-http-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/request/Cargo.toml', '--test',
      'bytecode_request', 'tests::raw_http_body_remains_heap_bytes', '--', '--exact',
    ], 'rust-exact'),
    spec(root, 'vm-scalar-vertical-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/vm/Cargo.toml', '--test', 'vertical',
      'tests::source_to_vm_scalar_tail_call_executes_through_the_verified_entry',
      '--', '--exact',
    ], 'rust-exact'),
    spec(root, 'host-mode-containment-regression', 'cargo', [
      'test', '--manifest-path', 'runtime/host/Cargo.toml', '--lib',
      'host::request_entry::bytecode_http_tests::canonical_http_server_stream_with_scalar_operation_fails_closed',
      '--', '--exact',
    ], 'rust-exact'),
  ]);
}

export function phase0CandidateSpecs(root) {
  return Object.freeze(['preflight', 'postflight', 'closure'].flatMap((phase) => [
    spec(root, `${phase}-head`, 'git', ['rev-parse', 'HEAD']),
    spec(root, `${phase}-tree`, 'git', ['rev-parse', 'HEAD^{tree}']),
    spec(root, `${phase}-status`, 'git', [
      'status', '--porcelain=v1', '--untracked-files=all',
    ]),
  ]));
}

export function phase0FreshCandidateSpecs(root) {
  return Object.freeze([
    spec(root, 'fresh-head', 'git', ['rev-parse', 'HEAD']),
    spec(root, 'fresh-tree', 'git', ['rev-parse', 'HEAD^{tree}']),
    spec(root, 'fresh-status', 'git', ['status', '--porcelain=v1', '--untracked-files=all']),
  ]);
}

function spec(cwd, id, command, args, testFormat = null) {
  return Object.freeze({
    id,
    command,
    args: Object.freeze(args),
    cwd,
    testFormat,
  });
}

export function parseTestSummary(format, output) {
  if (format === 'node-tap') return parseNodeTap(output);
  if (format === 'rust-exact') return parseRustExact(output);
  return null;
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

function parseRustExact(output) {
  const pattern = /^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out(?:;.*)?$/gm;
  const matches = [...output.matchAll(pattern)];
  if (matches.length !== 1) return rustSummary(null, null, null, null, null, false);
  const [, disposition, passed, failed, ignored, measured, filtered] = matches[0];
  return rustSummary(
    Number(passed), Number(failed), Number(ignored), Number(measured), Number(filtered),
    disposition === 'ok' && Number(passed) === 1 && Number(failed) === 0
      && Number(ignored) === 0 && Number(measured) === 0,
  );
}

function rustSummary(passed, failed, ignored, measured, filtered, valid) {
  return {
    format: 'rust-exact', total: passed, passed, failed, ignored, measured, filtered, valid,
  };
}

function uniqueInteger(output, pattern) {
  const matches = [...output.matchAll(pattern)];
  return matches.length === 1 ? Number(matches[0][1]) : null;
}

export function assertGitObject(value, label) {
  if (!GIT_OBJECT.test(value ?? '')) {
    throw new Error(`${label} must be a 40-character lowercase Git object identity`);
  }
  return value;
}

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

export function snapshotCommandEnvironment(environment) {
  return Object.freeze(Object.fromEntries(
    Object.entries(environment ?? {})
      .filter(([, value]) => typeof value === 'string')
      .sort(([left], [right]) => left.localeCompare(right)),
  ));
}

export function commandEnvironmentIdentity(environment) {
  const variables = Object.entries(snapshotCommandEnvironment(environment)).map(([name, value]) => ({
    name,
    bytes: Buffer.byteLength(value),
    valueSha256: sha256(value),
  }));
  return {
    variables,
    sha256: sha256(JSON.stringify(variables)),
  };
}

export function validSha256(value) {
  return /^[a-f0-9]{64}$/.test(value ?? '');
}
