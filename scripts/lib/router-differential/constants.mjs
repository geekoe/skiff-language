import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
export const defaultSkiffRoot = resolve(scriptDir, '..', '..', '..');

export const DIFFERENTIAL_SCHEMA_VERSION =
  'skiff-router-differential-inventory-v1';
export const ENVIRONMENT = 'router-differential';
export const GENERATION = 1;
export const REPLICA_ID = 'skiff-runtime-differential-replica';
export const ROUTER_PORT_MIN = 45000;
export const ROUTER_PORT_MAX = 45999;
export const ROUTER_PORTS_PER_SIDE = 3;
export const FORBIDDEN_PORTS = Object.freeze(new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]));

// Each implementation owns its canonical Mongo namespace. The differential
// harness seeds the same semantic EnvironmentActivationState into each
// side's own database/collections; the namespaces never overlap.
export const TS_DATABASE = 'skiff_router_ts_differential';
export const TS_STATE_COLLECTION = 'router_assembly_activation_states';
export const TS_AUDIT_COLLECTION = 'router_assembly_activation_audit';
export const RUST_DATABASE = 'skiff-router';
export const RUST_STATE_COLLECTION = 'activation_state';
export const RUST_AUDIT_COLLECTION = 'activation_audit';

export const ACTIVATION_STATE_SCHEMA_VERSION =
  'skiff-environment-activation-state-v2';
export const ACTOR_ROUTING_PROJECTION_RECORD_PATH =
  'records/actor-routing/current.json';
export const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';

export const SCENARIO_INVENTORY_REPO_PATH =
  'scripts/fixtures/router-differential/scenario-inventory.json';
export const FIXTURE_SERVICE_REPO_PATH =
  'scripts/fixtures/router-differential/ping';

export function scenarioInventoryPath(skiffRoot) {
  return resolve(skiffRoot, SCENARIO_INVENTORY_REPO_PATH);
}

export function fixtureServicePath(skiffRoot) {
  return resolve(skiffRoot, FIXTURE_SERVICE_REPO_PATH);
}

export function routerBinaryPath(targetDir, platform = process.platform) {
  return join(targetDir, 'debug', platform === 'win32'
    ? 'skiff-router.exe'
    : 'skiff-router');
}

export function runtimeBinaryPath(targetDir, platform = process.platform) {
  return join(targetDir, 'debug', platform === 'win32'
    ? 'runtime.exe'
    : 'runtime');
}

export function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}
