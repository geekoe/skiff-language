import {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase1TestSummary as parsePhase7TestSummary,
  phase1CandidateSpecs,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
} from './bytecode-vm-phase-1-contract.mjs';
import {
  PHASE6_REQUIRED_LANES,
  phase6BoundedWorkLedger,
  phase6WorkloadProvenance,
  phase6WorkloadSpecs,
} from './bytecode-vm-phase-6-contract.mjs';

export {
  assertGitObject,
  commandEnvironmentIdentity,
  parsePhase7TestSummary,
  sha256,
  snapshotCommandEnvironment,
  validSha256,
};

export const PHASE7_COMMAND_SCHEMA = 'skiff-bytecode-vm-phase-7-command-r1-v1';
export const PHASE7_MANIFEST_SCHEMA = 'skiff-bytecode-vm-phase-7-gate-r1-v1';
export const PHASE7_CATALOG_SCHEMA = 'skiff-bytecode-vm-phase-7-catalog-r1-v1';
export const PHASE7_HANDOFF_SCHEMA = 'skiff-bytecode-vm-phase-7-handoff-r1-v1';
export const PHASE7_IDENTITY_SCHEMA = 'skiff-bytecode-vm-phase-7-identity-r1-v1';
export const PHASE7_ADAPTER_SCHEMA = 'skiff-bytecode-vm-phase-7-adapter-r1-v1';
export const PHASE7_GENESIS = 'skiff-bytecode-vm-phase-7-gate-r1-v1-genesis';
export const PHASE7_EPOCH = 'P7-E0';

export const PHASE7_REQUIRED_LANES = Object.freeze([
  'P7G',
  'C01',
  'C13',
  'C15',
  'C16',
  'C17',
  'C18',
  'phase-6-regression',
]);

export const PHASE7_COVERAGE_ROWS = Object.freeze([
  'C01', 'C02', 'C03', 'C04', 'C05', 'C06', 'C07', 'C08', 'C09',
  'C10', 'C11', 'C12', 'C13', 'C14', 'C15', 'C16', 'C17', 'C18',
]);

export const PHASE7_CAPABILITY_KEYS = Object.freeze([
  'service',
  'task-function',
  'task-Actor',
  'interface-local',
  'interface-remote',
  'callback-same-runtime',
  'callback-cross-runtime',
  'Actor',
  'DB',
  'recoverable',
  'request-GC',
  'Actor-compaction',
]);

const PHASE7_GC_CAPABILITIES = Object.freeze(['request-GC', 'Actor-compaction']);

// Reviewed effective test counts for inherited Phase 1-5 cargo/node test
// commands whose original expectedTests state is missing. Values are the exact
// test totals recorded by the accepted Phase 5 Gate receipts for the same
// command identities (Phase 5 result records 107/107 commands and 500/500
// tests). Keyed by the Phase 5 source id carried in the provenance origin
// chain; never derived from ID prefix parsing.
//
// P7R-2 epoch: four Phase 1-3 inherited specs gained Phase 5/6 收尾 tests
// (phase_5_*/phase_6_* carrier, record-array and resource-carrier tests) that
// all pass on the candidate; the exact counts below were re-verified against
// the P7-E0 evidence stdout and updated to the executed totals. After the
// scheduler compile repair, two scheduler adapter counts were re-verified
// against the P7-E0 evidence stdout and aligned to the executed totals
// (k4-scheduler-park-resume: 5, k3-scheduler-resume-throw: 11). This change
// opens a new evidence epoch (P7-E0 evidence is no longer reusable).
const PHASE7_INHERITED_EFFECTIVE_COUNTS = Object.freeze({
  'a5-affine-take-opcode': 1,
  'a5-exact-executor-registry': 2,
  'a5-ordinary-shape-affine-child-rejection': 1,
  'a5-privileged-http-stream-composite': 2,
  'c5-affine-body-take-emission': 1,
  'c5-exact-registry-source-emission': 1,
  'c5-production-affine-publication': 1,
  'c5-second-body-take-fails-closed': 1,
  'c5-unsupported-registry-rows-fail-closed': 1,
  'h5-production-bytecode-http-composition': 15,
  'h5-server-stream-flush-ack': 4,
  'h5-typed-allocation-trait-object': 1,
  'k5-request-phase-5-integration': 18,
  'k5-request-phase-5-library': 26,
  'k5-scheduler-phase-5-ownership': 18,
  'phase-4-regression-c4-emission-host-effect-admission': 9,
  'phase-4-regression-k4-scheduler-duplicate-wake': 2,
  'phase-4-regression-k4-scheduler-park-resume': 5,
  'phase-4-regression-k4-scheduler-pending-publish-claim': 2,
  'phase-4-regression-k4-scheduler-terminal-race': 1,
  'phase-4-regression-phase-3-regression-c3-emission-throw-admission': 10,
  'phase-4-regression-phase-3-regression-c3-emission-throw-emission': 10,
  'phase-4-regression-phase-3-regression-k3-linker-throw-admission': 2,
  'phase-4-regression-phase-3-regression-k3-model-service-error-envelope': 18,
  'phase-4-regression-phase-3-regression-k3-request-user-error': 1,
  'phase-4-regression-phase-3-regression-k3-scheduler-resume-throw': 11,
  'phase-4-regression-phase-3-regression-k3-vm-throw-unwind': 5,
  'phase-4-regression-phase-3-regression-phase-1-regression-gate-self-tests': 63,
  'phase-4-regression-phase-3-regression-phase-1-regression-k0a-compiler-admission': 8,
  'phase-4-regression-phase-3-regression-phase-1-regression-k0a-emission-admission': 7,
  'phase-4-regression-phase-3-regression-phase-1-regression-k0b-tc-production-contract': 6,
  'phase-4-regression-phase-3-regression-phase-1-regression-k0c-request-containment': 1,
  'phase-4-regression-phase-3-regression-phase-1-regression-k2-deep-local-call-frame-fuel': 4,
  'phase-4-regression-phase-3-regression-phase-1-regression-l4-raw-fuel-exact-boundary': 15,
  'phase-4-regression-phase-3-regression-phase-1-regression-l5-deterministic-deadline-internal-stop': 10,
  'phase-4-regression-phase-3-regression-phase-1-regression-phase0-production-boundaries-regression': 1,
  'phase-4-regression-phase-3-regression-phase-1-regression-phase0-production-composition-regression': 1,
  'phase-4-regression-phase-3-regression-phase-1-regression-phase0-request-scalar-regression': 1,
  'phase-4-regression-phase-3-regression-phase-1-regression-tr-v1-production-proof': 1,
  'phase-4-regression-phase-3-regression-phase-2-regression-c2-emission-exact-plan': 2,
  'phase-4-regression-phase-3-regression-phase-2-regression-c2-pipeline-exact-facts': 5,
  'phase-4-regression-phase-3-regression-phase-2-regression-k2-lifecycle-executor': 8,
  'phase-4-regression-phase-3-regression-phase-2-regression-k2-linker-record-array-admission': 12,
  'phase-4-regression-phase-3-regression-phase-2-regression-k2-model-writable-path': 15,
  'phase-4-regression-phase-3-regression-phase-2-regression-k2-request-heap-cow': 32,
  'phase-4-regression-phase-3-regression-phase-2-regression-phase-2-gate-self-tests': 24,
  'phase-4-regression-phase-3-regression-phase-2-regression-phase-2-missing-plan-negative': 1,
  'phase-4-regression-phase-3-regression-phase-2-regression-phase-2-vcp-production-composition': 1,
  'phase-4-regression-phase-3-regression-phase-3-gate-self-tests': 26,
  'phase-4-regression-phase-3-regression-phase-3-negative-catch-mismatch': 1,
  'phase-4-regression-phase-3-regression-phase-3-negative-host-pending-throw': 1,
  'phase-4-regression-phase-3-regression-phase-3-negative-uncaught-throw': 1,
  'phase-4-regression-phase-3-regression-phase-3-vcp-production-composition': 1,
  'phase-4-regression-phase-3-regression-phase-3-controlled-resume-harness': 1,
  'phase-4-regression-phase-4-gate-self-tests': 25,
  'phase-4-regression-phase-4-negative-cancel-before-complete': 1,
  'phase-4-regression-phase-4-negative-deadline-race': 1,
  'phase-4-regression-phase-4-negative-duplicate-wake-drop': 1,
  'phase-4-regression-phase-4-negative-session-disconnect': 1,
  'phase-4-regression-phase-4-stage-sentinel-admission-to-emission': 1,
  'phase-4-regression-phase-4-stage-sentinel-atomic-link-to-image': 1,
  'phase-4-regression-phase-4-stage-sentinel-emission-to-atomic-link-input': 1,
  'phase-4-regression-phase-4-stage-sentinel-image-to-scheduler': 1,
  'phase-4-regression-phase-4-stage-sentinel-scheduler-to-request-response': 1,
  'phase-4-regression-phase-4-stage-sentinel-source-to-admission': 1,
  'phase-4-regression-phase-4-vcp-production-composition': 1,
  'phase-4-regression-v4-linker-typed-host-entry': 1,
  'phase-5-deterministic-tcp-upstream': 2,
  'phase-5-gate-self-tests': 37,
  'phase-5-lifecycle-race-matrix': 2,
  'phase-5-router-full-chain-vcp': 2,
  'phase-5-s1-source-to-admission': 1,
  'phase-5-s2-admission-to-emission': 1,
  'phase-5-s3-emission-to-atomic-link-input': 1,
  'phase-5-s4-atomic-link-to-image': 1,
  'phase-5-s5-image-to-scheduler': 1,
  'phase-5-s6-request-response': 1,
  'phase-5-single-worker-canary': 2,
  'phase-5-structural-no-bypass': 2,
  'phase-5-vcp-production-composition': 1,
  'v5r-atomic-image-runtime-views': 1,
  'v5r-host-binding-key-rejections': 1,
  'v5r-host-signature-drift-rejections': 3,
  'v5r-indexed-typed-executor-target': 1,
  'v5r-linker-stream-dual-resume': 1,
  'v5r-missing-statement-fact-rejection': 1,
  'v5r-production-affine-image': 1,
  'v5r-registry-executor-identity-closure': 3,
  'v5r-swapped-resume-descriptor-rejection': 1,
});

export function phase7ScenarioSpecs(root) {
  return Object.freeze([
    phase7Spec(root, 'phase-7-gate-self-tests', 'node', [
      '--test',
      '--test-reporter=tap',
      'scripts/tests/bytecode-vm-phase-7-gate-*.test.mjs',
    ], 'node-tap', ['P7G', 'C18'], 43),
    phase7Spec(root, 'phase-7-catalog-binding', 'node', [
      '--input-type=module',
      '-e',
      "import('./scripts/lib/bytecode-vm-phase-7-contract.mjs').then(m => m.assertPhase7Catalog(process.cwd())).catch(error => { console.error(error); process.exit(1); })",
    ], null, ['P7G', 'C01', 'C13', 'C15', 'C16']),
    phase7Spec(root, 'phase-7-identity-probe', 'node', [
      'scripts/lib/bytecode-vm-phase-7-identity-probe.mjs',
    ], null, ['P7G', 'C16', 'C17']),
    phase7Spec(root, 'phase-7-whole-system-producer', 'node', [
      'scripts/lib/bytecode-vm-phase-7-whole-system-harness.mjs',
      'producer',
    ], null, ['P7G', 'C01', 'C18'], undefined, {
      producedArtifacts: Object.freeze(['phase-7-whole-system-composition']),
    }),
    phase7Spec(root, 'phase-7-whole-system-consumer', 'node', [
      'scripts/lib/bytecode-vm-phase-7-whole-system-harness.mjs',
      'consumer',
    ], 'node-tap', ['P7G', 'C01', 'C18'], 1, {
      dependsOn: Object.freeze(['phase-7-whole-system-producer']),
      requiredArtifacts: Object.freeze(['phase-7-whole-system-composition']),
    }),
  ]);
}

// phase7WorkloadSpecs(root) = the Phase 7 scenario specs plus exactly one
// cumulative Phase 6 workload import. The Phase 6 list is re-IDed once under
// `phase-7-regression-` and keeps the exact phase6WorkloadProvenance record
// (sourcePhase/sourceId, immediate parent and ordered origin chain) plus the
// semantic lanes and the original expectedTests state. Provenance is imported,
// never re-derived by parsing nested ID prefixes.
export function phase7WorkloadSpecs(root) {
  const inherited = phase6WorkloadSpecs(root).map((entry) => {
    const id = `phase-7-regression-${entry.id}`;
    const originChain = [
      ...entry.originChain,
      { phase: 7, id },
    ];
    return spec({
      id,
      cwd: entry.cwd,
      command: entry.command,
      args: entry.args,
      testFormat: entry.testFormat,
      lanes: [...entry.lanes, 'phase-6-regression'],
      expectedTests: entry.expectedTests,
      sourcePhase: entry.sourcePhase,
      sourceId: entry.sourceId,
      parentPhase: 6,
      parentId: entry.id,
      originChain,
      dependsOn: entry.dependsOn,
      producedArtifacts: entry.producedArtifacts,
      requiredArtifacts: entry.requiredArtifacts,
    });
  });
  return Object.freeze([
    ...phase7ScenarioSpecs(root),
    ...inherited,
  ]);
}

export function phase7CandidateSpecs(root) {
  return phase1CandidateSpecs(root);
}

export function phase7WorkloadProvenance(root) {
  return phase7WorkloadSpecs(root).map((entry) => Object.freeze({
    id: entry.id,
    sourcePhase: entry.sourcePhase,
    sourceId: entry.sourceId,
    parentPhase: entry.parentPhase ?? null,
    parentId: entry.parentId ?? null,
    originChain: Object.freeze(entry.originChain.map(Object.freeze)),
  }));
}

export function phase7CapabilityLedger() {
  return Object.freeze({
    service: 'accepted',
    'task-function': 'accepted',
    'task-Actor': 'accepted',
    'interface-local': 'accepted',
    'interface-remote': 'accepted',
    'callback-same-runtime': 'accepted',
    'callback-cross-runtime': 'disabled',
    Actor: 'accepted',
    DB: 'accepted',
    recoverable: 'accepted',
    'request-GC': Object.freeze({ state: 'disabled', disposition: 'deferred' }),
    'Actor-compaction': Object.freeze({ state: 'disabled', disposition: 'deferred' }),
  });
}

export function phase7CapabilityCompanions(root) {
  const finalId = (sourceId) => resolveSpecId(root, sourceId);
  return Object.freeze({
    service: Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-service-matrix'),
    }),
    'task-function': Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-task-host-matrix'),
    }),
    'task-Actor': Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-actor-host-matrix'),
    }),
    'interface-local': Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-interface-local-matrix'),
    }),
    'interface-remote': Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-interface-remote-matrix'),
    }),
    'callback-same-runtime': Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-callback-matrix'),
    }),
    'callback-cross-runtime': Object.freeze({
      state: 'disabled', polarity: 'negative', companion: finalId('p6-containment-matrix'),
    }),
    Actor: Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-actor-host-matrix'),
    }),
    DB: Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-db-matrix'),
    }),
    recoverable: Object.freeze({
      state: 'accepted', polarity: 'positive', companion: finalId('p6-recoverable-matrix'),
    }),
    'request-GC': Object.freeze({
      state: 'disabled', polarity: 'negative', disposition: 'deferred',
      companion: finalId('p6-containment-matrix'),
    }),
    'Actor-compaction': Object.freeze({
      state: 'disabled', polarity: 'negative', disposition: 'deferred',
      companion: finalId('p6-containment-matrix'),
    }),
  });
}

export function phase7CoverageMap(root) {
  const inherited = phase7WorkloadSpecs(root)
    .filter(({ sourcePhase }) => sourcePhase < 6)
    .map(({ id }) => id);
  const finalId = (sourceId) => resolveSpecId(root, sourceId);
  const boundedWork = phase6BoundedWorkLedger(root);
  const boundedWorkIds = Object.values(boundedWork).flat().map((phase6Id) =>
    resolvePhase6Id(root, phase6Id));
  const phase4Negatives = [
    'phase-4-negative-cancel-before-complete',
    'phase-4-negative-deadline-race',
    'phase-4-negative-duplicate-wake-drop',
    'phase-4-negative-session-disconnect',
  ].map(finalId);
  const regression = (suffix) => resolveSpecId(root, `phase-5-${suffix}`);
  const coverage = {
    C01: [...inherited, 'phase-7-whole-system-consumer'],
    C02: ['phase-7-identity-probe', 'phase-7-catalog-binding'],
    C03: [finalId('p6-service-matrix')],
    C04: [regression('deterministic-tcp-upstream')],
    C05: [finalId('p6-service-matrix')],
    C06: [
      finalId('p6-task-host-matrix'),
      finalId('p6-task-router-matrix'),
      finalId('p6-actor-host-matrix'),
      finalId('p6-actor-router-matrix'),
    ],
    C07: [finalId('p6-interface-local-matrix'), finalId('p6-interface-remote-matrix')],
    C08: [finalId('p6-callback-matrix')],
    C09: [finalId('p6-actor-host-matrix'), finalId('p6-actor-router-matrix')],
    C10: [finalId('p6-recoverable-matrix'), finalId('p6-db-matrix')],
    C11: phase4Negatives,
    C12: [finalId('p6-kernel-focused'), finalId('p6-containment-matrix')],
    C13: [...new Set(boundedWork['p1-dispatch-fuel'].map((id) => resolvePhase6Id(root, id)))],
    C14: [finalId('p6-kernel-focused'), finalId('p6-containment-matrix')],
    C15: [...new Set(boundedWorkIds)].sort(),
    C16: ['phase-7-catalog-binding', 'phase-7-identity-probe'],
    C17: ['phase-7-identity-probe', finalId('p6-no-verifier-structural')],
    C18: ['phase-7-gate-self-tests', 'phase-7-whole-system-consumer'],
  };
  return Object.freeze(Object.fromEntries(
    PHASE7_COVERAGE_ROWS.map((row) => [row, Object.freeze([...coverage[row]])]),
  ));
}

export function phase7AdapterCatalog(root) {
  const inherited = phase7WorkloadSpecs(root).filter(({ sourcePhase }) => sourcePhase < 6);
  return Object.freeze({
    schemaVersion: PHASE7_ADAPTER_SCHEMA,
    rows: Object.freeze(inherited.map((specEntry) => Object.freeze({
      id: specEntry.id,
      originalState: expectedTestsState(specEntry),
      testFormat: specEntry.testFormat,
      effectiveCount: effectiveTestCount(specEntry),
    }))),
  });
}

export function phase7SpecCatalog(root) {
  const specs = phase7WorkloadSpecs(root).map((entry) => Object.freeze({
    id: entry.id,
    command: entry.command,
    args: Object.freeze([...entry.args]),
    cwd: entry.cwd,
    testFormat: entry.testFormat,
    lanes: Object.freeze([...entry.lanes]),
    sourcePhase: entry.sourcePhase,
    sourceId: entry.sourceId,
    parentPhase: entry.parentPhase ?? null,
    parentId: entry.parentId ?? null,
    originChain: Object.freeze(entry.originChain.map(Object.freeze)),
    dependsOn: Object.freeze(entry.dependsOn ?? []),
    producedArtifacts: Object.freeze(entry.producedArtifacts ?? []),
    requiredArtifacts: Object.freeze(entry.requiredArtifacts ?? []),
    expectedTests: expectedTestsState(entry),
    effectiveTests: effectiveTestCount(entry),
  }));
  const content = {
    schemaVersion: PHASE7_CATALOG_SCHEMA,
    specs,
    provenance: phase7WorkloadProvenance(root),
    adapter: phase7AdapterCatalog(root),
    coverage: phase7CoverageMap(root),
    capabilities: phase7CapabilityCompanions(root),
    ledger: phase7CapabilityLedger(root),
  };
  return Object.freeze({
    ...content,
    digest: sha256(JSON.stringify(content)),
  });
}

export function phase7SpecCatalogDigest(root) {
  return phase7SpecCatalog(root).digest;
}

export function phase7ExpectedTestsIdentity(spec) {
  return { state: expectedTestsState(spec), effective: effectiveTestCount(spec) };
}

export function phase7EffectiveTestCount(spec) {
  return effectiveTestCount(spec);
}

export function assertPhase7Catalog(root) {
  const specs = phase7WorkloadSpecs(root);
  const byId = new Map(specs.map((entry) => [entry.id, entry]));
  if (byId.size !== specs.length) {
    throw new Error(`Phase 7 workload catalog has duplicate spec ids`);
  }
  for (const phase of [1, 2, 3, 4, 5, 6, 7]) {
    if (!specs.some(({ sourcePhase }) => sourcePhase === phase)) {
      throw new Error(`Phase 7 catalog has no sourcePhase ${phase}`);
    }
  }
  const coverage = phase7CoverageMap(root);
  for (const row of PHASE7_COVERAGE_ROWS) {
    const ids = coverage[row];
    if (!Array.isArray(ids) || ids.length === 0) {
      throw new Error(`Phase 7 coverage row ${row} must map to at least one spec`);
    }
    if (new Set(ids).size !== ids.length) throw new Error(`Phase 7 coverage row ${row} repeats an id`);
    for (const id of ids) {
      if (!byId.has(id)) throw new Error(`Phase 7 coverage row ${row} has unknown id ${id}`);
    }
  }
  const companions = phase7CapabilityCompanions(root);
  for (const key of PHASE7_CAPABILITY_KEYS) {
    const row = companions[key];
    if (!row || !byId.has(row.companion)) {
      throw new Error(`Phase 7 capability ${key} lacks a valid companion spec`);
    }
  }
  const ledger = phase7CapabilityLedger(root);
  assertPhase7CapabilityLedger(ledger);
  for (const key of PHASE7_CAPABILITY_KEYS) {
    const ledgerState = capabilityState(ledger[key]);
    const companion = companions[key];
    if (ledgerState === 'accepted' && companion.polarity !== 'positive') {
      throw new Error(`Phase 7 accepted capability ${key} must use a positive companion`);
    }
    if (ledgerState === 'disabled' && companion.polarity !== 'negative') {
      throw new Error(`Phase 7 disabled capability ${key} must use a fail-closed companion`);
    }
  }
  const adapter = phase7AdapterCatalog(root);
  if (adapter?.schemaVersion !== PHASE7_ADAPTER_SCHEMA) {
    throw new Error('Phase 7 adapter catalog carries a stale schema');
  }
  const historical = specs.filter(({ sourcePhase }) => sourcePhase < 6);
  const adapterById = new Map(adapter.rows.map((row) => [row.id, row]));
  for (const entry of historical) {
    const row = adapterById.get(entry.id);
    if (!row) throw new Error(`Phase 7 adapter catalog is missing inherited spec ${entry.id}`);
    if (row.originalState !== expectedTestsState(entry)) {
      throw new Error(`Phase 7 adapter row ${entry.id} erased the original expectedTests state`);
    }
    if (row.testFormat !== entry.testFormat) {
      throw new Error(`Phase 7 adapter row ${entry.id} drifted its test format`);
    }
    if (entry.testFormat === null && row.effectiveCount !== null) {
      throw new Error(`Phase 7 non-test adapter row ${entry.id} must not carry an effective count`);
    }
    if (entry.testFormat !== null
      && (!Number.isSafeInteger(row.effectiveCount) || row.effectiveCount <= 0)) {
      throw new Error(`Phase 7 test-formatted adapter row ${entry.id} needs a positive effective count`);
    }
  }
  if (adapter.rows.length !== historical.length) {
    throw new Error('Phase 7 adapter catalog must bind every inherited spec exactly once');
  }
  assertPhase7LaneCoverage(specs);
  const provenance = phase7WorkloadProvenance(root);
  assertPhase7ProvenanceCoverage(specs, provenance);
  assertPhase7DependencyGraph(specs);
  for (const entry of specs) {
    if (entry.command === 'cargo' && entry.args[0] === 'test') {
      const normalized = entry.args.filter((argument) => argument === '--no-fail-fast');
      if (normalized.length !== 1) {
        throw new Error(`Phase 7 inherited cargo test ${entry.id} must normalize to exactly one --no-fail-fast`);
      }
    }
  }
  return true;
}

export function assertPhase7CapabilityLedger(ledger) {
  if (!ledger || typeof ledger !== 'object' || Array.isArray(ledger)) {
    throw new Error('Phase 7 capability ledger must be an object');
  }
  const actual = Object.keys(ledger).sort();
  const expected = [...PHASE7_CAPABILITY_KEYS].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`Phase 7 capability ledger keys drifted: ${actual.join(',')}`);
  }
  for (const key of PHASE7_CAPABILITY_KEYS) {
    const value = ledger[key];
    if (PHASE7_GC_CAPABILITIES.includes(key)) {
      if (!value || typeof value !== 'object' || Array.isArray(value)
        || !['accepted', 'disabled'].includes(value.state)
        || !['disabled', 'deferred'].includes(value.disposition)) {
        throw new Error(`Phase 7 GC capability ${key} requires an explicit state and disposition`);
      }
    } else if (!['accepted', 'disabled'].includes(value)) {
      throw new Error(`Phase 7 capability ${key} has an unrecognized state`);
    }
  }
}

export function assertPhase7LaneCoverage(specs) {
  const observed = new Set(specs.flatMap(({ lanes }) => lanes));
  const missing = [...PHASE7_REQUIRED_LANES, ...PHASE6_REQUIRED_LANES]
    .filter((lane) => !observed.has(lane));
  if (missing.length > 0) {
    throw new Error(`Phase 7 r1 Gate workload matrix is missing lane(s): ${missing.join(', ')}`);
  }
}

export function assertPhase7ProvenanceCoverage(specs, provenance) {
  const byId = new Map(specs.map((entry) => [entry.id, entry]));
  const seen = new Map();
  if (!Array.isArray(provenance)) {
    throw new Error('Phase 7 provenance must be an array');
  }
  for (const row of provenance) {
    if (!row || typeof row !== 'object' || typeof row.id !== 'string') {
      throw new Error('Phase 7 provenance rows require an id');
    }
    if (seen.has(row.id)) throw new Error(`duplicate Phase 7 provenance id ${row.id}`);
    seen.set(row.id, row);
    const spec = byId.get(row.id);
    if (!spec) throw new Error(`unknown Phase 7 provenance id ${row.id}`);
    if (spec.sourcePhase !== row.sourcePhase || spec.sourceId !== row.sourceId
      || spec.parentPhase !== row.parentPhase || spec.parentId !== row.parentId
      || JSON.stringify(spec.originChain) !== JSON.stringify(row.originChain)) {
      throw new Error(`provenance row does not match spec ${row.id}`);
    }
    if (!Array.isArray(row.originChain) || row.originChain.length === 0) {
      throw new Error(`provenance row ${row.id} has no origin chain`);
    }
    const phases = row.originChain.map(({ phase }) => phase);
    if (phases.some((phase) => !Number.isInteger(phase) || phase < 1 || phase > 7)) {
      throw new Error(`provenance row ${row.id} has invalid origin phase`);
    }
    for (let index = 1; index < phases.length; index += 1) {
      if (phases[index] <= phases[index - 1]) {
        throw new Error(`provenance row ${row.id} origin phases are not strictly increasing`);
      }
    }
    if (row.originChain.at(-1)?.phase !== 7 || row.originChain.at(-1)?.id !== row.id) {
      throw new Error(`provenance row ${row.id} must end with the final Phase 7 id`);
    }
    if (spec.sourcePhase < 7) {
      if (row.originChain[0]?.phase !== spec.sourcePhase
        || row.originChain[0]?.id !== spec.sourceId) {
        throw new Error(`provenance row ${row.id} must start with the original source identity`);
      }
      const phase6 = row.originChain.at(-2);
      if (phase6?.phase !== 6 || phase6?.id !== spec.parentId) {
        throw new Error(`provenance row ${row.id} must record the immediate Phase 6 parent`);
      }
    }
  }
  for (const spec of specs) {
    if (!seen.has(spec.id)) throw new Error(`spec ${spec.id} is missing provenance`);
  }
  for (const phase of [1, 2, 3, 4, 5, 6, 7]) {
    if (!provenance.some(({ sourcePhase }) => sourcePhase === phase)) {
      throw new Error(`Phase 7 provenance has no sourcePhase ${phase}`);
    }
  }
  const phase6Ids = new Set(phase6WorkloadSpecs('/candidate').map(({ id }) => id));
  const importedParents = provenance
    .filter((row) => row.parentPhase === 6)
    .map((row) => row.parentId);
  if (new Set(importedParents).size !== importedParents.length
    || importedParents.some((id) => !phase6Ids.has(id))
    || importedParents.length !== phase6Ids.size) {
    throw new Error('Phase 7 provenance must import each Phase 6 workload exactly once');
  }
}

export function assertPhase7DependencyGraph(specs) {
  const ids = new Set(specs.map(({ id }) => id));
  for (const entry of specs) {
    for (const dependency of entry.dependsOn ?? []) {
      if (typeof dependency !== 'string' || !ids.has(dependency)) {
        throw new Error(`spec ${entry.id} has an unknown dependency ${dependency}`);
      }
      if (dependency === entry.id) throw new Error(`spec ${entry.id} depends on itself`);
    }
    for (const artifact of [...(entry.producedArtifacts ?? []), ...(entry.requiredArtifacts ?? [])]) {
      if (typeof artifact !== 'string' || artifact.length === 0) {
        throw new Error(`spec ${entry.id} declares an invalid artifact identity`);
      }
    }
    const required = new Set(entry.requiredArtifacts ?? []);
    for (const producer of specs.filter((candidate) =>
      (candidate.producedArtifacts ?? []).some((artifact) => required.has(artifact)))) {
      if (!(entry.dependsOn ?? []).includes(producer.id)) {
        throw new Error(`spec ${entry.id} requires artifact produced by ${producer.id} without a dependency`);
      }
    }
  }
  const remaining = new Map(specs.map((entry) => [
    entry.id,
    new Set((entry.dependsOn ?? []).filter((dependency) => ids.has(dependency))),
  ]));
  const order = [];
  const ready = new Set([...remaining.keys()].filter((id) => remaining.get(id).size === 0));
  while (ready.size > 0) {
    const next = [...ready].sort().shift();
    ready.delete(next);
    order.push(next);
    for (const [id, deps] of remaining) {
      if (deps.delete(next) && deps.size === 0) ready.add(id);
    }
  }
  if (order.length !== specs.length) {
    throw new Error('Phase 7 workload dependency graph is cyclic');
  }
  return order;
}

export function phase7ExecutionOrder(root) {
  const candidate = phase7CandidateSpecs(root);
  const workloads = assertPhase7DependencyGraph(phase7WorkloadSpecs(root)).map((id) => id);
  return [
    ...candidate.slice(0, 3).map(({ id }) => id),
    ...workloads,
    ...candidate.slice(3).map(({ id }) => id),
  ];
}

function expectedTestsState(spec) {
  if (!Object.hasOwn(spec, 'expectedTests')) return 'missing';
  return spec.expectedTests;
}

function effectiveTestCount(spec) {
  const original = Object.hasOwn(spec, 'expectedTests') ? spec.expectedTests : 'missing';
  if (spec.testFormat === null) return null;
  if (Number.isInteger(original)) return original;
  if (spec.testFormat === 'rust-exact') return 1;
  const phase5 = spec.originChain?.find(({ phase }) => phase === 5);
  const reviewed = phase5 ? PHASE7_INHERITED_EFFECTIVE_COUNTS[phase5.id] : undefined;
  if (!Number.isSafeInteger(reviewed) || reviewed <= 0) {
    throw new Error(`Phase 7 adapter catalog has no reviewed effective count for ${spec.id}`);
  }
  return reviewed;
}

function resolveSpecId(root, sourceId) {
  const match = phase7WorkloadSpecs(root).find(({ sourceId: candidate }) => candidate === sourceId);
  if (!match) throw new Error(`Phase 7 catalog cannot resolve source id ${sourceId}`);
  return match.id;
}

function resolvePhase6Id(root, phase6Id) {
  const match = phase7WorkloadSpecs(root).find((entry) =>
    entry.parentPhase === 6 && entry.parentId === phase6Id);
  if (!match) throw new Error(`Phase 7 catalog cannot resolve Phase 6 id ${phase6Id}`);
  return match.id;
}

function capabilityState(value) {
  return typeof value === 'object' && value !== null ? value.state : value;
}

function phase7Spec(cwd, id, command, args, testFormat, lanes, expectedTests = undefined, options = {}) {
  const entry = {
    id,
    cwd,
    command,
    args,
    testFormat,
    lanes,
    sourcePhase: 7,
    sourceId: id,
    parentPhase: null,
    parentId: null,
    originChain: [{ phase: 7, id }],
    ...options,
  };
  return expectedTests === undefined ? spec(entry) : Object.freeze({
    ...spec(entry),
    expectedTests,
  });
}

function spec(entry) {
  return Object.freeze({
    id: entry.id,
    command: entry.command,
    args: Object.freeze([...entry.args]),
    cwd: entry.cwd,
    testFormat: entry.testFormat,
    lanes: Object.freeze([...entry.lanes]),
    ...(entry.expectedTests === undefined ? {} : { expectedTests: entry.expectedTests }),
    sourcePhase: entry.sourcePhase,
    sourceId: entry.sourceId,
    parentPhase: entry.parentPhase,
    parentId: entry.parentId,
    originChain: Object.freeze(entry.originChain.map(Object.freeze)),
    ...(entry.dependsOn === undefined ? {} : { dependsOn: Object.freeze([...entry.dependsOn]) }),
    ...(entry.producedArtifacts === undefined
      ? {} : { producedArtifacts: Object.freeze([...entry.producedArtifacts]) }),
    ...(entry.requiredArtifacts === undefined
      ? {} : { requiredArtifacts: Object.freeze([...entry.requiredArtifacts]) }),
  });
}