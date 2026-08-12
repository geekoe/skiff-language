import { lstat, readFile, readdir } from 'node:fs/promises';
import { join, relative, sep } from 'node:path';

import {
  PHASE0_MANIFEST_SCHEMA,
  phase0CandidateSpecs,
  phase0WorkloadSpecs,
  sha256,
  validSha256,
} from './bytecode-vm-phase-0-contract.mjs';
import {
  loadAndValidateCommandReceipts,
  writeJsonExclusive,
} from './bytecode-vm-phase-0-receipts.mjs';

const MANIFEST_NAME = 'manifest.json';

export async function finalizePhase0Evidence({
  outputDir,
  repoRoot,
  expectedCommit,
  expectedTree,
  transcriptPaths,
  startedAt,
  finishedAt,
}) {
  const assessment = await deriveAssessment({
    outputDir, repoRoot, expectedCommit, expectedTree, transcriptPaths,
  });
  const evidenceFiles = await snapshotEvidenceFiles(outputDir);
  const manifest = {
    schemaVersion: PHASE0_MANIFEST_SCHEMA,
    request: { repoRoot, outputDir, expectedCommit, expectedTree, startedAt, finishedAt },
    ...assessment,
    evidenceFiles,
  };
  await writeJsonExclusive(join(outputDir, MANIFEST_NAME), manifest);
  return manifest;
}

export async function checkPhase0Evidence(outputDir, request) {
  const manifest = await readJson(join(outputDir, MANIFEST_NAME), 'manifest');
  if (manifest?.schemaVersion !== PHASE0_MANIFEST_SCHEMA) {
    throw new Error(`manifest schemaVersion must be ${PHASE0_MANIFEST_SCHEMA}`);
  }
  const expectedRequest = {
    repoRoot: request.repoRoot,
    outputDir,
    expectedCommit: request.expectedCommit,
    expectedTree: request.expectedTree,
    startedAt: manifest.request?.startedAt,
    finishedAt: manifest.request?.finishedAt,
  };
  if (JSON.stringify(manifest.request) !== JSON.stringify(expectedRequest)) {
    throw new Error('manifest request does not match the Gate invocation');
  }
  const actualFiles = await snapshotEvidenceFiles(outputDir);
  assertFileClosure(manifest.evidenceFiles, actualFiles);
  const assessment = await deriveAssessment({
    outputDir,
    repoRoot: request.repoRoot,
    expectedCommit: request.expectedCommit,
    expectedTree: request.expectedTree,
    transcriptPaths: request.transcriptPaths,
  });
  for (const key of ['candidate', 'verdict', 'counts', 'commands', 'transcripts', 'failures']) {
    if (JSON.stringify(manifest[key]) !== JSON.stringify(assessment[key])) {
      throw new Error(`manifest ${key} was not derived from command evidence`);
    }
  }
  return manifest;
}

async function deriveAssessment({
  outputDir, repoRoot, expectedCommit, expectedTree, transcriptPaths,
}) {
  const specs = [
    ...phase0CandidateSpecs(repoRoot),
    ...phase0WorkloadSpecs(repoRoot, transcriptPaths),
  ];
  const loaded = await loadAndValidateCommandReceipts(outputDir, specs);
  const failures = [...loaded.failures];
  const candidate = deriveCandidate(loaded.records, expectedCommit, expectedTree);
  if (!candidate.exact) failures.push(failure('candidate.stale', 'candidate commit or tree drifted'));
  if (!candidate.clean) failures.push(failure('candidate.dirty', 'candidate worktree was not clean'));
  const transcripts = [];
  for (const [id, path] of Object.entries(transcriptPaths ?? {})) {
    const record = await inspectTranscript(outputDir, id, path);
    transcripts.push(record);
    if (record.error !== null) failures.push(failure('transcript.invalid', `${id}: ${record.error}`));
  }
  const commands = specs.map((spec) => summarizeCommand(spec, loaded.records.get(spec.id)));
  const counts = summarizeCounts(commands);
  const uniqueFailures = deduplicate(failures);
  return {
    candidate,
    verdict: uniqueFailures.length === 0 ? 'PASS' : 'FAIL',
    counts,
    commands,
    transcripts,
    failures: uniqueFailures,
  };
}

function deriveCandidate(records, expectedCommit, expectedTree) {
  const phase = (name) => ({
    commit: commandText(records.get(`${name}-head`)),
    tree: commandText(records.get(`${name}-tree`)),
    status: commandText(records.get(`${name}-status`), false),
  });
  const preflight = phase('preflight');
  const postflight = phase('postflight');
  const closure = phase('closure');
  return {
    expectedCommit,
    expectedTree,
    preflight,
    postflight,
    closure,
    exact: [preflight, postflight, closure]
      .every(({ commit, tree }) => commit === expectedCommit && tree === expectedTree),
    clean: [preflight, postflight, closure].every(({ status }) => status === ''),
  };
}

function commandText(record, trim = true) {
  if (record?.valid !== true || typeof record.stdout !== 'string') return null;
  return trim ? record.stdout.trim() : record.stdout;
}

function summarizeCommand(spec, record) {
  return {
    id: spec.id,
    status: record?.valid === true ? 'PASS' : 'FAIL',
    outcome: record?.receipt?.outcome?.status ?? 'MISSING',
    testSummary: record?.testSummary ?? null,
    receipt: `commands/${spec.id}.receipt.json`,
  };
}

function summarizeCounts(commands) {
  const summaries = commands.map(({ testSummary }) => testSummary).filter(Boolean);
  return {
    commands: {
      total: commands.length,
      passed: commands.filter(({ status }) => status === 'PASS').length,
      failed: commands.filter(({ status }) => status !== 'PASS').length,
    },
    tests: {
      declared: summaries.reduce((sum, item) => sum + (item.total ?? 0), 0),
      passed: summaries.reduce((sum, item) => sum + (item.passed ?? 0), 0),
      failed: summaries.reduce((sum, item) => sum + (item.failed ?? 0), 0),
      skipped: summaries.reduce((sum, item) => sum + (item.skipped ?? 0), 0),
      todo: summaries.reduce((sum, item) => sum + (item.todo ?? 0), 0),
      cancelled: summaries.reduce((sum, item) => sum + (item.cancelled ?? 0), 0),
      ignored: summaries.reduce((sum, item) => sum + (item.ignored ?? 0), 0),
    },
  };
}

async function inspectTranscript(outputDir, id, path) {
  const expectedPrefix = `${outputDir}${sep}`;
  if (typeof path !== 'string' || !path.startsWith(expectedPrefix)) {
    return { id, path, present: false, bytes: 0, sha256: null, error: 'path is not inside evidence output' };
  }
  try {
    const metadata = await lstat(path);
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      return { id, path, present: true, bytes: 0, sha256: null, error: 'not a regular file' };
    }
    const bytes = await readFile(path);
    if (bytes.length === 0) {
      return { id, path, present: true, bytes: 0, sha256: sha256(bytes), error: 'file is empty' };
    }
    return { id, path, present: true, bytes: bytes.length, sha256: sha256(bytes), error: null };
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return { id, path, present: false, bytes: 0, sha256: null, error: null };
    }
    return { id, path, present: false, bytes: 0, sha256: null, error: error?.code ?? error?.message };
  }
}

export async function snapshotEvidenceFiles(outputDir) {
  const files = [];
  await walk(outputDir, files);
  const records = [];
  for (const absolute of files) {
    const path = relative(outputDir, absolute).split(sep).join('/');
    if (path === MANIFEST_NAME) continue;
    const bytes = await readFile(absolute);
    records.push({ path, bytes: bytes.length, sha256: sha256(bytes) });
  }
  return records.sort((left, right) => left.path.localeCompare(right.path));
}

async function walk(root, files) {
  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const path = join(root, entry.name);
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink()) throw new Error(`evidence contains symlink ${path}`);
    if (metadata.isDirectory()) await walk(path, files);
    else if (metadata.isFile()) files.push(path);
    else throw new Error(`evidence contains non-regular entry ${path}`);
  }
}

function assertFileClosure(stored, actual) {
  if (!Array.isArray(stored)
    || stored.some(({ path, bytes, sha256: digest }) => typeof path !== 'string'
      || !Number.isSafeInteger(bytes) || bytes < 0 || !validSha256(digest))
    || JSON.stringify(stored) !== JSON.stringify(actual)) {
    throw new Error('durable evidence file hash closure does not match bundle');
  }
}

function failure(code, message) {
  return { code, message };
}

function deduplicate(failures) {
  const seen = new Set();
  return failures.filter((entry) => {
    const key = JSON.stringify(entry);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

async function readJson(path, label) {
  try {
    return JSON.parse(await readFile(path, 'utf8'));
  } catch (error) {
    throw new Error(`${label} is missing or invalid: ${error?.code ?? error?.message}`);
  }
}
