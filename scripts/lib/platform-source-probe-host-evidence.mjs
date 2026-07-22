import { createHash } from 'node:crypto';

import {
  assertHostDiagnosticMatchesOutcome,
  captureHostDiagnostic,
} from './platform-source-probe-diagnostic.mjs';
import { commandText, errorMessage } from './platform-source-probe-support.mjs';

const HOST_TEST_NAME = 'provider observes helper mutation';
const HOST_EXPECTED_FINAL_VALUE = 'provider-observed-helper-mutated';
const HOST_ASSERTION_PATTERN = /^assert root\.main\.run\(\) == "([^"]+)"$/;
const HOST_RESULT_PATTERN = /^test result: ok\. (\d+) passed; (\d+) failed$/;
const HOST_PASS_LINE_MAX_BYTES = 512;
const HOST_MODULE_PATTERN = '[A-Za-z0-9_-]+(?:\\.[A-Za-z0-9_-]+)*';
const HOST_PASS_PATTERN = new RegExp(`^PASS (${HOST_MODULE_PATTERN})::([\\x20-\\x7e]+)$`);

export function inspectHostFixture(source, assertionPath) {
  const lines = source
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  const expectedHeader = `test "${HOST_TEST_NAME}" {`;
  if (lines.length !== 3 || lines[0] !== expectedHeader || lines[2] !== '}') {
    throw new Error('Host fixture must contain one reachable assertion and no alternate pass path');
  }
  const assertion = HOST_ASSERTION_PATTERN.exec(lines[1]);
  if (assertion === null || assertion[1] !== HOST_EXPECTED_FINAL_VALUE) {
    throw new Error(`Host fixture must assert ${HOST_EXPECTED_FINAL_VALUE} exactly once`);
  }
  return {
    assertionPath,
    assertion: lines[1],
    testName: HOST_TEST_NAME,
    expectedFinalValue: assertion[1],
  };
}

export function beginHostAttempt(command, args) {
  return {
    status: 'RUNNING',
    command: { executable: command, args: [...args] },
    code: null,
    signal: null,
    error: null,
    phase: 'unknown',
    subject: 'unknown',
    stdoutBytes: null,
    stderrBytes: null,
    diagnostics: [],
    diagnosticOmittedCount: null,
    stdoutSha256: null,
    stderrSha256: null,
    outputSha256: null,
    resultLines: [],
    counts: [],
    passLines: [],
    expectedPassLine: null,
    observedPassLine: null,
    exactPassLineCount: 0,
    processEvidencePresent: false,
    portEvidencePresent: false,
    sourceSuite: null,
    issues: [],
    firstIssue: null,
  };
}

export function failThrownHostAttempt(attempt, error) {
  const diagnostic = captureHostDiagnostic({ error, stdout: '', stderr: '' });
  const issue = {
    kind: 'command-throw',
    message: `full Host command threw before returning an outcome: ${errorMessage(error)}`,
  };
  return {
    ...attempt,
    ...diagnostic,
    status: 'FAIL',
    error: errorMessage(error),
    issues: [issue],
    firstIssue: issue,
  };
}

export function completeHostAttempt(attempt, outcome, fixture, {
  processEvidencePresent,
  portEvidencePresent,
}) {
  const stdout = outcome?.stdout ?? '';
  const stderr = outcome?.stderr ?? '';
  const output = commandText(outcome ?? {});
  const parsed = parseHostOutput(stdout, stderr, fixture.testName);
  const diagnostic = captureHostDiagnostic(outcome);
  const issues = hostEvidenceIssues(outcome, parsed, fixture.testName, {
    processEvidencePresent,
    portEvidencePresent,
  });
  const sourceSuite = issues.length === 0 ? projectSourceSuite(parsed, fixture) : null;
  const completed = {
    ...attempt,
    ...diagnostic,
    status: issues.length === 0 ? 'PASS' : 'FAIL',
    code: Number.isInteger(outcome?.code) ? outcome.code : null,
    signal: outcome?.signal ?? null,
    error: outcome?.error == null ? null : errorMessage(outcome.error),
    stdoutSha256: sha256(stdout),
    stderrSha256: sha256(stderr),
    outputSha256: sha256(output),
    resultLines: parsed.resultLines.map((line) => (
      HOST_RESULT_PATTERN.test(line) ? line : 'test result: <invalid>'
    )),
    counts: parsed.counts,
    passLines: parsed.passLines.map(storedPassLine),
    expectedPassLine: `PASS <runtime-module-path>::${fixture.testName}`,
    observedPassLine: parsed.observedPassLine,
    exactPassLineCount: parsed.matchingPassLines.length,
    processEvidencePresent,
    portEvidencePresent,
    sourceSuite,
    issues,
    firstIssue: issues[0] ?? null,
  };
  assertHostDiagnosticMatchesOutcome(completed, outcome);
  return completed;
}

export function assertHostAttempt(attempt) {
  if (attempt.status !== 'PASS' || attempt.firstIssue !== null) {
    throw new Error(attempt.firstIssue?.message ?? 'full Host attempt failed');
  }
}

function parsePassIdentity(line) {
  if (Buffer.byteLength(line, 'utf8') > HOST_PASS_LINE_MAX_BYTES) {
    return { line, valid: false, testName: null };
  }
  const match = HOST_PASS_PATTERN.exec(line);
  if (match === null || match[2].trim() !== match[2]) {
    return { line, valid: false, testName: null };
  }
  return { line, valid: true, testName: match[2] };
}

function parseHostOutput(stdout, stderr, testName) {
  const stdoutLines = stdout.split(/\r?\n/);
  const resultEntries = stdoutLines
    .map((line, index) => ({ line, index }))
    .filter((entry) => entry.line.startsWith('test result:'));
  const resultLines = resultEntries.map((entry) => entry.line);
  const counts = resultLines.map((line) => {
    const match = HOST_RESULT_PATTERN.exec(line);
    return match === null ? null : { passed: Number(match[1]), failed: Number(match[2]) };
  });
  const inHostResultSegment = (stream, index) => stream === 'stdout'
    && resultEntries.length === 2
    && index > resultEntries[0].index
    && index < resultEntries[1].index;
  const passLines = [
    ...stdoutLines.map((line, index) => ({ line, index, stream: 'stdout' })),
    ...stderr.split(/\r?\n/).map((line, index) => ({ line, index, stream: 'stderr' })),
  ]
    .filter((entry) => entry.line.startsWith('PASS'))
    .map((entry) => {
      const parsed = parsePassIdentity(entry.line);
      return {
        ...parsed,
        inHostResultSegment: inHostResultSegment(entry.stream, entry.index),
        matchesTestName: parsed.valid && parsed.testName === testName,
      };
    });
  const matchingPassLines = passLines.filter((entry) => entry.matchesTestName);
  const observedPassLineEntry = matchingPassLines.length === 1
    && matchingPassLines[0].inHostResultSegment
    ? matchingPassLines[0]
    : null;
  return {
    resultLines,
    counts,
    passLines: passLines.map((entry) => ({
      ...entry,
      retainActual: entry === observedPassLineEntry,
    })),
    matchingPassLines,
    observedPassLine: observedPassLineEntry?.line ?? null,
  };
}

function hostEvidenceIssues(outcome, parsed, testName, evidence) {
  const issues = [];
  if (outcome?.error != null || outcome?.signal != null || outcome?.code !== 0) {
    issues.push({
      kind: 'command-outcome',
      message: `full Host command failed (${outcome?.signal ?? outcome?.code ?? 'spawn'})`,
    });
  }
  if (!evidence.processEvidencePresent) {
    issues.push({
      kind: 'missing-process-evidence',
      message: 'full gate omitted owned process cleanup evidence',
    });
  }
  if (!evidence.portEvidencePresent) {
    issues.push({
      kind: 'missing-port-evidence',
      message: 'full gate omitted observed port cleanup evidence',
    });
  }
  if (!hasExactResultCounts(parsed)) {
    issues.push({
      kind: 'result-counts',
      message: `full gate must report exact std 11/11 and Host 1/1; observed ${parsed.resultLines.length} result line(s)`,
    });
  }
  const invalidPassLineCount = parsed.passLines.filter((entry) => !entry.valid).length;
  if (invalidPassLineCount !== 0) {
    issues.push({
      kind: 'pass-line-format',
      message: `full gate observed ${invalidPassLineCount} malformed or oversized PASS line(s)`,
    });
  }
  if (parsed.matchingPassLines.length !== 1 || parsed.observedPassLine === null) {
    issues.push({
      kind: 'pass-line',
      message: `full gate must report exactly one syntax-valid PASS identity for test "${testName}" across all output, in the stdout Host result segment`,
    });
  }
  return issues;
}

function hasExactResultCounts(parsed) {
  return parsed.resultLines.length === 2
    && parsed.counts[0]?.passed === 11
    && parsed.counts[0]?.failed === 0
    && parsed.counts[1]?.passed === 1
    && parsed.counts[1]?.failed === 0;
}

function projectSourceSuite(parsed, fixture) {
  return {
    std: { passed: parsed.counts[0].passed, total: parsed.counts[0].passed },
    host: { passed: parsed.counts[1].passed, total: parsed.counts[1].passed },
    finalValue: fixture.expectedFinalValue,
    finalValueEvidence: {
      passLine: parsed.observedPassLine,
      assertionPath: fixture.assertionPath,
      assertion: fixture.assertion,
    },
  };
}

function storedPassLine(entry) {
  return entry.valid && entry.retainActual
    ? entry.line
    : `PASS <unexpected sha256:${sha256(entry.line)}>`;
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
