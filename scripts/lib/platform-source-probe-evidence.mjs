import { createHash } from 'node:crypto';
import { readFile, readdir, stat } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';

export {
  assertHostAttempt,
  beginHostAttempt,
  completeHostAttempt,
  failThrownHostAttempt,
  inspectHostFixture,
} from './platform-source-probe-host-evidence.mjs';
import { commandText, errorMessage } from './platform-source-probe-support.mjs';

export const PROBE_TARGETED_CRATES = Object.freeze([
  'skiff-test-runner', 'skiff-compiler', 'skiff-compiler-input', 'skiff-compiler-source',
]);

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
