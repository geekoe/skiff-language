import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  RUST_CLIPPY_BASELINE_ARGS,
  runRustClippyBaselineCheck,
} from '../lib/rust-clippy-baseline-check.mjs';
import {
  analyzeClippyRun,
  assertTooManyLinesBaselineMatches,
  collectTooManyLinesFindings,
  parseCargoJsonMessages,
  parseTooManyLinesBaseline,
} from '../lib/rust-clippy-baseline.mjs';

test('extracts stable path + Rust item identities and deduplicates repeated target diagnostics', () => {
  const diagnostic = cargoDiagnostic({
    path: 'src/service.rs',
    line: 40,
    source: 'pub async fn handle_request(\n    input: Request,\n) -> Result<Response> {',
  });
  const findings = collectTooManyLinesFindings([
    diagnostic,
    { ...diagnostic, target: { name: 'crate_name_test' } },
  ], { root: '/repo' });
  assert.deepEqual(findings, [{ path: 'src/service.rs', item: 'fn handle_request' }]);
  assert.equal(Object.hasOwn(findings[0], 'line'), false);
});

test('same path + item at different primary spans is an identity collision', () => {
  assert.throws(
    () => collectTooManyLinesFindings([
      cargoDiagnostic({ path: 'src/service.rs', line: 40, source: 'fn repeated() {' }),
      cargoDiagnostic({ path: 'src/service.rs', line: 240, source: 'fn repeated() {' }),
    ], { root: '/repo' }),
    /identity collision.*src\/service\.rs :: fn repeated.*:40:1.*:240:1/,
  );
});

test('unexpected and stale too_many_lines identities both fail with actionable differences', () => {
  assert.throws(
    () => assertTooManyLinesBaselineMatches(
      [{ path: 'src/new.rs', item: 'fn new_debt' }],
      [],
    ),
    /unexpected finding\(s\):\n\+ src\/new\.rs :: fn new_debt/,
  );
  assert.throws(
    () => assertTooManyLinesBaselineMatches(
      [],
      [{ path: 'src/old.rs', item: 'fn removed_debt' }],
    ),
    /stale baseline entry\/entries \(remove them\):\n- src\/old\.rs :: fn removed_debt/,
  );
});

test('baseline schema requires unique sorted repository-relative item identities', () => {
  const valid = {
    version: 1,
    lint: 'clippy::too_many_lines',
    entries: [
      { path: 'src/a.rs', item: 'fn alpha' },
      { path: 'src/b.rs', item: 'fn beta' },
    ],
  };
  assert.deepEqual(parseTooManyLinesBaseline(valid).entries, valid.entries);
  assert.throws(
    () => parseTooManyLinesBaseline({ ...valid, entries: [...valid.entries].reverse() }),
    /must be sorted/,
  );
  assert.throws(
    () => parseTooManyLinesBaseline({ ...valid, entries: [valid.entries[0], valid.entries[0]] }),
    /must have unique/,
  );
  assert.throws(
    () => parseTooManyLinesBaseline({
      ...valid,
      entries: [{ path: '../outside.rs', item: 'fn outside' }],
    }),
    /invalid repository-relative path/,
  );
});

test('Cargo JSON parsing and hard diagnostics fail closed', () => {
  assert.throws(() => parseCargoJsonMessages('{not json}\n'), /invalid JSON.*line 1/);
  assert.throws(
    () => analyzeClippyRun(cargoOutcome({ code: 0, stdout: jsonLines([
      cargoDiagnostic({
        code: 'clippy::never_loop',
        level: 'error',
        path: 'src/lib.rs',
        source: 'loop { break; }',
      }),
    ]) }), { root: '/repo' }),
    /hard diagnostic.*clippy::never_loop/s,
  );
});

test('checker uses the canonical workspace Clippy command and rejects nonzero Cargo', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-rust-clippy-baseline-'));
  const baselinePath = join(fixture, 'baseline.json');
  const calls = [];
  try {
    await writeFile(baselinePath, JSON.stringify({
      version: 1,
      lint: 'clippy::too_many_lines',
      entries: [],
    }));
    const passing = await runRustClippyBaselineCheck({
      root: fixture,
      baselinePath,
      env: { SENTINEL: 'yes' },
      captureCommand: async (command, args, options) => {
        calls.push({ command, args, options });
        return cargoOutcome({ code: 0 });
      },
    });
    assert.equal(passing.findings.length, 0);
    assert.deepEqual(calls, [{
      command: 'cargo',
      args: [...RUST_CLIPPY_BASELINE_ARGS],
      options: { cwd: fixture, env: { SENTINEL: 'yes' } },
    }]);

    await assert.rejects(
      runRustClippyBaselineCheck({
        root: fixture,
        baselinePath,
        captureCommand: async () => cargoOutcome({
          code: 101,
          stderr: 'could not compile workspace',
        }),
      }),
      /cargo clippy exited with exit code 101.*could not compile workspace/s,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

function cargoDiagnostic({
  code = 'clippy::too_many_lines',
  level = 'warning',
  path,
  line = 1,
  source,
}) {
  return {
    reason: 'compiler-message',
    target: { name: 'crate_name' },
    message: {
      code: { code },
      level,
      message: code,
      spans: [{
        file_name: path,
        line_start: line,
        line_end: line + source.split('\n').length - 1,
        column_start: 1,
        column_end: 2,
        is_primary: true,
        text: source.split('\n').map((text) => ({ text })),
      }],
    },
  };
}

function cargoOutcome({ code, stdout = '', stderr = '', signal = null, error = null }) {
  return { code, stdout, stderr, signal, error };
}

function jsonLines(messages) {
  return `${messages.map((message) => JSON.stringify(message)).join('\n')}\n`;
}
