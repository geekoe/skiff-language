import assert from 'node:assert/strict';
import test from 'node:test';

import { validSha256 } from '../lib/bytecode-vm-phase-1-contract.mjs';
import {
  PHASE1_OBSERVATION_KINDS,
  PHASE1_OBSERVATION_ORDER,
  PHASE1_OBSERVATION_QUEUE_CAPACITY_REFERENCE,
  PHASE1_OBSERVATION_SCHEMA_VERSION,
  PHASE1_PRODUCTION_OBSERVATION_MAX,
  phase1ObservationSchemaContent,
  phase1ObservationSchemaIdentity,
  validatePhase1ObservationStream,
} from '../lib/bytecode-vm-phase-1-observation-schema.mjs';

function deploymentRef(overrides = {}) {
  return {
    serviceId: 'example.com/service',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision',
    deploymentArtifactIdentity: 'deployment',
    ...overrides,
  };
}

function ownerInventory(overrides = {}) {
  return {
    pending: { current: 0, everCreated: false },
    resource: { current: 0, everCreated: false },
    child: { current: 0, everCreated: false },
    ...overrides,
  };
}

function canonicalStream() {
  return [
    { kind: 'DeploymentImageSelected', payload: { deployment: deploymentRef(), deploymentBuildId: 'build' } },
    {
      kind: 'RouteEntryPinned',
      payload: {
        imageOwner: deploymentRef(),
        selector: { kind: 'Gateway', identity: { protocol: 'Http', method: 'POST', path: '/call' } },
        gatewayKey: 'key',
        gatewayIdentity: `skiff-gateway-entry-v2:sha256:${'a'.repeat(64)}`,
        callableRole: 'Handler',
        verifiedFunctionIndex: 1,
      },
    },
    { kind: 'VmFunctionFrameEntered', payload: { role: 'Root', functionIndex: 1, frameDepth: 1, slotCount: 2 } },
    {
      kind: 'VmFirstInstructionDispatched',
      payload: {
        imageOwner: deploymentRef(),
        rootEntryFunctionIndex: 1,
        currentFunctionIndex: 1,
        instructionIndex: 0,
        opcode: 'LoadSlot',
      },
    },
    {
      kind: 'VmLocalCallDispatched',
      payload: { callerFunctionIndex: 1, calleeFunctionIndex: 2, callerFrameDepth: 1, calleeFrameDepth: 2 },
    },
    {
      kind: 'VmFunctionFrameEntered',
      payload: { role: 'FirstRootLocalCallee', functionIndex: 2, frameDepth: 2, slotCount: 1 },
    },
    {
      kind: 'VmFunctionReturned',
      payload: { role: 'FirstRootLocalCallee', functionIndex: 2, callerFunctionIndex: 1, remainingFrameDepth: 1 },
    },
    {
      kind: 'VmFunctionReturned',
      payload: { role: 'Root', functionIndex: 1, callerFunctionIndex: null, remainingFrameDepth: 0 },
    },
    {
      kind: 'VmBudgetAccounted',
      payload: { rawExecutedCount: 11, chargedInstructionCount: 11, hardLimit: 100000, pollCount: 2 },
    },
    { kind: 'RequestTerminalClaimed', payload: { terminal: 'Succeeded' } },
    { kind: 'RequestCleanupComplete', payload: { ownerInventory: ownerInventory() } },
  ];
}

test('valid canonical eleven-event stream passes schema validation', () => {
  assert.deepEqual(validatePhase1ObservationStream(canonicalStream()), {
    valid: true,
    failures: [],
  });
});

test('streams longer than the production maximum are rejected', () => {
  const events = canonicalStream();
  events.splice(0, 0, events[0]);
  const result = validatePhase1ObservationStream(events);
  assert.equal(result.valid, false);
  assert.equal(result.failures.some((message) => /exceeds the production maximum 11/.test(message)), true);
});

test('events missing a kind are rejected', () => {
  const result = validatePhase1ObservationStream([{ payload: {} }]);
  assert.equal(result.valid, false);
  assert.equal(result.failures.some((message) => /missing a kind/.test(message)), true);
});

test('unknown kinds are rejected', () => {
  const result = validatePhase1ObservationStream([{ kind: 'NotAKind', payload: {} }]);
  assert.equal(result.valid, false);
  assert.equal(result.failures.some((message) => /unknown kind "NotAKind"/.test(message)), true);
});

test('duplicate kinds beyond their declared cardinality are rejected', () => {
  const events = [
    { kind: 'RequestTerminalClaimed', payload: { terminal: 'Succeeded' } },
    { kind: 'RequestTerminalClaimed', payload: { terminal: 'Succeeded' } },
  ];
  const result = validatePhase1ObservationStream(events);
  assert.equal(result.valid, false);
  assert.equal(
    result.failures.some((message) => /RequestTerminalClaimed appears 2 times, exceeding the declared maximum 1/.test(message)),
    true,
  );

  const frames = canonicalStream().filter(({ kind }) => kind === 'VmFunctionFrameEntered');
  const overFramed = validatePhase1ObservationStream([
    frames[0], frames[1], frames[0],
  ]);
  assert.equal(overFramed.valid, false);
  assert.equal(
    overFramed.failures.some((message) => /VmFunctionFrameEntered appears 3 times, exceeding the declared maximum 2/.test(message)),
    true,
  );
});

test('cleanup that is not the final observation is rejected', () => {
  const events = canonicalStream();
  const cleanup = events.pop();
  const terminal = events.pop();
  events.push(cleanup);
  events.push(terminal);
  const result = validatePhase1ObservationStream(events);
  assert.equal(result.valid, false);
  assert.equal(
    result.failures.some((message) => /RequestCleanupComplete must be the final observation/.test(message)),
    true,
  );
});

test('missing and mistyped payload fields are rejected', () => {
  const stream = canonicalStream();
  stream[4].payload.callerFrameDepth = '1';
  const wrongType = validatePhase1ObservationStream(stream);
  assert.equal(wrongType.valid, false);
  assert.equal(
    wrongType.failures.some((message) => /payload.callerFrameDepth must be a safe integer/.test(message)),
    true,
  );

  const budget = canonicalStream()[8];
  delete budget.payload.hardLimit;
  const missing = validatePhase1ObservationStream([budget]);
  assert.equal(missing.valid, false);
  assert.equal(missing.failures.some((message) => /payload.hardLimit is missing/.test(message)), true);

  const frame = canonicalStream()[2];
  frame.payload.role = 'Helper';
  const badRole = validatePhase1ObservationStream([frame]);
  assert.equal(badRole.valid, false);
  assert.equal(
    badRole.failures.some((message) => /payload.role must be one of Root, FirstRootLocalCallee/.test(message)),
    true,
  );
});

test('nested cleanup wire shape is validated exactly', () => {
  const cleanup = canonicalStream()[10];

  assert.equal(validatePhase1ObservationStream([cleanup]).valid, true);

  const missingDomain = structuredClone(cleanup);
  delete missingDomain.payload.ownerInventory.resource;
  const missingDomainResult = validatePhase1ObservationStream([missingDomain]);
  assert.equal(missingDomainResult.valid, false);
  assert.equal(
    missingDomainResult.failures.some((message) => /payload.ownerInventory.resource is missing/.test(message)),
    true,
  );

  const wrongCurrent = structuredClone(cleanup);
  wrongCurrent.payload.ownerInventory.child.current = '0';
  const wrongCurrentResult = validatePhase1ObservationStream([wrongCurrent]);
  assert.equal(wrongCurrentResult.valid, false);
  assert.equal(
    wrongCurrentResult.failures.some((message) => /payload.ownerInventory.child.current must be a safe integer/.test(message)),
    true,
  );

  const wrongBit = structuredClone(cleanup);
  wrongBit.payload.ownerInventory.pending.everCreated = 'false';
  const wrongBitResult = validatePhase1ObservationStream([wrongBit]);
  assert.equal(wrongBitResult.valid, false);
  assert.equal(
    wrongBitResult.failures.some((message) => /payload.ownerInventory.pending.everCreated must be a boolean/.test(message)),
    true,
  );

  const absentInventory = structuredClone(cleanup);
  absentInventory.payload.ownerInventory = null;
  const absentInventoryResult = validatePhase1ObservationStream([absentInventory]);
  assert.equal(absentInventoryResult.valid, false);
  assert.equal(
    absentInventoryResult.failures.some((message) => /payload.ownerInventory must be an object/.test(message)),
    true,
  );
});

test('schema declaration matches the adjudicated nine-kind, eleven-event sequence field-by-field', () => {
  const expectedKinds = [
    ['DeploymentImageSelected', ['deployment', 'deploymentBuildId'], 1],
    ['RouteEntryPinned', ['imageOwner', 'selector', 'gatewayKey', 'gatewayIdentity', 'callableRole', 'verifiedFunctionIndex'], 1],
    ['VmFunctionFrameEntered', ['role', 'functionIndex', 'frameDepth', 'slotCount'], 2],
    ['VmFirstInstructionDispatched', ['imageOwner', 'rootEntryFunctionIndex', 'currentFunctionIndex', 'instructionIndex', 'opcode'], 1],
    ['VmLocalCallDispatched', ['callerFunctionIndex', 'calleeFunctionIndex', 'callerFrameDepth', 'calleeFrameDepth'], 1],
    ['VmFunctionReturned', ['role', 'functionIndex', 'callerFunctionIndex', 'remainingFrameDepth'], 2],
    ['VmBudgetAccounted', ['rawExecutedCount', 'chargedInstructionCount', 'hardLimit', 'pollCount'], 1],
    ['RequestTerminalClaimed', ['terminal'], 1],
    ['RequestCleanupComplete', ['ownerInventory'], 1],
  ];
  assert.deepEqual(Object.keys(PHASE1_OBSERVATION_KINDS), expectedKinds.map(([name]) => name));
  for (const [name, fields, maxCount] of expectedKinds) {
    const spec = PHASE1_OBSERVATION_KINDS[name];
    assert.deepEqual(Object.keys(spec.fields), fields, name);
    assert.equal(spec.maxCount, maxCount, name);
  }
  assert.deepEqual(PHASE1_OBSERVATION_KINDS.VmFunctionFrameEntered.roles, ['Root', 'FirstRootLocalCallee']);
  assert.deepEqual(PHASE1_OBSERVATION_KINDS.VmFunctionReturned.roles, ['Root', 'FirstRootLocalCallee']);

  assert.deepEqual(
    PHASE1_OBSERVATION_ORDER,
    [
      { ordinal: 0, kind: 'DeploymentImageSelected' },
      { ordinal: 1, kind: 'RouteEntryPinned' },
      { ordinal: 2, kind: 'VmFunctionFrameEntered', role: 'Root' },
      { ordinal: 3, kind: 'VmFirstInstructionDispatched' },
      { ordinal: 4, kind: 'VmLocalCallDispatched' },
      { ordinal: 5, kind: 'VmFunctionFrameEntered', role: 'FirstRootLocalCallee' },
      { ordinal: 6, kind: 'VmFunctionReturned', role: 'FirstRootLocalCallee' },
      { ordinal: 7, kind: 'VmFunctionReturned', role: 'Root' },
      { ordinal: 8, kind: 'VmBudgetAccounted' },
      { ordinal: 9, kind: 'RequestTerminalClaimed' },
      { ordinal: 10, kind: 'RequestCleanupComplete' },
    ],
  );
  assert.deepEqual(
    Object.keys(PHASE1_OBSERVATION_KINDS.RequestCleanupComplete.fields.ownerInventory.fields),
    ['pending', 'resource', 'child'],
  );
  for (const domain of ['pending', 'resource', 'child']) {
    assert.deepEqual(
      Object.keys(PHASE1_OBSERVATION_KINDS.RequestCleanupComplete.fields.ownerInventory.fields[domain].fields),
      ['current', 'everCreated'],
    );
  }
  assert.deepEqual(
    Object.keys(PHASE1_OBSERVATION_KINDS.RouteEntryPinned.fields.selector.variants),
    ['Operation', 'Gateway'],
  );
});

test('schema identity is a stable versioned sha256 of the canonical content', () => {
  const identity = phase1ObservationSchemaIdentity();
  assert.equal(identity.version, PHASE1_OBSERVATION_SCHEMA_VERSION);
  assert.equal(validSha256(identity.sha256), true);
  assert.deepEqual(phase1ObservationSchemaIdentity(), identity);
  assert.deepEqual(phase1ObservationSchemaContent(), phase1ObservationSchemaContent());
  assert.equal(phase1ObservationSchemaContent().productionMax, 11);
  assert.equal(phase1ObservationSchemaContent().queueCapacityReference, 16);
  assert.equal(phase1ObservationSchemaContent().sequence.length, 11);
});

test('production maximum and queue capacity are recorded as facts, not VM semantics', () => {
  assert.equal(PHASE1_PRODUCTION_OBSERVATION_MAX, 11);
  assert.equal(PHASE1_OBSERVATION_QUEUE_CAPACITY_REFERENCE, 16);
  assert.equal(
    PHASE1_PRODUCTION_OBSERVATION_MAX <= PHASE1_OBSERVATION_QUEUE_CAPACITY_REFERENCE,
    true,
  );
});

test('the validator does not judge VM, budget or cleanup semantics', () => {
  const stream = canonicalStream();
  stream[8].payload = {
    rawExecutedCount: 0,
    chargedInstructionCount: 999,
    hardLimit: 0,
    pollCount: 0,
  };
  stream[9].payload.terminal = 'Cancelled';
  stream[10].payload.ownerInventory = ownerInventory({
    pending: { current: 3, everCreated: true },
  });
  assert.deepEqual(validatePhase1ObservationStream(stream), {
    valid: true,
    failures: [],
  });
});
