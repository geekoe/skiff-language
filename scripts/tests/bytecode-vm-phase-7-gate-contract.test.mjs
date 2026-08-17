import assert from 'node:assert/strict';
import { resolve, dirname } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  PHASE7_ADAPTER_SCHEMA,
  PHASE7_CATALOG_SCHEMA,
  PHASE7_COMMAND_SCHEMA,
  PHASE7_COVERAGE_ROWS,
  PHASE7_HANDOFF_SCHEMA,
  PHASE7_IDENTITY_SCHEMA,
  PHASE7_MANIFEST_SCHEMA,
  PHASE7_REQUIRED_LANES,
  assertPhase7Catalog,
  assertPhase7CapabilityLedger,
  assertPhase7DependencyGraph,
  phase7AdapterCatalog,
  phase7CandidateSpecs,
  phase7CapabilityCompanions,
  phase7CapabilityLedger,
  phase7CoverageMap,
  phase7ExecutionOrder,
  phase7ScenarioSpecs,
  phase7SpecCatalog,
  phase7SpecCatalogDigest,
  phase7WorkloadProvenance,
  phase7WorkloadSpecs,
} from '../lib/bytecode-vm-phase-7-contract.mjs';
import {
  PHASE6_REQUIRED_LANES,
  phase6WorkloadProvenance,
  phase6WorkloadSpecs,
} from '../lib/bytecode-vm-phase-6-contract.mjs';
import {
  assertPhase6NoVerifierStructural,
} from '../lib/bytecode-vm-phase-6-contract.mjs';
import {
  PHASE7_DIRECTORY_IDENTITY_FILE,
  PHASE7_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-7-evidence-root.mjs';
import { buildVerifyPlan, PUBLIC_SELECTORS } from '../lib/verify-plan.mjs';

const ROOT = '/candidate';
const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

test('r1 schemas are distinct from every earlier Phase and bind the closure epoch', () => {
  assert.equal(PHASE7_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-7-command-r1-v1');
  assert.equal(PHASE7_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-7-gate-r1-v1');
  assert.equal(PHASE7_CATALOG_SCHEMA, 'skiff-bytecode-vm-phase-7-catalog-r1-v1');
  assert.equal(PHASE7_HANDOFF_SCHEMA, 'skiff-bytecode-vm-phase-7-handoff-r1-v1');
  assert.equal(PHASE7_IDENTITY_SCHEMA, 'skiff-bytecode-vm-phase-7-identity-r1-v1');
  assert.equal(PHASE7_ADAPTER_SCHEMA, 'skiff-bytecode-vm-phase-7-adapter-r1-v1');
  assert.equal(PHASE7_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-7-directory-identity-r1-v1');
  assert.equal(PHASE7_DIRECTORY_IDENTITY_FILE, 'phase-7-r1-v1-directory-identities.json');
});

test('catalog is the Phase 7 scenarios plus exactly one Phase 6 cumulative import', () => {
  const scenarios = phase7ScenarioSpecs(ROOT);
  const workloads = phase7WorkloadSpecs(ROOT);
  const phase6 = phase6WorkloadSpecs(ROOT);
  assert.equal(scenarios.length, 5);
  assert.equal(phase6.length, 111);
  assert.equal(workloads.length, 111 + 5);
  const ids = workloads.map(({ id }) => id);
  assert.equal(new Set(ids).size, ids.length);
  const imported = workloads.filter(({ id }) => id.startsWith('phase-7-regression-'));
  assert.equal(imported.length, 111, 'exactly one Phase 6 cumulative import');
  assert.equal(
    imported.every(({ parentPhase }) => parentPhase === 6),
    true,
  );
  const phase6Ids = new Set(phase6.map(({ id }) => id));
  assert.equal(
    imported.every(({ parentId }) => phase6Ids.has(parentId)),
    true,
    'the immediate Phase 6 parent must come from the imported provenance record',
  );
  assert.equal(
    new Set(imported.map(({ parentId }) => parentId)).size,
    111,
    'the Phase 6 import must be bijective',
  );
  for (const scenario of scenarios) {
    assert.equal(scenario.sourcePhase, 7);
    assert.equal(scenario.sourceId, scenario.id);
    assert.equal(scenario.parentPhase, null);
    assert.equal(scenario.parentId, null);
    assert.deepEqual(scenario.originChain, [{ phase: 7, id: scenario.id }]);
  }
  assert.equal(phase7CandidateSpecs(ROOT).length, 12);
});

test('provenance is imported from Phase 6, never derived from ID prefixes', () => {
  const workloads = phase7WorkloadSpecs(ROOT);
  const provenance = phase7WorkloadProvenance(ROOT);
  assert.equal(provenance.length, workloads.length);
  assert.doesNotThrow(() => assertPhase7Catalog(ROOT));
  const phase6Provenance = new Map(
    phase6WorkloadProvenance(ROOT).map((row) => [row.id, row]),
  );
  for (const inherited of workloads.filter(({ sourcePhase }) => sourcePhase < 7)) {
    const row = provenance.find(({ id }) => id === inherited.id);
    assert.ok(row, inherited.id);
    const parent = phase6Provenance.get(inherited.parentId);
    assert.ok(parent, `missing Phase 6 provenance for ${inherited.parentId}`);
    assert.deepEqual(inherited.originChain.slice(0, -1), parent.originChain);
    assert.deepEqual(inherited.originChain.at(-1), { phase: 7, id: inherited.id });
    assert.equal(inherited.sourcePhase, parent.sourcePhase);
    assert.equal(inherited.sourceId, parent.sourceId);
    const phases = inherited.originChain.map(({ phase }) => phase);
    assert.equal(phases.every((phase, index) => index === 0 || phase > phases[index - 1]), true);
  }
  assert.deepEqual(
    new Set(provenance.map(({ sourcePhase }) => sourcePhase)),
    new Set([1, 2, 3, 4, 5, 6, 7]),
  );
});

test('adapter catalog binds every inherited expectedTests residual without erasure', () => {
  const adapter = phase7AdapterCatalog(ROOT);
  assert.equal(adapter.schemaVersion, PHASE7_ADAPTER_SCHEMA);
  const inherited = phase7WorkloadSpecs(ROOT).filter(({ sourcePhase }) => sourcePhase < 6);
  assert.equal(adapter.rows.length, inherited.length);
  assert.equal(adapter.rows.length, 95);
  const byId = new Map(adapter.rows.map((row) => [row.id, row]));
  const residual = { missing: 0, null: 0, integer: 0 };
  for (const spec of inherited) {
    const row = byId.get(spec.id);
    assert.ok(row, spec.id);
    const original = Object.hasOwn(spec, 'expectedTests') ? spec.expectedTests : 'missing';
    assert.equal(row.originalState, original, `${spec.id} original state`);
    assert.equal(row.testFormat, spec.testFormat);
    if (spec.testFormat === null) {
      assert.equal(row.effectiveCount, null, `${spec.id} non-test has no effective count`);
    } else {
      assert.equal(Number.isSafeInteger(row.effectiveCount) && row.effectiveCount > 0, true,
        `${spec.id} needs a positive effective count`);
      if (spec.testFormat === 'rust-exact') assert.equal(row.effectiveCount, 1);
    }
    if (original === 'missing') residual.missing += 1;
    else if (original === null) residual.null += 1;
    else residual.integer += 1;
  }
  assert.deepEqual(residual, { missing: 71, null: 0, integer: 24 });
});

test('coverage and capability companions cover every row and ledger key exactly', () => {
  const workloads = phase7WorkloadSpecs(ROOT);
  const ids = new Set(workloads.map(({ id }) => id));
  const coverage = phase7CoverageMap(ROOT);
  assert.deepEqual(Object.keys(coverage).sort(), [...PHASE7_COVERAGE_ROWS].sort());
  for (const row of PHASE7_COVERAGE_ROWS) {
    assert.equal(coverage[row].length > 0, true, `${row} must be non-empty`);
    assert.equal(coverage[row].every((id) => ids.has(id)), true, `${row} unknown id`);
  }
  const ledger = phase7CapabilityLedger(ROOT);
  assert.doesNotThrow(() => assertPhase7CapabilityLedger(ledger));
  assert.deepEqual(Object.keys(ledger).sort(), [
    'Actor',
    'Actor-compaction',
    'DB',
    'callback-cross-runtime',
    'callback-same-runtime',
    'interface-local',
    'interface-remote',
    'recoverable',
    'request-GC',
    'service',
    'task-Actor',
    'task-function',
  ]);
  const companions = phase7CapabilityCompanions(ROOT);
  for (const key of Object.keys(ledger)) {
    const row = companions[key];
    assert.ok(row, key);
    assert.equal(ids.has(row.companion), true, `${key} companion`);
    const state = typeof ledger[key] === 'object' ? ledger[key].state : ledger[key];
    assert.equal(state === 'accepted' ? 'positive' : 'negative', row.polarity, key);
  }
  for (const key of ['request-GC', 'Actor-compaction']) {
    assert.equal(ledger[key].disposition, 'deferred');
  }
});

test('dependency graph is acyclic and orders the consumer after its producer', () => {
  const order = assertPhase7DependencyGraph(phase7WorkloadSpecs(ROOT));
  assert.equal(order.length, phase7WorkloadSpecs(ROOT).length);
  const producer = phase7WorkloadSpecs(ROOT).find(({ id }) => id === 'phase-7-whole-system-producer');
  const consumer = phase7WorkloadSpecs(ROOT).find(({ id }) => id === 'phase-7-whole-system-consumer');
  assert.deepEqual(producer.producedArtifacts, ['phase-7-whole-system-composition']);
  assert.deepEqual(consumer.dependsOn, ['phase-7-whole-system-producer']);
  assert.deepEqual(consumer.requiredArtifacts, ['phase-7-whole-system-composition']);
  assert.equal(order.indexOf('phase-7-whole-system-producer')
    < order.indexOf('phase-7-whole-system-consumer'), true);
  assert.doesNotThrow(() => assertPhase7Catalog(ROOT));
  const execution = phase7ExecutionOrder(ROOT);
  assert.deepEqual(execution.slice(0, 3),
    ['preflight-head', 'preflight-tree', 'preflight-status']);
  assert.deepEqual(execution.slice(-3), ['fresh-head', 'fresh-tree', 'fresh-status']);
  assert.equal(execution.length, 128);
});

test('catalog digest is deterministic and binds the full catalog', () => {
  const first = phase7SpecCatalogDigest(ROOT);
  assert.equal(first, phase7SpecCatalogDigest(ROOT));
  const catalog = phase7SpecCatalog(ROOT);
  assert.equal(catalog.schemaVersion, PHASE7_CATALOG_SCHEMA);
  assert.equal(catalog.digest, first);
  assert.equal(catalog.provenance.length, catalog.specs.length);
  assert.equal(catalog.adapter.rows.length, 95);
  assert.equal(Object.keys(catalog.coverage).length, 18);
  assert.deepEqual(Object.keys(catalog.ledger).length, 12);
  assert.equal(catalog.specs.some(({ id }) => id === 'phase-7-whole-system-consumer'), true);
});

test('lane coverage spans Phase 6 plus Phase 7 obligations', () => {
  const workloads = phase7WorkloadSpecs(ROOT);
  const observed = new Set(workloads.flatMap(({ lanes }) => lanes));
  for (const lane of [...PHASE6_REQUIRED_LANES, ...PHASE7_REQUIRED_LANES]) {
    assert.equal(observed.has(lane), true, `${lane} must be covered`);
  }
});

test('inherited cargo test args keep exactly one idempotent --no-fail-fast', () => {
  for (const spec of phase7WorkloadSpecs(ROOT)) {
    if (spec.command === 'cargo' && spec.args[0] === 'test') {
      assert.equal(spec.args[0], 'test');
      assert.equal(spec.args.filter((arg) => arg === '--no-fail-fast').length, 1, spec.id);
    }
  }
});

test('public verify selector reaches the exclusive Phase 7 r1 Gate runner', async () => {
  assert.equal(PUBLIC_SELECTORS.includes('bytecode-vm-phase-7-gate'), true);
  const plan = await buildVerifyPlan({
    root: REPOSITORY,
    selectors: ['bytecode-vm-phase-7-gate'],
    catalogRoot: REPOSITORY,
  });
  assert.deepEqual(plan.tasks, [{
    id: 'bytecode-vm-phase-7:gate',
    kind: 'implementation:runtime',
    command: 'node',
    args: ['scripts/run-bytecode-vm-phase-7-gate.mjs'],
    cwd: REPOSITORY,
    exclusive: true,
    slots: 1,
  }]);
  assert.equal(phase7CandidateSpecs(ROOT).length + phase7WorkloadSpecs(ROOT).length, 128);
});

test('no-verifier structural checker accepts the clean Phase 7 candidate', async () => {
  await assertPhase6NoVerifierStructural(REPOSITORY);
});