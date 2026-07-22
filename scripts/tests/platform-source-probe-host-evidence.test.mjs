import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';

import {
  beginHostAttempt,
  completeHostAttempt,
  inspectHostFixture,
} from '../lib/platform-source-probe-host-evidence.mjs';

const assertionPath = '/owned/b/test-runner/fixtures/package-service-host/consumer/main.test.skiff';
const testName = 'provider observes helper mutation';
const finalValue = 'provider-observed-helper-mutated';
const actualPassLine = `PASS main.__test::${testName}`;
const validFixtureSource = [
  `test "${testName}" {`,
  `  assert root.main.run() == "${finalValue}"`,
  '}',
  '',
].join('\n');
const v6HostAttemptFields = [
  'code',
  'command',
  'counts',
  'diagnosticOmittedCount',
  'diagnostics',
  'error',
  'exactPassLineCount',
  'expectedPassLine',
  'firstIssue',
  'issues',
  'observedPassLine',
  'outputSha256',
  'passLines',
  'phase',
  'portEvidencePresent',
  'processEvidencePresent',
  'resultLines',
  'signal',
  'sourceSuite',
  'status',
  'stderrBytes',
  'stderrSha256',
  'stdoutBytes',
  'stdoutSha256',
  'subject',
];

test('fixture guard owns the exact test and sole reachable final-value assertion', () => {
  assert.deepEqual(inspectHostFixture(validFixtureSource, assertionPath), {
    assertionPath,
    assertion: `assert root.main.run() == "${finalValue}"`,
    testName,
    expectedFinalValue: finalValue,
  });

  for (const source of [
    validFixtureSource.replace(testName, 'wrong observation'),
    validFixtureSource.replace(finalValue, 'wrong-value'),
    validFixtureSource.replace('}', '  assert true\n}'),
    validFixtureSource.replace('assert root.main.run()', 'if false { assert root.main.run()'),
  ]) {
    assert.throws(() => inspectHostFixture(source, assertionPath));
  }
});

test('actual runtime PASS identity and fixture assertion jointly project final value', () => {
  const unrelated = 'PASS std.crypto.__test::hashes a value';
  const outcome = hostOutcome([unrelated, actualPassLine]);
  const attempt = beginHostAttempt('node', ['/owned/b/scripts/run-skiff-tests.mjs']);
  const completed = completeHostAttempt(attempt, outcome, fixture(), evidencePresent);

  assert.equal(completed.status, 'PASS');
  assert.equal(completed.firstIssue, null);
  assert.equal(completed.expectedPassLine, `PASS <runtime-module-path>::${testName}`);
  assert.equal(completed.observedPassLine, actualPassLine);
  assert.equal(completed.exactPassLineCount, 1);
  assert.deepEqual(completed.sourceSuite, {
    std: { passed: 11, total: 11 },
    host: { passed: 1, total: 1 },
    finalValue,
    finalValueEvidence: {
      passLine: actualPassLine,
      assertionPath,
      assertion: `assert root.main.run() == "${finalValue}"`,
    },
  });
  assert.deepEqual(completed.passLines, [
    `PASS <unexpected sha256:${sha256(unrelated)}>`,
    actualPassLine,
  ]);
  assert.equal(JSON.stringify(completed).includes(unrelated), false);
  assert.deepEqual(Object.keys(attempt).sort(), v6HostAttemptFields);
  assert.deepEqual(Object.keys(completed).sort(), v6HostAttemptFields);
});

test('an alternate runtime module is accepted when it is the sole Host identity', () => {
  const alternatePassLine = `PASS nested.main.__test::${testName}`;
  const completed = completeHostAttempt(
    beginHostAttempt('node', ['/owned/b/scripts/run-skiff-tests.mjs']),
    hostOutcome([alternatePassLine]),
    fixture(),
    evidencePresent,
  );

  assert.equal(completed.status, 'PASS');
  assert.equal(completed.exactPassLineCount, 1);
  assert.equal(completed.observedPassLine, alternatePassLine);
  assert.deepEqual(completed.passLines, [alternatePassLine]);
  assert.equal(completed.sourceSuite.finalValueEvidence.passLine, alternatePassLine);
});

test('same-name identities outside the Host segment make a correct Host identity ambiguous', async (t) => {
  const cases = [
    ['before std', { beforeStd: [actualPassLine] }],
    ['after Host', { afterHost: [actualPassLine] }],
    ['stderr', { stderrLines: [actualPassLine] }],
  ];
  for (const [name, outcomeOverrides] of cases) {
    await t.test(name, () => {
      const completed = completeHostAttempt(
        beginHostAttempt('node', ['/owned/b/scripts/run-skiff-tests.mjs']),
        hostOutcome([actualPassLine], outcomeOverrides),
        fixture(),
        evidencePresent,
      );

      assert.equal(completed.status, 'FAIL');
      assert.equal(completed.firstIssue.kind, 'pass-line');
      assert.equal(completed.exactPassLineCount, 2);
      assert.equal(completed.observedPassLine, null);
      assert.equal(completed.sourceSuite, null);
      assert.equal(completed.passLines.length, 2);
      assert.equal(completed.passLines.every(isUnexpectedPassToken), true);
    });
  }
});

test('wrong, missing, duplicate, malformed, illegal, and oversized identities fail closed', async (t) => {
  const cases = [
    ['wrong test name', ['PASS main.__test::wrong observation'], 'pass-line', {}],
    ['missing target', ['PASS std.crypto.__test::hashes a value'], 'pass-line', {}],
    ['std cannot impersonate Host', [], 'pass-line', { beforeStd: [actualPassLine] }],
    ['duplicate target', [actualPassLine, actualPassLine], 'pass-line', {}],
    ['duplicate across modules', [actualPassLine, `PASS nested.main.__test::${testName}`], 'pass-line', {}],
    ['empty module', [`PASS ::${testName}`], 'pass-line-format', {}],
    ['illegal module', [`PASS main/module::__test::${testName}`], 'pass-line-format', {}],
    ['non ASCII module', [`PASS mäin.__test::${testName}`], 'pass-line-format', {}],
    ['malformed delimiter', [`PASS main.__test:${testName}`], 'pass-line-format', {}],
    ['oversized', [`PASS main.__test::${'x'.repeat(600)}`], 'pass-line-format', {}],
  ];
  for (const [name, passLines, expectedIssue, outcomeOverrides] of cases) {
    await t.test(name, () => {
      const completed = completeHostAttempt(
        beginHostAttempt('node', ['/owned/b/scripts/run-skiff-tests.mjs']),
        hostOutcome(passLines, outcomeOverrides),
        fixture(),
        evidencePresent,
      );
      assert.equal(completed.status, 'FAIL');
      assert.equal(completed.sourceSuite, null);
      assert.equal(completed.firstIssue.kind, expectedIssue);
      assert.equal(completed.passLines.every(isUnexpectedPassToken), true);
    });
  }
});

test('command outcome, exact result counts, process evidence, and port evidence are mandatory', async (t) => {
  const cases = [
    ['nonzero', { outcome: { code: 7 }, evidence: evidencePresent }, 'command-outcome'],
    ['signal', { outcome: { code: null, signal: 'SIGTERM' }, evidence: evidencePresent }, 'command-outcome'],
    ['wrong std count', { outcome: { std: 'test result: ok. 10 passed; 0 failed' }, evidence: evidencePresent }, 'result-counts'],
    ['wrong Host count', { outcome: { host: 'test result: ok. 0 passed; 1 failed' }, evidence: evidencePresent }, 'result-counts'],
    ['extra result', { outcome: { extra: 'test result: ok. 1 passed; 0 failed' }, evidence: evidencePresent }, 'result-counts'],
    ['missing process', { outcome: {}, evidence: { ...evidencePresent, processEvidencePresent: false } }, 'missing-process-evidence'],
    ['missing port', { outcome: {}, evidence: { ...evidencePresent, portEvidencePresent: false } }, 'missing-port-evidence'],
  ];
  for (const [name, { outcome: overrides, evidence }, expectedIssue] of cases) {
    await t.test(name, () => {
      const completed = completeHostAttempt(
        beginHostAttempt('node', ['/owned/b/scripts/run-skiff-tests.mjs']),
        hostOutcome([actualPassLine], overrides),
        fixture(),
        evidence,
      );
      assert.equal(completed.status, 'FAIL');
      assert.equal(completed.sourceSuite, null);
      assert.equal(completed.firstIssue.kind, expectedIssue);
    });
  }
});

function fixture() {
  return inspectHostFixture(validFixtureSource, assertionPath);
}

function hostOutcome(passLines, {
  code = 0,
  signal = null,
  std = 'test result: ok. 11 passed; 0 failed',
  host = 'test result: ok. 1 passed; 0 failed',
  extra = null,
  beforeStd = [],
  afterHost = [],
  stderrLines = [],
} = {}) {
  return {
    code,
    signal,
    error: null,
    stdout: [
      ...beforeStd,
      std,
      ...passLines,
      host,
      ...afterHost,
      ...(extra === null ? [] : [extra]),
    ].join('\n'),
    stderr: stderrLines.join('\n'),
  };
}

function isUnexpectedPassToken(line) {
  return /^PASS <unexpected sha256:[a-f0-9]{64}>$/.test(line);
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

const evidencePresent = Object.freeze({
  processEvidencePresent: true,
  portEvidencePresent: true,
});
