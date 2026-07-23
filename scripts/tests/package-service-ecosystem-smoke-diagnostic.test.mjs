import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';

import { commandExecutionError } from '../lib/command-execution-internal.mjs';
import {
  captureFixtureCargoDiagnostic,
  FIXTURE_CARGO_DIAGNOSTIC_EXCERPT_MAX_BYTES,
  FIXTURE_CARGO_DIAGNOSTIC_MAX_ENTRIES,
  FIXTURE_CARGO_DIAGNOSTIC_PROPERTY,
  FIXTURE_CARGO_DIAGNOSTIC_SCHEMA_VERSION,
  FIXTURE_CARGO_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES,
  retainFixtureCargoDiagnostic,
} from '../lib/package-service-ecosystem-smoke-diagnostic.mjs';

const EMPTY_SHA256 = sha256('');

test('fixture Cargo diagnostic bounds multibyte output and redacts ANSI, paths, and secrets', () => {
  const secret = 'P5_F26A_SECRET_SENTINEL';
  const absolutePath = `/private/var/tmp/${secret}/fixture/main.skiff`;
  const firstLine =
    `\u001B[31merror: token=${secret} at ${absolutePath} ${'界'.repeat(400)}\u001B[0m`;
  const stderr = [
    firstLine,
    'warning: compilation stopped',
    'note: request body=PRIVATE_HTTP_BODY',
    'help: rerun with RUST_BACKTRACE=1',
  ].join('\n');
  const stdout = 'partial fixture output';
  const error = fixtureCargoError({ code: 101, stdout, stderr });
  for (const stream of ['stdout', 'stderr']) {
    assert.equal(Object.getOwnPropertyDescriptor(error, stream).enumerable, false);
  }

  assert.equal(retainFixtureCargoDiagnostic(error), error);
  const property = Object.getOwnPropertyDescriptor(
    error,
    FIXTURE_CARGO_DIAGNOSTIC_PROPERTY,
  );
  assert.equal(property.enumerable, true);
  assert.equal(property.writable, false);
  assert.equal(property.configurable, false);
  const evidence = error[FIXTURE_CARGO_DIAGNOSTIC_PROPERTY];
  assert.deepEqual(
    {
      schemaVersion: evidence.schemaVersion,
      command: evidence.command,
      phase: evidence.phase,
      code: evidence.code,
      signal: evidence.signal,
      stdoutBytes: evidence.stdoutBytes,
      stdoutSha256: evidence.stdoutSha256,
      stderrBytes: evidence.stderrBytes,
      stderrSha256: evidence.stderrSha256,
    },
    {
      schemaVersion: FIXTURE_CARGO_DIAGNOSTIC_SCHEMA_VERSION,
      command: 'cargo',
      phase: 'fixture-cargo',
      code: 101,
      signal: null,
      stdoutBytes: Buffer.byteLength(stdout),
      stdoutSha256: sha256(stdout),
      stderrBytes: Buffer.byteLength(stderr),
      stderrSha256: sha256(stderr),
    },
  );
  assert.equal(evidence.diagnostics.length, FIXTURE_CARGO_DIAGNOSTIC_MAX_ENTRIES);
  assert.equal(evidence.diagnosticOmittedCount, 2);
  assert.equal(evidence.diagnostics[0].stream, 'stderr');
  assert.equal(evidence.diagnostics[0].truncated, true);
  assert.equal(evidence.diagnostics[0].originalLineSha256, sha256(firstLine));
  assert.match(evidence.diagnostics[0].sanitizedExcerpt, /<PATH>/);
  assert.match(evidence.diagnostics[0].sanitizedExcerpt, /<REDACTED_SECRET>/);
  assert.doesNotMatch(evidence.diagnostics[0].sanitizedExcerpt, /\u001B/);
  assert.doesNotMatch(evidence.diagnostics[0].sanitizedExcerpt, /\uFFFD/);
  const totalExcerptBytes = evidence.diagnostics.reduce(
    (total, diagnostic) => total + Buffer.byteLength(diagnostic.sanitizedExcerpt),
    0,
  );
  assert.ok(totalExcerptBytes <= FIXTURE_CARGO_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES);
  for (const diagnostic of evidence.diagnostics) {
    assert.ok(
      Buffer.byteLength(diagnostic.sanitizedExcerpt)
        <= FIXTURE_CARGO_DIAGNOSTIC_EXCERPT_MAX_BYTES,
    );
  }

  const serialized = JSON.stringify(error);
  assert.equal(serialized.includes(secret), false);
  assert.equal(serialized.includes(absolutePath), false);
  assert.equal(serialized.includes('PRIVATE_HTTP_BODY'), false);
  assert.equal(serialized.includes(stderr), false);
});

test('fixture Cargo diagnostic handles empty stderr and silent outcomes', () => {
  const secret = 'P5_F26A_STDOUT_SECRET_SENTINEL';
  const stdout = `fatal: api_key=${secret} at C:\\private\\fixture\\main.rs`;
  const signalled = captureFixtureCargoDiagnostic(fixtureCargoError({
    code: null,
    signal: 'SIGTERM',
    stdout,
    stderr: '',
  }));
  assert.equal(signalled.code, null);
  assert.equal(signalled.signal, 'SIGTERM');
  assert.equal(signalled.stderrBytes, 0);
  assert.equal(signalled.stderrSha256, EMPTY_SHA256);
  assert.equal(signalled.diagnostics.length, 1);
  assert.equal(signalled.diagnostics[0].stream, 'stdout');
  assert.match(signalled.diagnostics[0].sanitizedExcerpt, /<REDACTED_SECRET>/);
  assert.match(signalled.diagnostics[0].sanitizedExcerpt, /<PATH>/);
  assert.equal(JSON.stringify(signalled).includes(secret), false);

  const silent = captureFixtureCargoDiagnostic(fixtureCargoError({
    code: 7,
    stdout: '',
    stderr: '',
  }));
  assert.equal(silent.stdoutSha256, EMPTY_SHA256);
  assert.equal(silent.stderrSha256, EMPTY_SHA256);
  assert.equal(silent.diagnostics.length, 1);
  assert.deepEqual(silent.diagnostics[0], {
    stream: 'none',
    sanitizedExcerpt: 'cargo exited with 7',
    originalLineSha256: sha256('cargo exited with 7'),
    truncated: false,
  });
  assert.equal(silent.diagnosticOmittedCount, 0);
});

function fixtureCargoError({
  code,
  signal = null,
  stdout,
  stderr,
}) {
  return commandExecutionError(
    'cargo',
    { code, signal, error: null },
    { stdout, stderr },
  );
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
