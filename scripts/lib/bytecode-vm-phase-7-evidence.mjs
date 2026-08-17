import {
  PHASE7_CATALOG_SCHEMA,
  PHASE7_COVERAGE_ROWS,
  PHASE7_HANDOFF_SCHEMA,
  PHASE7_MANIFEST_SCHEMA,
  PHASE7_EPOCH,
  assertPhase7LaneCoverage,
  assertPhase7ProvenanceCoverage,
  phase7AdapterCatalog,
  phase7CapabilityLedger,
  phase7CandidateSpecs,
  phase7CoverageMap,
  phase7ExpectedTestsIdentity,
  phase7ExecutionOrder,
  phase7SpecCatalog,
  phase7SpecCatalogDigest,
  phase7WorkloadProvenance,
  phase7WorkloadSpecs,
  sha256,
  validSha256,
} from './bytecode-vm-phase-7-contract.mjs';
import { openPhase7EvidenceRoot } from './bytecode-vm-phase-7-evidence-root.mjs';
import { phase7IdentityRecord } from './bytecode-vm-phase-7-identity-probe.mjs';
import {
  loadAndValidatePhase7CommandReceipts,
  writeJsonExclusive,
} from './bytecode-vm-phase-7-receipts.mjs';
import { phase6BoundedWorkLedger } from './bytecode-vm-phase-6-contract.mjs';

const MANIFEST_NAME = 'manifest.json';
const CATALOG_NAME = 'catalog.json';
const HANDOFF_NAME = 'handoff.json';
const OBSERVATIONS_DIR = 'observations/';

export async function finalizePhase7Evidence({
  evidenceRoot,
  repoRoot,
  expectedCommit,
  expectedTree,
  commandEnvironments,
  startedAt,
  finishedAt,
}) {
  await evidenceRoot.assertAll();
  const specCatalogDigest = phase7SpecCatalogDigest(repoRoot);
  const assessment = await deriveAssessment({
    evidenceRoot,
    repoRoot,
    expectedCommit,
    expectedTree,
    commandEnvironments,
    specCatalogDigest,
  });
  const identities = await phase7IdentityRecord(repoRoot);
  const handoff = deriveHandoff(repoRoot, identities);
  const observations = deriveObservations(repoRoot, assessment.records);
  await writeJsonExclusive(evidenceRoot, CATALOG_NAME, phase7SpecCatalog(repoRoot));
  await writeJsonExclusive(evidenceRoot, HANDOFF_NAME, handoff);
  for (const observation of observations) {
    await writeJsonExclusive(
      evidenceRoot,
      `${OBSERVATIONS_DIR}${observation.row}-${observation.identity}.json`,
      observation,
    );
  }
  const evidenceFiles = await snapshotPhase7EvidenceFiles(evidenceRoot);
  const manifest = {
    schemaVersion: PHASE7_MANIFEST_SCHEMA,
    request: {
      repoRoot,
      outputDir: evidenceRoot.outputDir,
      expectedCommit,
      expectedTree,
      directoryIdentities: evidenceRoot.identities(),
      epoch: PHASE7_EPOCH,
      startedAt,
      finishedAt,
    },
    catalogDigest: specCatalogDigest,
    identities,
    handoffDigest: sha256(JSON.stringify(handoff)),
    ...assessment.assessment,
    evidenceFiles,
  };
  await writeJsonExclusive(evidenceRoot, MANIFEST_NAME, manifest);
  await evidenceRoot.assertAll();
  return manifest;
}

export async function checkPhase7Evidence(outputDir, request) {
  const evidenceRoot = await openPhase7EvidenceRoot(outputDir, request.directoryIdentities);
  await evidenceRoot.assertAll();
  const manifest = await readJson(evidenceRoot, MANIFEST_NAME, 'manifest');
  if (manifest?.schemaVersion !== PHASE7_MANIFEST_SCHEMA) {
    throw new Error(`manifest schemaVersion must be ${PHASE7_MANIFEST_SCHEMA}`);
  }
  const expectedRequest = {
    repoRoot: request.repoRoot,
    outputDir,
    expectedCommit: request.expectedCommit,
    expectedTree: request.expectedTree,
    directoryIdentities: request.directoryIdentities,
    epoch: PHASE7_EPOCH,
    startedAt: manifest.request?.startedAt,
    finishedAt: manifest.request?.finishedAt,
  };
  if (JSON.stringify(manifest.request) !== JSON.stringify(expectedRequest)) {
    throw new Error('manifest request does not match the Phase 7 r1 Gate invocation');
  }
  const failures = [];
  const specCatalogDigest = phase7SpecCatalogDigest(request.repoRoot);
  if (manifest.catalogDigest !== specCatalogDigest) {
    failures.push(failure('catalog.cross-epoch', 'spec/provenance catalog digest changed'));
  }
  const storedCatalog = await readJson(evidenceRoot, CATALOG_NAME, 'catalog');
  if (storedCatalog?.schemaVersion !== PHASE7_CATALOG_SCHEMA
    || JSON.stringify(storedCatalog) !== JSON.stringify(phase7SpecCatalog(request.repoRoot))) {
    failures.push(failure('catalog.cross-epoch', 'catalog.json does not match the canonical catalog'));
  }
  const identities = await phase7IdentityRecord(request.repoRoot);
  if (JSON.stringify(manifest.identities) !== JSON.stringify(identities)) {
    failures.push(failure('identity.drift', 'dynamic production identities drifted'));
  }
  const handoff = deriveHandoff(request.repoRoot, identities);
  if (manifest.handoffDigest !== sha256(JSON.stringify(handoff))) {
    failures.push(failure('handoff.drift', 'handoff.json does not match the canonical handoff'));
  }
  const storedHandoff = await readJson(evidenceRoot, HANDOFF_NAME, 'handoff');
  if (storedHandoff?.schemaVersion !== PHASE7_HANDOFF_SCHEMA
    || JSON.stringify(storedHandoff) !== JSON.stringify(handoff)) {
    failures.push(failure('handoff.drift', 'handoff.json does not match the canonical handoff'));
  }
  const assessment = await deriveAssessment({
    evidenceRoot,
    repoRoot: request.repoRoot,
    expectedCommit: request.expectedCommit,
    expectedTree: request.expectedTree,
    commandEnvironments: request.commandEnvironments,
    specCatalogDigest,
  });
  for (const key of ['candidate', 'verdict', 'counts', 'commands', 'coverage', 'chain']) {
    if (JSON.stringify(manifest[key]) !== JSON.stringify(assessment.assessment[key])) {
      failures.push(failure('assessment.rederive',
        `manifest ${key} was not derived from command evidence`));
    }
  }
  if (JSON.stringify(manifest.failures) !== JSON.stringify(assessment.assessment.failures)) {
    failures.push(failure('assessment.rederive', 'manifest failures were not derived from evidence'));
  }
  const expectedObservations = deriveObservations(request.repoRoot, assessment.records);
  for (const observation of expectedObservations) {
    const stored = await readJson(
      evidenceRoot,
      `${OBSERVATIONS_DIR}${observation.row}-${observation.identity}.json`,
      `observation ${observation.row}`,
    ).catch(() => null);
    if (JSON.stringify(stored) !== JSON.stringify(observation)) {
      failures.push(failure('observation.drift',
        `observation ${observation.row} does not match the derived coverage`));
    }
  }
  const closureFailures = checkFileClosure(
    manifest.evidenceFiles,
    await snapshotPhase7EvidenceFiles(evidenceRoot),
    await expectedEvidencePaths(evidenceRoot, request.repoRoot),
  );
  failures.push(...closureFailures);
  failures.push(...assessment.assessment.failures);
  await evidenceRoot.assertAll();
  return {
    verdict: failures.length === 0 ? 'PASS' : 'FAIL',
    failures: deduplicate(failures),
    manifest,
  };
}

async function deriveAssessment({
  evidenceRoot,
  repoRoot,
  expectedCommit,
  expectedTree,
  commandEnvironments,
  specCatalogDigest,
}) {
  const workloads = phase7WorkloadSpecs(repoRoot);
  assertPhase7CatalogShapes(workloads, repoRoot);
  const specs = [
    ...phase7CandidateSpecs(repoRoot),
    ...workloads,
  ];
  const loaded = await loadAndValidatePhase7CommandReceipts(
    evidenceRoot,
    specs,
    commandEnvironments,
    {
      order: phase7ExecutionOrder(repoRoot),
      expectedCommit,
      expectedTree,
      specCatalogDigest,
    },
  );
  const failures = [...loaded.failures];
  const genesis = await readGenesisReceipt(evidenceRoot);
  if (genesis?.specCatalogDigest !== specCatalogDigest) {
    failures.push(failure('catalog.cross-epoch',
      'genesis receipt binds a different spec/provenance catalog digest'));
  }
  const candidate = deriveCandidate(loaded.records, expectedCommit, expectedTree);
  if (!candidate.exact) failures.push(failure('candidate.stale', 'candidate commit or tree drifted'));
  if (!candidate.clean) failures.push(failure('candidate.dirty', 'candidate worktree was not clean'));
  const commands = specs.map((spec) => summarizeCommand(spec, loaded.records.get(spec.id)));
  const counts = summarizeCounts(commands);
  const coverage = deriveCoverage(repoRoot, loaded.records);
  const chain = loaded.chain;
  const uniqueFailures = deduplicate(failures);
  return {
    records: loaded.records,
    assessment: {
      candidate,
      verdict: uniqueFailures.length === 0 ? 'PASS' : 'FAIL',
      counts,
      commands,
      coverage,
      chain: {
        receipts: loaded.chain,
        head: loaded.chain.at(-1)?.digest ?? null,
      },
      failures: uniqueFailures,
    },
  };
}

function assertPhase7CatalogShapes(workloads, root) {
  assertPhase7LaneCoverage(workloads);
  const provenance = phase7WorkloadProvenance(root);
  assertPhase7ProvenanceCoverage(workloads, provenance);
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
  const status = record?.receipt?.outcome?.status ?? 'MISSING';
  return {
    id: spec.id,
    lanes: [...spec.lanes],
    expectedTests: phase7ExpectedTestsIdentity(spec),
    sourcePhase: spec.sourcePhase,
    sourceId: spec.sourceId,
    parentPhase: spec.parentPhase ?? null,
    parentId: spec.parentId ?? null,
    originChain: spec.originChain,
    status,
    outcome: status,
    blockedBy: record?.receipt?.outcome?.blockedBy ?? null,
    testSummary: record?.testSummary ?? null,
    environment: record?.receipt?.identity?.environment ?? null,
    receipt: `commands/${record?.receipt?.sequence ?? 0}-${spec.id}.receipt.json`,
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

function deriveCoverage(root, records) {
  const coverage = phase7CoverageMap(root);
  const rows = {};
  for (const row of PHASE7_COVERAGE_ROWS) {
    const specIds = coverage[row];
    const status = specIds.every((id) => records.get(id)?.receipt?.outcome?.status === 'PASS')
      ? 'PASS'
      : 'FAIL';
    rows[row] = { specIds, status };
  }
  return { rows };
}

function deriveObservations(root, records) {
  const coverage = phase7CoverageMap(root);
  return PHASE7_COVERAGE_ROWS.map((row) => {
    const specIds = [...coverage[row]];
    const identity = sha256(JSON.stringify({ row, specIds }));
    const outcome = specIds.every((id) => records.get(id)?.receipt?.outcome?.status === 'PASS')
      ? 'PASS'
      : 'FAIL';
    return { row, identity, specIds, outcome };
  });
}

function deriveHandoff(root, identities) {
  const workloads = phase7WorkloadSpecs(root);
  const historical = workloads.filter(({ sourcePhase }) => sourcePhase < 6);
  const originalState = new Map(
    phase7AdapterCatalog(root).rows.map((row) => [row.id, row.originalState]),
  );
  const residual = {
    inherited: historical.length,
    missing: historical.filter(({ id }) => originalState.get(id) === 'missing').length,
    null: historical.filter(({ id }) => originalState.get(id) === null).length,
    integer: historical.filter(({ id }) =>
      Number.isInteger(originalState.get(id))).length,
  };
  return {
    schemaVersion: PHASE7_HANDOFF_SCHEMA,
    epoch: PHASE7_EPOCH,
    upstream: {
      phase: 6,
      cumulativeExport: 'phase6WorkloadSpecs',
      provenanceExport: 'phase6WorkloadProvenance',
    },
    capabilityLedger: phase7CapabilityLedger(root),
    boundedWorkLedger: phase6BoundedWorkLedger(root),
    residualInventory: residual,
    identitiesDigest: identities.digest,
  };
}

export async function snapshotPhase7EvidenceFiles(evidenceRoot) {
  const records = [];
  for (const { path, bytes } of await evidenceRoot.snapshotFiles()) {
    if (path === MANIFEST_NAME) continue;
    records.push({ path, bytes: bytes.length, sha256: sha256(bytes) });
  }
  return records.sort((left, right) => left.path.localeCompare(right.path));
}

async function expectedEvidencePaths(evidenceRoot, root) {
  const identityFile = evidenceRoot.identityFile;
  const paths = new Set([
    identityFile,
    CATALOG_NAME,
    HANDOFF_NAME,
    'commands/0-genesis.receipt.json',
  ]);
  const order = phase7ExecutionOrder(root);
  order.forEach((id, index) => {
    const sequence = index + 1;
    paths.add(`commands/${sequence}-${id}.receipt.json`);
    paths.add(`commands/${sequence}-${id}.stdout.log`);
    paths.add(`commands/${sequence}-${id}.stderr.log`);
  });
  const coverage = phase7CoverageMap(root);
  for (const row of PHASE7_COVERAGE_ROWS) {
    const specIds = [...coverage[row]];
    const identity = sha256(JSON.stringify({ row, specIds }));
    paths.add(`${OBSERVATIONS_DIR}${row}-${identity}.json`);
  }
  return paths;
}

function checkFileClosure(stored, actual, allowedPaths) {
  const failures = [];
  if (!Array.isArray(stored)
    || stored.some(({ path, bytes, sha256: digest }) => typeof path !== 'string'
      || !Number.isSafeInteger(bytes) || bytes < 0 || !validSha256(digest))) {
    return [failure('evidence.closure', 'durable evidence file closure is malformed')];
  }
  const storedByName = new Map(stored.map((entry) => [entry.path, entry]));
  const actualByName = new Map(actual.map((entry) => [entry.path, entry]));
  for (const entry of stored) {
    if (!allowedPaths.has(entry.path)) {
      failures.push(failure('evidence.allowed', `evidence path ${entry.path} is outside the allowed layout`));
    }
  }
  for (const entry of actual) {
    const expected = storedByName.get(entry.path);
    if (expected === undefined) {
      failures.push(failure('evidence.unexpected', `unexpected evidence file ${entry.path}`));
    } else if (expected.bytes !== entry.bytes || expected.sha256 !== entry.sha256) {
      failures.push(failure('evidence.tampered', `evidence file ${entry.path} was tampered`));
    }
  }
  for (const entry of stored) {
    if (!actualByName.has(entry.path)) {
      failures.push(failure('evidence.missing', `evidence file ${entry.path} is missing`));
    }
  }
  return failures;
}

async function readGenesisReceipt(evidenceRoot) {
  try {
    return JSON.parse(await evidenceRoot.readFile('commands/0-genesis.receipt.json', 'utf8'));
  } catch (error) {
    return null;
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