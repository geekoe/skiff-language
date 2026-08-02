// E-actor-parity differential constants (plan §7/§8/§9, batch 10).
//
// The actor parity scenarios live in their own inventory
// (`scripts/fixtures/router-differential/actor_parity_inventory.json`) with
// the `actor_parity_*` prefix; the shared scenario inventory remains owned by
// the differential extension node. Every side gets independent ports, artifact
// root, runtime home and Mongo namespace, two real Runtime replicas through
// test-only relays, and the same real HTTP full-chain driver.

import {
  ACTOR_ROUTING_PROJECTION_RECORD_PATH,
  ENVIRONMENT,
  GENERATION,
  ROUTER_PORT_MAX,
  ROUTER_PORT_MIN,
  TS_AUDIT_COLLECTION,
  TS_DATABASE,
  TS_STATE_COLLECTION,
  RUST_AUDIT_COLLECTION,
  RUST_DATABASE,
  RUST_STATE_COLLECTION,
} from './constants.mjs';

export {
  ACTOR_ROUTING_PROJECTION_RECORD_PATH,
  ENVIRONMENT,
  GENERATION,
  ROUTER_PORT_MAX,
  ROUTER_PORT_MIN,
  RUST_AUDIT_COLLECTION,
  RUST_DATABASE,
  RUST_STATE_COLLECTION,
  TS_AUDIT_COLLECTION,
  TS_DATABASE,
  TS_STATE_COLLECTION,
};

export const ACTOR_PARITY_SCHEMA_VERSION =
  'skiff-router-differential-inventory-v1';
export const ACTOR_PARITY_BASELINE =
  'edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a';
export const ACTOR_PARITY_ENVIRONMENT = 'actor-parity';
export const ACTOR_PARITY_GENERATION = 1;
export const ACTOR_PARITY_REPLICA_ONE_ID = 'actor-parity-replica-1';
export const ACTOR_PARITY_REPLICA_TWO_ID = 'actor-parity-replica-2';
export const ACTOR_PARITY_PORTS_PER_SIDE = 4;

export const ACTOR_PARITY_INVENTORY_REPO_PATH =
  'scripts/fixtures/router-differential/actor_parity_inventory.json';

export const ACTOR_PARITY_SERVICE_SOURCE_FIXTURE =
  'test-runner/fixtures/actor-full-chain-acceptance';

// Implementation-neutral HTTP full-chain steps in frozen driver order. Each
// step records `{ name, status, bodyNorm, bodyRaw }`; poll steps wait for a
// deterministic actor value. This mirrors the TS full-chain acceptance driver
// (scripts/lib/actor-full-chain-acceptance-real.mjs) and the Rust
// actor_live_probe happy path, so both implementations receive identical
// business input through real HTTP.
export const ACTOR_PARITY_STEPS = Object.freeze([
  { name: 'probe-1', entrypoint: 'probe', expectStatus: 200, expectBody: 'actor-count-1' },
  { name: 'probe-2', entrypoint: 'probe', expectStatus: 200, expectBody: 'actor-count-next' },
  { name: 'slow-get', entrypoint: 'slowGet', expectStatus: 200, expectBody: 'slow-get-ok', minElapsedMs: 200 },
  { name: 'slow-increment-1', entrypoint: 'slowIncrement', expectStatus: 200, expectBody: 'slow-ok' },
  { name: 'slow-increment-2', entrypoint: 'slowIncrement', expectStatus: 200, expectBody: 'slow-ok' },
  { name: 'synchronous-self-call', entrypoint: 'synchronousSelfCall', expectStatus: 200, expectBody: 105 },
  { name: 'synchronous-self-count', entrypoint: 'synchronousSelfCount', expectStatus: 200, expectBody: 105 },
  { name: 'spawn-external', entrypoint: 'spawnExternal', expectStatus: 200, expectBody: 'external-submitted', maxElapsedMs: 250 },
  { name: 'spawn-self-kick', entrypoint: 'spawnSelfKick', expectStatus: 200, expectBody: 'kicked' },
  { name: 'spawn-fanout', entrypoint: 'spawnFanout', expectStatus: 200, expectBody: 'fanned' },
  { name: 'chain-kick', entrypoint: 'chainKick', expectStatus: 200, expectBody: 'chain-kicked', maxElapsedMs: 250 },
  { name: 'spawn-throw', entrypoint: 'spawnThrow', expectStatus: 200, expectBody: 'throw-spawned' },
]);

export const ACTOR_PARITY_POLL_STEPS = Object.freeze([
  { name: 'external-count', entrypoint: 'externalCount', expected: 1 },
  { name: 'external-history', entrypoint: 'externalHistory', expected: 'x' },
  { name: 'self-kick-count', entrypoint: 'selfKickCount', expected: 1 },
  { name: 'self-kick-history', entrypoint: 'selfKickHistory', expected: 's' },
  { name: 'fanout-count', entrypoint: 'fanoutCount', expected: 3 },
  { name: 'fanout-history', entrypoint: 'fanoutHistory', expected: 'abc' },
  { name: 'chain-steps', entrypoint: 'chainSteps', expected: 160, timeoutMs: 90_000 },
  { name: 'chain-history', entrypoint: 'chainHistory', expected: 'c'.repeat(160), timeoutMs: 90_000 },
]);

// Frame keys whose values are ephemeral correlation/identity tokens. They are
// replaced per key with stable `<key-N>` placeholders in first-seen order, so
// cross-side comparison checks shape/order/correlation rather than id format.
export const ACTOR_PARITY_TOKEN_KEYS = Object.freeze([
  'rpcId',
  'requestId',
  'invocationId',
  'spawnId',
  'claimId',
  'ownerLeaseId',
  'evictionRequestId',
  'traceId',
  'spanId',
  'parentSpanId',
  'errorId',
  'activationId',
]);

export const ACTOR_PARITY_TIMESTAMP_KEYS = Object.freeze([
  'expiresAt',
  'observedAt',
]);

// Frames excluded from the compared semantic sequence (handshake/health are
// covered by the session-handshake scenario and periodic health frames have
// non-semantic interleaving). They stay in the raw recordOnly evidence.
export const ACTOR_PARITY_EXCLUDED_FRAME_TYPES = new Set([
  'router.bootstrap',
  'runtime.capabilities',
  'assembly.activation',
  'runtime.registered',
  'runtime.health',
]);

export const ACTOR_PARITY_HANDSHAKE_SEQUENCE = Object.freeze([
  'router.bootstrap',
  'runtime.capabilities',
  'assembly.activation',
  'runtime.registered',
  'runtime.health',
]);
