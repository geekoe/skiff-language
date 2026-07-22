import { createHash } from 'node:crypto';
import { readFile, readdir, stat } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';

import {
  assertHostDiagnosticMatchesOutcome,
  captureHostDiagnostic,
} from './platform-source-probe-diagnostic.mjs';
import { commandText, errorMessage } from './platform-source-probe-support.mjs';

export const PROBE_TARGETED_CRATES = Object.freeze([
  'skiff-test-runner', 'skiff-compiler', 'skiff-compiler-input', 'skiff-compiler-source',
]);

const HOST_TEST_NAME = 'provider observes helper mutation';
const HOST_EXPECTED_FINAL_VALUE = 'provider-observed-helper-mutated';
const HOST_ASSERTION_PATTERN = /^assert root\.main\.run\(\) == "([^"]+)"$/;
const HOST_RESULT_PATTERN = /^test result: ok\. (\d+) passed; (\d+) failed$/;

export async function snapshotProbeArtifacts(targetRoot) {
  const debugRoot = join(targetRoot, 'debug');
  const files = await walkFiles(debugRoot);
  const records = [];
  for (const path of files.sort()) {
    const traits = artifactTraits(path, debugRoot);
    if (traits === null) continue;
    const metadata = await stat(path);
    const contents = await readFile(path);
    records.push({
      path,
      sha256: sha256(contents),
      mtimeMs: metadata.mtimeMs,
      size: metadata.size,
      ...traits,
      ...(traits.classification === 'root-specific-dep-info'
        ? { materializationText: contents.toString('utf8') }
        : {}),
    });
  }
  return records;
}

export function artifactSnapshotForLedger(snapshot) {
  return snapshot.map(({ materializationText: _contents, ...entry }) => ({ ...entry }));
}

export function createArtifactEvidence({
  mode,
  label,
  outcome,
  before,
  after,
  sourceRoot,
  targetRoot,
  requireIdentity,
}) {
  if (mode !== 'combined' && mode !== 'full') {
    throw new Error(`unknown artifact evidence mode: ${mode}`);
  }
  const cargo = cargoEvidence(outcome);
  const fresh = freshEvidence(cargo.targetedLines);
  const diff = artifactDiff({ mode, before, after, sourceRoot, targetRoot });
  const identityTargetPresent = before.some((entry) => entry.identityTest === true)
    && after.some((entry) => entry.identityTest === true);
  const issues = [];
  if (cargo.error !== null || cargo.signal !== null || cargo.code !== 0) {
    issues.push({
      kind: 'cargo-outcome',
      path: null,
      message: `${label} Cargo command failed (${cargo.signal ?? cargo.code ?? 'spawn'})`,
    });
  }
  for (const crate of fresh.missing) {
    issues.push({
      kind: 'missing-fresh',
      path: null,
      message: `${label} omitted Fresh crate evidence: ${crate}`,
    });
  }
  for (const conflict of fresh.conflicts) {
    issues.push({
      kind: 'conflicting-cargo-unit',
      path: null,
      message: `${label} reported ${conflict.state} for Fresh unit ${conflict.crate}`,
    });
  }
  if (requireIdentity && !identityTargetPresent) {
    issues.push({
      kind: 'missing-identity-target',
      path: null,
      message: `${label} omitted the identity integration-test artifact`,
    });
  }
  for (const entry of diff.entries.filter((item) => item.allowed !== true)) {
    issues.push({
      kind: 'artifact-diff',
      path: entry.path,
      classification: entry.classification,
      before: entry.before,
      after: entry.after,
      message: `${label} changed ${entry.classification} artifact: ${entry.path}`,
    });
  }
  if (mode === 'full' && diff.rootMaterializations.length === 0) {
    issues.push({
      kind: 'missing-root-materialization',
      path: null,
      message: `${label} omitted exact A-to-B top-level dep-info materialization`,
    });
  }
  return {
    mode,
    label,
    comparator: mode === 'combined'
      ? 'strict-stable-artifact-v1'
      : 'full-root-materialization-v1',
    before: artifactSnapshotForLedger(before),
    cargo,
    after: artifactSnapshotForLedger(after),
    diff,
    fresh,
    identityTargetRequired: requireIdentity,
    identityTargetPresent,
    verdict: issues.length === 0 ? 'PASS' : 'FAIL',
    issues,
    firstIssue: issues[0] ?? null,
  };
}

export function assertArtifactEvidence(evidence) {
  if (evidence.verdict !== 'PASS' || evidence.firstIssue !== null) {
    throw new Error(evidence.firstIssue?.message ?? `${evidence.label} artifact evidence failed`);
  }
}

export function combinedArtifactEvidenceIsComplete(evidence) {
  return Array.isArray(evidence)
    && evidence.length === 2
    && JSON.stringify(evidence.map((entry) => entry.label))
      === JSON.stringify(['A-origin/B-root', 'B-origin/A-root'])
    && evidence.every((entry) => {
      const canonicalFresh = freshEvidence(entry?.cargo?.targetedLines ?? []);
      const identityTargetPresent = entry?.before?.some((artifact) => artifact.identityTest === true)
        && entry?.after?.some((artifact) => artifact.identityTest === true);
      return (
        entry?.mode === 'combined'
        && entry.comparator === 'strict-stable-artifact-v1'
        && entry.verdict === 'PASS'
        && entry.firstIssue === null
        && entry.issues?.length === 0
        && Array.isArray(entry.before)
        && entry.before.length > 0
        && entry.before.every(storedArtifactIsValid)
        && Array.isArray(entry.after)
        && entry.after.length > 0
        && entry.after.every(storedArtifactIsValid)
        && JSON.stringify(entry.before) === JSON.stringify(entry.after)
        && entry.diff?.entries?.length === 0
        && entry.diff?.rootMaterializations?.length === 0
        && entry.diff?.changedCount === 0
        && entry.diff?.allowedCount === 0
        && entry.diff?.disallowedCount === 0
        && entry.diff?.firstDisallowed === null
        && JSON.stringify(entry.fresh) === JSON.stringify(canonicalFresh)
        && entry.identityTargetRequired === true
        && entry.identityTargetPresent === identityTargetPresent
        && identityTargetPresent === true
        && entry.cargo?.code === 0
        && entry.cargo?.signal === null
        && entry.cargo?.error === null
        && /^[a-f0-9]{64}$/.test(entry.cargo?.outputSha256)
      );
    });
}

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
    expectedPassLine: `PASS main.test.skiff::${HOST_TEST_NAME}`,
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
  const outputLines = output.split(/\r?\n/).map((line) => line.trim());
  const resultLines = outputLines.filter((line) => line.startsWith('test result:'));
  const passLines = outputLines.filter((line) => line.startsWith('PASS '));
  const parsedCounts = resultLines.map((line) => {
    const match = HOST_RESULT_PATTERN.exec(line);
    return match === null ? null : { passed: Number(match[1]), failed: Number(match[2]) };
  });
  const matchingPassLines = outputLines.filter((line) => line === fixture.expectedPassLine);
  const exactPassLineCount = matchingPassLines.length;
  const observedPassLine = exactPassLineCount === 1 ? matchingPassLines[0] : null;
  const observedFinalValue = observedPassLine === null ? null : fixture.expectedFinalValue;
  const diagnostic = captureHostDiagnostic(outcome);
  const issues = [];
  if (outcome?.error != null || outcome?.signal != null || outcome?.code !== 0) {
    issues.push({
      kind: 'command-outcome',
      message: `full Host command failed (${outcome?.signal ?? outcome?.code ?? 'spawn'})`,
    });
  }
  if (!processEvidencePresent) {
    issues.push({
      kind: 'missing-process-evidence',
      message: 'full gate omitted owned process cleanup evidence',
    });
  }
  if (!portEvidencePresent) {
    issues.push({
      kind: 'missing-port-evidence',
      message: 'full gate omitted observed port cleanup evidence',
    });
  }
  if (
    resultLines.length !== 2
    || parsedCounts.some((entry) => entry === null)
    || parsedCounts[0]?.passed !== 11
    || parsedCounts[0]?.failed !== 0
    || parsedCounts[1]?.passed !== 1
    || parsedCounts[1]?.failed !== 0
  ) {
    issues.push({
      kind: 'result-counts',
      message: `full gate must report exact std 11/11 and Host 1/1; observed ${resultLines.length} result line(s)`,
    });
  }
  if (exactPassLineCount !== 1) {
    issues.push({
      kind: 'pass-line',
      message: `full gate must report ${fixture.expectedPassLine} exactly once`,
    });
  }
  const sourceSuite = issues.length === 0 ? {
    std: { passed: parsedCounts[0].passed, total: parsedCounts[0].passed },
    host: { passed: parsedCounts[1].passed, total: parsedCounts[1].passed },
    finalValue: observedFinalValue,
    finalValueEvidence: {
      passLine: observedPassLine,
      assertionPath: fixture.assertionPath,
      assertion: fixture.assertion,
    },
  } : null;
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
    resultLines: resultLines.map((line) => (
      HOST_RESULT_PATTERN.test(line) ? line : 'test result: <invalid>'
    )),
    counts: parsedCounts,
    passLines: passLines.map((line) => (
      line === fixture.expectedPassLine ? line : 'PASS <unexpected>'
    )),
    expectedPassLine: fixture.expectedPassLine,
    observedPassLine,
    exactPassLineCount,
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

function cargoEvidence(outcome) {
  const stdout = outcome?.stdout ?? '';
  const stderr = outcome?.stderr ?? '';
  const output = commandText(outcome ?? {});
  return {
    code: Number.isInteger(outcome?.code) ? outcome.code : null,
    signal: outcome?.signal ?? null,
    error: outcome?.error == null ? null : errorMessage(outcome.error),
    stdoutSha256: sha256(stdout),
    stderrSha256: sha256(stderr),
    outputSha256: sha256(output),
    targetedLines: targetedCargoLines(output),
  };
}

function targetedCargoLines(output) {
  const lines = [];
  for (const original of output.split(/\r?\n/)) {
    const line = original.trim();
    const match = /^(Fresh|Dirty|Compiling)\s+([^\s]+)/.exec(line);
    if (match !== null && PROBE_TARGETED_CRATES.includes(match[2])) {
      lines.push({ state: match[1], crate: match[2], line });
    }
  }
  return lines;
}

function freshEvidence(lines) {
  const freshCrates = PROBE_TARGETED_CRATES.filter((crate) => (
    lines.some((line) => line.crate === crate && line.state === 'Fresh')
  ));
  return {
    requiredCrates: [...PROBE_TARGETED_CRATES],
    freshCrates,
    missing: PROBE_TARGETED_CRATES.filter((crate) => !freshCrates.includes(crate)),
    conflicts: lines.filter((line) => line.state !== 'Fresh'),
  };
}

function artifactDiff({ mode, before, after, sourceRoot, targetRoot }) {
  const beforeByPath = new Map(before.map((entry) => [entry.path, entry]));
  const afterByPath = new Map(after.map((entry) => [entry.path, entry]));
  const paths = [...new Set([...beforeByPath.keys(), ...afterByPath.keys()])].sort();
  const entries = [];
  for (const path of paths) {
    const beforeEntry = beforeByPath.get(path) ?? null;
    const afterEntry = afterByPath.get(path) ?? null;
    if (artifactEntriesEqual(beforeEntry, afterEntry)) continue;
    const classification = beforeEntry?.classification
      ?? afterEntry?.classification
      ?? 'unknown';
    const materialization = mode === 'full'
      && classification === 'root-specific-dep-info'
      && beforeEntry !== null
      && afterEntry !== null
      ? rootMaterialization(beforeEntry, afterEntry, sourceRoot, targetRoot)
      : null;
    entries.push({
      path,
      classification,
      change: beforeEntry === null ? 'added' : afterEntry === null ? 'removed' : 'modified',
      allowed: materialization?.exact === true,
      before: publicArtifact(beforeEntry),
      after: publicArtifact(afterEntry),
      rootMaterialization: materialization,
    });
  }
  const rootMaterializations = entries
    .filter((entry) => entry.rootMaterialization?.exact === true)
    .map((entry) => ({ path: entry.path, ...entry.rootMaterialization }));
  return {
    entries,
    changedCount: entries.length,
    allowedCount: entries.filter((entry) => entry.allowed === true).length,
    disallowedCount: entries.filter((entry) => entry.allowed !== true).length,
    firstDisallowed: entries.find((entry) => entry.allowed !== true) ?? null,
    rootMaterializations,
  };
}

function rootMaterialization(before, after, sourceRoot, targetRoot) {
  const beforeText = before.materializationText;
  const afterText = after.materializationText;
  const rootsValid = typeof sourceRoot === 'string'
    && typeof targetRoot === 'string'
    && sourceRoot.length > 0
    && targetRoot.length > 0
    && sourceRoot !== targetRoot;
  if (!rootsValid || typeof beforeText !== 'string' || typeof afterText !== 'string') {
    return {
      exact: false,
      sourceRoot,
      targetRoot,
      sourceOccurrencesBefore: 0,
      targetOccurrencesBefore: 0,
      sourceOccurrencesAfter: 0,
      targetOccurrencesAfter: 0,
      replacedSha256: null,
      afterSha256: after.sha256,
    };
  }
  const sourceOccurrencesBefore = countOccurrences(beforeText, sourceRoot);
  const targetOccurrencesBefore = countOccurrences(beforeText, targetRoot);
  const sourceOccurrencesAfter = countOccurrences(afterText, sourceRoot);
  const targetOccurrencesAfter = countOccurrences(afterText, targetRoot);
  const replaced = beforeText.split(sourceRoot).join(targetRoot);
  return {
    exact: sourceOccurrencesBefore > 0
      && targetOccurrencesBefore === 0
      && sourceOccurrencesAfter === 0
      && targetOccurrencesAfter === sourceOccurrencesBefore
      && replaced === afterText,
    sourceRoot,
    targetRoot,
    sourceOccurrencesBefore,
    targetOccurrencesBefore,
    sourceOccurrencesAfter,
    targetOccurrencesAfter,
    replacedSha256: sha256(replaced),
    afterSha256: after.sha256,
  };
}

function artifactEntriesEqual(left, right) {
  if (left === null || right === null) return left === right;
  return left.path === right.path
    && left.sha256 === right.sha256
    && left.mtimeMs === right.mtimeMs
    && left.size === right.size
    && left.classification === right.classification
    && left.depInfo === right.depInfo
    && left.structureSubject === right.structureSubject
    && left.identityTest === right.identityTest
    && left.materializationText === right.materializationText;
}

function publicArtifact(entry) {
  if (entry === null) return null;
  const { materializationText: _contents, ...publicEntry } = entry;
  return { ...publicEntry };
}

function storedArtifactIsValid(entry) {
  return typeof entry?.path === 'string'
    && entry.path.length > 0
    && /^[a-f0-9]{64}$/.test(entry.sha256)
    && Number.isFinite(entry.mtimeMs)
    && Number.isInteger(entry.size)
    && entry.size >= 0
    && ['root-specific-dep-info', 'hashed-dep-info', 'binary', 'rlib', 'identity-test']
      .includes(entry.classification)
    && typeof entry.depInfo === 'boolean'
    && typeof entry.structureSubject === 'boolean'
    && typeof entry.identityTest === 'boolean'
    && !Object.hasOwn(entry, 'materializationText');
}

function artifactTraits(path, debugRoot) {
  const name = basename(path);
  const depInfo = name.endsWith('.d')
    && /(?:skiff[-_](?:compiler|test[-_]runner|package[-_]service[-_]smoke[-_]fixture)|package_service_contract_deployment)/
      .test(name);
  if (depInfo) {
    return {
      classification: dirname(path) === debugRoot
        ? 'root-specific-dep-info'
        : 'hashed-dep-info',
      depInfo: true,
      structureSubject: false,
      identityTest: false,
    };
  }
  const structureSubject = name === 'skiff-compiler'
    || name === 'skiff-test-runner'
    || name === 'skiff-package-service-smoke-fixture'
    || /^libskiff_compiler(?:_input|_source)?-[^.]+\.rlib$/.test(name);
  const identityTest = /^package_service_contract_deployment-[^.]+$/.test(name);
  if (!structureSubject && !identityTest) return null;
  return {
    classification: identityTest
      ? 'identity-test'
      : name.endsWith('.rlib') ? 'rlib' : 'binary',
    depInfo: false,
    structureSubject,
    identityTest,
  };
}

async function walkFiles(root) {
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') return [];
    throw error;
  }
  const files = [];
  for (const entry of entries) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...await walkFiles(path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

function countOccurrences(value, needle) {
  return value.split(needle).length - 1;
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
