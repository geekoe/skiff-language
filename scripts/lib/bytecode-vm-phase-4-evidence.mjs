import {
  assertPhase4LaneCoverage,
  PHASE4_MANIFEST_SCHEMA,
  phase4CandidateSpecs,
  phase4WorkloadSpecs,
  sha256,
  validSha256,
} from './bytecode-vm-phase-4-contract.mjs';
import { openPhase4EvidenceRoot } from './bytecode-vm-phase-4-evidence-root.mjs';
import { phase1ObservationSchemaIdentity } from './bytecode-vm-phase-1-observation-schema.mjs';
import {
  loadAndValidatePhase4CommandReceipts,
  writeJsonExclusive,
} from './bytecode-vm-phase-4-receipts.mjs';

const MANIFEST_NAME = 'manifest.json';

export async function finalizePhase4Evidence({
  evidenceRoot,
  repoRoot,
  expectedCommit,
  expectedTree,
  commandEnvironments,
  startedAt,
  finishedAt,
}) {
  await evidenceRoot.assertAll();
  const assessment = await deriveAssessment({
    evidenceRoot, repoRoot, expectedCommit, expectedTree, commandEnvironments,
  });
  const evidenceFiles = await snapshotPhase4EvidenceFiles(evidenceRoot);
  const directoryIdentities = evidenceRoot.identities();
  const manifest = {
    schemaVersion: PHASE4_MANIFEST_SCHEMA,
    request: {
      repoRoot,
      outputDir: evidenceRoot.outputDir,
      expectedCommit,
      expectedTree,
      directoryIdentities,
      startedAt,
      finishedAt,
    },
    // Phase 4 adds no observation kinds: the accepted Phase 1 eleven-event
    // schema is the recorded observation authority for this epoch. The single
    // publish/wake/claim cardinality is pinned by the K4 scheduler lanes and
    // the frozen owner inventory in the harness receipts.
    observationSchema: phase1ObservationSchemaIdentity(),
    ...assessment,
    evidenceFiles,
  };
  await writeJsonExclusive(evidenceRoot, MANIFEST_NAME, manifest);
  await evidenceRoot.assertAll();
  return manifest;
}

export async function checkPhase4Evidence(outputDir, request) {
  const evidenceRoot = await openPhase4EvidenceRoot(outputDir, request.directoryIdentities);
  await evidenceRoot.assertAll();
  const manifest = await readJson(evidenceRoot, MANIFEST_NAME, 'manifest');
  if (manifest?.schemaVersion !== PHASE4_MANIFEST_SCHEMA) {
    throw new Error(`manifest schemaVersion must be ${PHASE4_MANIFEST_SCHEMA}`);
  }
  const expectedObservationSchema = phase1ObservationSchemaIdentity();
  if (JSON.stringify(manifest.observationSchema) !== JSON.stringify(expectedObservationSchema)) {
    throw new Error('manifest observationSchema does not match the accepted observation schema');
  }
  const expectedRequest = {
    repoRoot: request.repoRoot,
    outputDir,
    expectedCommit: request.expectedCommit,
    expectedTree: request.expectedTree,
    directoryIdentities: request.directoryIdentities,
    startedAt: manifest.request?.startedAt,
    finishedAt: manifest.request?.finishedAt,
  };
  if (JSON.stringify(manifest.request) !== JSON.stringify(expectedRequest)) {
    throw new Error('manifest request does not match the Phase 4 Gate invocation');
  }
  const actualFiles = await snapshotPhase4EvidenceFiles(evidenceRoot);
  assertFileClosure(manifest.evidenceFiles, actualFiles);
  const assessment = await deriveAssessment({
    evidenceRoot,
    repoRoot: request.repoRoot,
    expectedCommit: request.expectedCommit,
    expectedTree: request.expectedTree,
    commandEnvironments: request.commandEnvironments,
  });
  for (const key of ['candidate', 'verdict', 'counts', 'commands', 'failures']) {
    if (JSON.stringify(manifest[key]) !== JSON.stringify(assessment[key])) {
      throw new Error(`manifest ${key} was not derived from command evidence`);
    }
  }
  await evidenceRoot.assertAll();
  return manifest;
}

async function deriveAssessment({
  evidenceRoot, repoRoot, expectedCommit, expectedTree, commandEnvironments,
}) {
  const workloads = phase4WorkloadSpecs(repoRoot);
  assertPhase4LaneCoverage(workloads);
  const specs = [
    ...phase4CandidateSpecs(repoRoot),
    ...workloads,
  ];
  const loaded = await loadAndValidatePhase4CommandReceipts(
    evidenceRoot,
    specs,
    commandEnvironments,
  );
  const failures = [...loaded.failures];
  const candidate = deriveCandidate(loaded.records, expectedCommit, expectedTree);
  if (!candidate.exact) failures.push(failure('candidate.stale', 'candidate commit or tree drifted'));
  if (!candidate.clean) failures.push(failure('candidate.dirty', 'candidate worktree was not clean'));
  const commands = specs.map((spec) => summarizeCommand(spec, loaded.records.get(spec.id)));
  const counts = summarizeCounts(commands);
  const uniqueFailures = deduplicate(failures);
  return {
    candidate,
    verdict: uniqueFailures.length === 0 ? 'PASS' : 'FAIL',
    counts,
    commands,
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
  const fresh = phase('fresh');
  return {
    expectedCommit,
    expectedTree,
    preflight,
    postflight,
    closure,
    fresh,
    exact: [preflight, postflight, closure, fresh]
      .every(({ commit, tree }) => commit === expectedCommit && tree === expectedTree),
    clean: [preflight, postflight, closure, fresh].every(({ status }) => status === ''),
  };
}

function commandText(record, trim = true) {
  if (record?.valid !== true || typeof record.stdout !== 'string') return null;
  return trim ? record.stdout.trim() : record.stdout;
}

function summarizeCommand(spec, record) {
  return {
    id: spec.id,
    lanes: [...spec.lanes],
    status: record?.valid === true ? 'PASS' : 'FAIL',
    outcome: record?.receipt?.outcome?.status ?? 'MISSING',
    testSummary: record?.testSummary ?? null,
    environment: record?.receipt?.identity?.environment ?? null,
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

export async function snapshotPhase4EvidenceFiles(evidenceRoot) {
  const records = [];
  for (const { path, bytes } of await evidenceRoot.snapshotFiles()) {
    if (path === MANIFEST_NAME) continue;
    records.push({ path, bytes: bytes.length, sha256: sha256(bytes) });
  }
  return records.sort((left, right) => left.path.localeCompare(right.path));
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

async function readJson(evidenceRoot, path, label) {
  try {
    return JSON.parse(await evidenceRoot.readFile(path, 'utf8'));
  } catch (error) {
    throw new Error(`${label} is missing or invalid: ${error?.code ?? error?.message}`);
  }
}
