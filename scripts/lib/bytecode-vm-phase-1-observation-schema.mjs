import { sha256 } from './bytecode-vm-phase-1-contract.mjs';

export const PHASE1_OBSERVATION_SCHEMA_VERSION =
  'skiff-bytecode-vm-phase-1-observation-v1';

// Phase 1 production maximum: eleven observations per admitted root request.
// This is the declared maximum the Gate may validate. It must not be used to
// infer VM, budget or cleanup semantics, which remain the T-R Rust proof's
// sole authority.
export const PHASE1_PRODUCTION_OBSERVATION_MAX = 11;

// Documented production fact from the Rust model
// (runtime/model/src/bytecode_execution_observation.rs). The five spare slots
// are queue headroom, not permission to mint more events. The Gate records the
// bound as a fact only and performs no VM-semantics inference from it.
export const PHASE1_OBSERVATION_QUEUE_CAPACITY_REFERENCE = 16;

// ---- Field type descriptors. One source of truth for both the schema
// declaration and the pure shape validator. ----

const STRING = Object.freeze({ type: 'string' });
const INTEGER = Object.freeze({ type: 'integer' });
const BOOLEAN = Object.freeze({ type: 'boolean' });

const FRAME_ROLE = Object.freeze({
  type: 'enum',
  values: ['Root', 'FirstRootLocalCallee'],
});

const REQUEST_TERMINAL = Object.freeze({
  type: 'enum',
  values: ['Succeeded', 'Failed', 'Cancelled'],
});

const CALLABLE_ROLE = Object.freeze({
  type: 'enum',
  values: ['Handler', 'Pre', 'Guard', 'CloseHandler'],
});

const INGRESS_PROTOCOL = Object.freeze({
  type: 'enum',
  values: ['Http', 'WebSocket'],
});

const SERVICE_DEPLOYMENT_REF = Object.freeze({
  type: 'object',
  fields: Object.freeze({
    serviceId: STRING,
    contractVersion: STRING,
    deploymentRevision: STRING,
    deploymentArtifactIdentity: STRING,
  }),
});

const INGRESS_SELECTOR = Object.freeze({
  type: 'object',
  fields: Object.freeze({
    protocol: INGRESS_PROTOCOL,
    method: Object.freeze({ type: 'nullable', of: STRING }),
    path: STRING,
  }),
});

// Wire shape of BytecodeRouteEntrySelector:
// { "kind": "Operation", "identity": "<ContractOperationId>" }
// { "kind": "Gateway",   "identity": <IngressSelector> }
const ROUTE_ENTRY_SELECTOR = Object.freeze({
  type: 'tagged',
  tag: 'kind',
  variants: Object.freeze({
    Operation: STRING,
    Gateway: INGRESS_SELECTOR,
  }),
});

const FROZEN_OWNER_DOMAIN = Object.freeze({
  type: 'object',
  fields: Object.freeze({
    current: INTEGER,
    everCreated: BOOLEAN,
  }),
});

const OWNER_INVENTORY = Object.freeze({
  type: 'object',
  fields: Object.freeze({
    pending: FROZEN_OWNER_DOMAIN,
    resource: FROZEN_OWNER_DOMAIN,
    child: FROZEN_OWNER_DOMAIN,
    childHeap: FROZEN_OWNER_DOMAIN,
    boundary: FROZEN_OWNER_DOMAIN,
    actor: FROZEN_OWNER_DOMAIN,
  }),
});

// Nine kind names carrying eleven event instances, with the exact payload
// field names and wire types declared by DEC1-O and produced by
// runtime/model/src/bytecode_execution_observation.rs. `maxCount` is the
// per-kind cardinality for one root request; `roles` records the two allowed
// VmObservedFrameRole values where applicable.
export const PHASE1_OBSERVATION_KINDS = Object.freeze({
  DeploymentImageSelected: kind({ maxCount: 1 }, {
    deployment: SERVICE_DEPLOYMENT_REF,
    deploymentBuildId: STRING,
  }),
  RouteEntryPinned: kind({ maxCount: 1 }, {
    imageOwner: SERVICE_DEPLOYMENT_REF,
    selector: ROUTE_ENTRY_SELECTOR,
    gatewayKey: Object.freeze({ type: 'nullable', of: STRING }),
    gatewayIdentity: Object.freeze({ type: 'nullable', of: STRING }),
    callableRole: Object.freeze({ type: 'nullable', of: CALLABLE_ROLE }),
    verifiedFunctionIndex: INTEGER,
  }),
  VmFunctionFrameEntered: kind({
    maxCount: 2,
    roles: ['Root', 'FirstRootLocalCallee'],
  }, {
    role: FRAME_ROLE,
    functionIndex: INTEGER,
    frameDepth: INTEGER,
    slotCount: INTEGER,
  }),
  VmFirstInstructionDispatched: kind({ maxCount: 1 }, {
    imageOwner: SERVICE_DEPLOYMENT_REF,
    rootEntryFunctionIndex: INTEGER,
    currentFunctionIndex: INTEGER,
    instructionIndex: INTEGER,
    opcode: STRING,
  }),
  VmLocalCallDispatched: kind({ maxCount: 1 }, {
    callerFunctionIndex: INTEGER,
    calleeFunctionIndex: INTEGER,
    callerFrameDepth: INTEGER,
    calleeFrameDepth: INTEGER,
  }),
  VmFunctionReturned: kind({
    maxCount: 2,
    roles: ['Root', 'FirstRootLocalCallee'],
  }, {
    role: FRAME_ROLE,
    functionIndex: INTEGER,
    callerFunctionIndex: Object.freeze({ type: 'nullable', of: INTEGER }),
    remainingFrameDepth: INTEGER,
  }),
  VmBudgetAccounted: kind({ maxCount: 1 }, {
    rawExecutedCount: INTEGER,
    chargedInstructionCount: INTEGER,
    hardLimit: INTEGER,
    pollCount: INTEGER,
  }),
  RequestTerminalClaimed: kind({ maxCount: 1 }, {
    terminal: REQUEST_TERMINAL,
  }),
  RequestCleanupComplete: kind({ maxCount: 1 }, {
    ownerInventory: OWNER_INVENTORY,
  }),
});

// The adjudicated Phase 1 scalar ordinals from DEC1-O. This is a recorded
// schema fact, not a semantic assertion the Gate re-derives.
export const PHASE1_OBSERVATION_ORDER = Object.freeze([
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
]);

export function phase1ObservationSchemaContent() {
  return Object.freeze({
    schemaVersion: PHASE1_OBSERVATION_SCHEMA_VERSION,
    productionMax: PHASE1_PRODUCTION_OBSERVATION_MAX,
    queueCapacityReference: PHASE1_OBSERVATION_QUEUE_CAPACITY_REFERENCE,
    kinds: PHASE1_OBSERVATION_KINDS,
    sequence: PHASE1_OBSERVATION_ORDER,
  });
}

export function phase1ObservationSchemaIdentity() {
  return Object.freeze({
    version: PHASE1_OBSERVATION_SCHEMA_VERSION,
    sha256: sha256(JSON.stringify(phase1ObservationSchemaContent())),
  });
}

/**
 * Pure schema/shape/count/order validation of one Phase 1 observation stream.
 *
 * Input is an array of the typed event envelopes (`{ kind, payload }`) from
 * DEC1-O. The validator rejects streams longer than the production maximum,
 * events without a known kind, kind counts above their declared cardinality,
 * missing or mistyped payload fields, and a RequestCleanupComplete that is not
 * the final observation. It deliberately performs no VM, budget or cleanup
 * semantic judgment; those facts are the T-R Rust proof's sole authority.
 */
export function validatePhase1ObservationStream(events) {
  const failures = [];
  if (!Array.isArray(events)) {
    return { valid: false, failures: ['observations must be an array'] };
  }
  if (events.length > PHASE1_PRODUCTION_OBSERVATION_MAX) {
    failures.push(
      `observation count ${events.length} exceeds the production maximum `
      + `${PHASE1_PRODUCTION_OBSERVATION_MAX}`,
    );
  }
  const counts = new Map();
  const cleanupIndexes = [];
  events.forEach((event, index) => {
    if (!isPlainObject(event)) {
      failures.push(`observation ${index} must be an object`);
      return;
    }
    if (typeof event.kind !== 'string' || event.kind.length === 0) {
      failures.push(`observation ${index} is missing a kind`);
      return;
    }
    const spec = PHASE1_OBSERVATION_KINDS[event.kind];
    if (spec === undefined) {
      failures.push(`observation ${index} has unknown kind ${JSON.stringify(event.kind)}`);
      return;
    }
    counts.set(event.kind, (counts.get(event.kind) ?? 0) + 1);
    if (!isPlainObject(event.payload)) {
      failures.push(`observation ${index} (${event.kind}) payload must be an object`);
      return;
    }
    for (const [field, descriptor] of Object.entries(spec.fields)) {
      const error = fieldError(event.payload[field], descriptor, `payload.${field}`);
      if (error !== null) {
        failures.push(`observation ${index} (${event.kind}): ${error}`);
      }
    }
    if (event.kind === 'RequestCleanupComplete') cleanupIndexes.push(index);
  });
  for (const [kind, count] of counts) {
    const { maxCount } = PHASE1_OBSERVATION_KINDS[kind];
    if (count > maxCount) {
      failures.push(
        `kind ${kind} appears ${count} times, exceeding the declared maximum ${maxCount}`,
      );
    }
  }
  for (const index of cleanupIndexes) {
    if (index !== events.length - 1) {
      failures.push(
        `RequestCleanupComplete must be the final observation (found at ${index} `
        + `of ${events.length - 1})`,
      );
    }
  }
  return { valid: failures.length === 0, failures };
}

function kind(options, fields) {
  return Object.freeze({
    maxCount: options.maxCount,
    ...(options.roles === undefined ? {} : { roles: Object.freeze([...options.roles]) }),
    fields: Object.freeze(fields),
  });
}

function fieldError(value, spec, path) {
  if (value === undefined) return `${path} is missing`;
  switch (spec.type) {
    case 'string':
      return typeof value === 'string' ? null : `${path} must be a string`;
    case 'boolean':
      return typeof value === 'boolean' ? null : `${path} must be a boolean`;
    case 'integer':
      return Number.isSafeInteger(value) ? null : `${path} must be a safe integer`;
    case 'enum':
      return typeof value === 'string' && spec.values.includes(value)
        ? null
        : `${path} must be one of ${spec.values.join(', ')}`;
    case 'nullable':
      return value === null ? null : fieldError(value, spec.of, path);
    case 'object': {
      if (!isPlainObject(value)) return `${path} must be an object`;
      for (const [name, sub] of Object.entries(spec.fields)) {
        const error = fieldError(value[name], sub, `${path}.${name}`);
        if (error !== null) return error;
      }
      return null;
    }
    case 'tagged': {
      if (!isPlainObject(value)) return `${path} must be an object`;
      const tag = value[spec.tag];
      if (typeof tag !== 'string' || !(tag in spec.variants)) {
        return `${path} must carry a valid ${spec.tag}`;
      }
      return fieldError(value.identity, spec.variants[tag], `${path}.identity`);
    }
    default:
      return `${path} has an unsupported schema descriptor ${JSON.stringify(spec?.type)}`;
  }
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
