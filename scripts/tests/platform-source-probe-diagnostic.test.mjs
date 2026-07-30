import assert from 'node:assert/strict';
import test from 'node:test';

import {
  HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES,
  HOST_DIAGNOSTIC_MAX_ENTRIES,
  HOST_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES,
  assertHostDiagnosticMatchesOutcome,
  captureHostDiagnostic,
} from '../lib/platform-source-probe-diagnostic.mjs';

test('causal stderr outranks stdout shutdown without claiming cross-stream chronology', () => {
  const outcome = {
    code: 1,
    signal: null,
    error: null,
    stdout: [
      '[skiff-tests] phase startup: isolated-runtime',
      '[skiff-instance] stopping after startup failure',
    ].join('\n'),
    stderr: '[skiff-instance] supervisor failure: runtime exited before readiness',
  };

  const evidence = captureHostDiagnostic(outcome);

  assert.equal(evidence.phase, 'startup');
  assert.equal(evidence.subject, 'isolated-runtime');
  assert.deepEqual(
    evidence.diagnostics.map(({ kind, stream }) => ({ kind, stream })),
    [
      { kind: 'error', stream: 'stderr' },
      { kind: 'failure', stream: 'stdout' },
    ],
  );
  assert.equal(evidence.diagnosticOmittedCount, 0);
  assert.equal(Object.hasOwn(evidence.diagnostics[0], 'lineIndex'), false);
  assert.equal(Object.hasOwn(evidence.diagnostics[0], 'timestamp'), false);
  assertHostDiagnosticMatchesOutcome(evidence, outcome);
});

test('diagnostic collection is redacted before per-entry and aggregate bounding', () => {
  const privatePath = '/Users/private/worktree/runtime/runtime.yml';
  const secret = 'P5_F21A_SECRET_SENTINEL';
  const body = 'P5_F21A_HTTP_BODY_SENTINEL';
  const longTail = '雪'.repeat(700);
  const outcome = {
    code: 9,
    signal: null,
    error: null,
    stdout: '[skiff-tests] phase startup: isolated-runtime',
    stderr: [
      `error: first path=${privatePath} secret=${secret} ${longTail}`,
      `fatal: second request body: ${body}`,
      `error: third https://example.invalid/private?token=${secret} ${longTail}`,
      'error: fourth supporting diagnostic',
      'error: fifth supporting diagnostic',
    ].join('\n'),
  };

  const evidence = captureHostDiagnostic(outcome);
  const serialized = JSON.stringify(evidence);
  const totalBytes = evidence.diagnostics.reduce(
    (total, diagnostic) => total + Buffer.byteLength(diagnostic.sanitizedExcerpt),
    0,
  );

  assert.equal(evidence.diagnostics.length, HOST_DIAGNOSTIC_MAX_ENTRIES);
  assert.equal(evidence.diagnosticOmittedCount, 2);
  assert.equal(
    evidence.diagnostics.every((diagnostic) => (
      Buffer.byteLength(diagnostic.sanitizedExcerpt) <= HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES
      && /^[a-f0-9]{64}$/.test(diagnostic.originalLineSha256)
      && typeof diagnostic.truncated === 'boolean'
    )),
    true,
  );
  assert.ok(totalBytes <= HOST_DIAGNOSTIC_TOTAL_EXCERPT_MAX_BYTES);
  assert.match(serialized, /<PATH>/);
  assert.match(serialized, /<REDACTED_SECRET>/);
  assert.match(serialized, /<REDACTED_HTTP_BODY>/);
  assert.equal(serialized.includes(privatePath), false);
  assert.equal(serialized.includes(secret), false);
  assert.equal(serialized.includes(body), false);
  assert.equal(Object.hasOwn(evidence, 'stdout'), false);
  assert.equal(Object.hasOwn(evidence, 'stderr'), false);
  assertHostDiagnosticMatchesOutcome(evidence, outcome);
});

test('validator recomputes the production selection and rejects stale or reordered evidence', () => {
  const outcome = {
    code: 1,
    signal: null,
    error: null,
    stdout: '[skiff-instance] stopping after startup failure',
    stderr: [
      '[skiff-instance] supervisor failure: router exited before readiness',
      'error: supporting detail',
    ].join('\n'),
  };
  const evidence = captureHostDiagnostic(outcome);

  const reordered = structuredClone(evidence);
  reordered.diagnostics.reverse();
  assert.throws(
    () => assertHostDiagnosticMatchesOutcome(reordered, outcome),
    /does not match the original command outcome/,
  );

  const stale = {
    ...evidence,
    diagnostics: undefined,
    diagnosticOmittedCount: undefined,
    firstDiagnostic: evidence.diagnostics[0],
  };
  assert.throws(
    () => assertHostDiagnosticMatchesOutcome(stale, outcome),
    /does not match the original command outcome/,
  );
});
