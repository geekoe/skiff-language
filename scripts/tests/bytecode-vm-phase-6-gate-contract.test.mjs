import assert from 'node:assert/strict';
import { resolve, dirname } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  assertPhase6BoundedWorkCoverage,
  assertPhase6LaneCoverage,
  assertPhase6ProvenanceCoverage,
  PHASE6_COMMAND_SCHEMA,
  PHASE6_MANIFEST_SCHEMA,
  PHASE6_REQUIRED_LANES,
  phase6BoundedWorkLedger,
  phase6CandidateSpecs,
  phase6ScenarioSpecs,
  phase6WorkloadProvenance,
  phase6WorkloadSpecs,
} from '../lib/bytecode-vm-phase-6-contract.mjs';
import {
  PHASE6_DIRECTORY_IDENTITY_FILE,
  PHASE6_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-6-evidence-root.mjs';
import { buildVerifyPlan, PUBLIC_SELECTORS } from '../lib/verify-plan.mjs';

const ROOT = '/candidate';
const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

test('r1 schemas cannot accept earlier Phase 6 receipts', () => {
  assert.equal(PHASE6_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-6-command-r1-v1');
  assert.equal(PHASE6_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-6-gate-r1-v1');
  assert.equal(PHASE6_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-6-directory-identity-r1-v1');
  assert.equal(PHASE6_DIRECTORY_IDENTITY_FILE, 'phase-6-r1-v1-directory-identities.json');
});

test('r1 matrix names every capability and keeps expectedTests nonzero', () => {
  const scenarios = phase6ScenarioSpecs(ROOT);
  const workloads = phase6WorkloadSpecs(ROOT);
  assert.equal(scenarios.length, 16);
  assert.equal(workloads.length, 111);
  assert.doesNotThrow(() => assertPhase6LaneCoverage(workloads));
  const observed = new Set(workloads.flatMap(({ lanes }) => lanes));
  for (const lane of PHASE6_REQUIRED_LANES) {
    assert.equal(observed.has(lane), true, `${lane} must be covered`);
  }
  for (const scenario of scenarios) {
    if (scenario.testFormat !== null) {
      assert.equal(Number.isSafeInteger(scenario.expectedTests)
        && scenario.expectedTests > 0, true, `${scenario.id} needs positive expectedTests`);
    }
    assert.deepEqual(scenario.sourcePhase, 6);
    assert.deepEqual(scenario.sourceId, scenario.id);
    assert.deepEqual(scenario.originChain, [{ phase: 6, id: scenario.id }]);
  }
  const capabilityIds = [
    'p6-service-matrix',
    'p6-interface-local-matrix',
    'p6-interface-remote-matrix',
    'p6-callback-matrix',
    'p6-recoverable-matrix',
    'p6-db-matrix',
    'p6-task-host-matrix',
    'p6-task-router-matrix',
    'p6-actor-host-matrix',
    'p6-actor-router-matrix',
    'p6-containment-matrix',
    'p6-kernel-focused',
  ];
  for (const expectedId of capabilityIds) {
    assert.equal(scenarios.some(({ id }) => id === expectedId), true, expectedId);
  }
});

test('inherited specs preserve Phase 5 and normalize only cargo test args', () => {
  const inherited = phase6WorkloadSpecs(ROOT)
    .filter(({ id }) => id.startsWith('phase-5-regression-'));
  assert.equal(inherited.length, 95);
  assert.equal(inherited.every(({ parentPhase }) => parentPhase === 5), true);
  assert.equal(inherited.every(({ lanes }) => lanes.includes('phase-5-regression')), true);
  for (const entry of inherited) {
    assert.equal(entry.sourcePhase >= 1 && entry.sourcePhase <= 5, true);
    assert.equal(entry.originChain.at(-1).id, entry.id);
    assert.equal(entry.originChain.at(-1).phase, 6);
    if (entry.command === 'cargo' && entry.args[0] === 'test') {
      assert.equal(entry.args.includes('--no-fail-fast'), true);
      assert.equal(
        entry.args.filter((argument) => argument === '--no-fail-fast').length,
        1,
      );
      assert.equal(entry.args[0], 'test');
    }
  }
  const missingTests = inherited.find(({ id }) => id === 'phase-5-regression-phase-5-gate-self-tests');
  assert.equal(Object.hasOwn(missingTests, 'expectedTests'), false);
  assert.equal(missingTests.expectedTests, undefined);
  const provenance = phase6WorkloadProvenance(ROOT);
  assert.doesNotThrow(() => assertPhase6ProvenanceCoverage(phase6WorkloadSpecs(ROOT), provenance));
  const ledger = phase6BoundedWorkLedger(ROOT);
  assert.doesNotThrow(() => assertPhase6BoundedWorkCoverage(phase6WorkloadSpecs(ROOT), ledger));
  assert.deepEqual([...new Set(provenance.map(({ sourcePhase }) => sourcePhase))].sort(),
    [1, 2, 3, 4, 5, 6]);
});

test('public verify selector reaches the exclusive Phase 6 r1 Gate runner', async () => {
  assert.equal(PUBLIC_SELECTORS.includes('bytecode-vm-phase-6-gate'), true);
  const plan = await buildVerifyPlan({
    root: REPOSITORY,
    selectors: ['bytecode-vm-phase-6-gate'],
    catalogRoot: REPOSITORY,
  });
  assert.deepEqual(plan.tasks, [{
    id: 'bytecode-vm-phase-6:gate',
    kind: 'implementation:runtime',
    command: 'node',
    args: ['scripts/run-bytecode-vm-phase-6-gate.mjs'],
    cwd: REPOSITORY,
    exclusive: true,
  }]);
  assert.equal(phase6CandidateSpecs(ROOT).length + phase6WorkloadSpecs(ROOT).length, 123);
});
