import {
  assertRouterProcessSpec,
  routerProcessInvocation,
} from './dev-runtime-paths.mjs';

export const ROUTER_ROLLBACK_MANIFEST_SCHEMA = 'skiff-router-rollback-unit-v1';

export function buildRouterRollbackManifest(spec) {
  assertRouterProcessSpec(spec);
  const process = routerProcessInvocation(spec);
  return Object.freeze({
    schemaVersion: ROUTER_ROLLBACK_MANIFEST_SCHEMA,
    implementation: spec.implementation,
    config_path: spec.config_path,
    ...(spec.implementation === 'ts'
      ? { ts_source_root: spec.ts_source_root }
      : { rust_binary_path: spec.rust_binary_path }),
    process: Object.freeze({
      command: process.command,
      args: Object.freeze([...process.args]),
    }),
  });
}

export function assertRouterRollbackManifest(manifest) {
  if (!manifest || typeof manifest !== 'object' || Array.isArray(manifest)) {
    throw new Error('router rollback manifest must be an object');
  }
  if (manifest.schemaVersion !== ROUTER_ROLLBACK_MANIFEST_SCHEMA) {
    throw new Error(
      `router rollback manifest schema must be ${ROUTER_ROLLBACK_MANIFEST_SCHEMA}`,
    );
  }
  const spec = {
    implementation: manifest.implementation,
    config_path: manifest.config_path,
  };
  if (spec.implementation === 'ts') {
    spec.ts_source_root = manifest.ts_source_root;
  } else {
    spec.rust_binary_path = manifest.rust_binary_path;
  }
  assertRouterProcessSpec(spec);
  const expectedKeys = spec.implementation === 'ts'
    ? ['schemaVersion', 'implementation', 'config_path', 'ts_source_root', 'process']
    : ['schemaVersion', 'implementation', 'config_path', 'rust_binary_path', 'process'];
  const actualKeys = Object.keys(manifest).sort();
  if (actualKeys.join(',') !== [...expectedKeys].sort().join(',')) {
    throw new Error(
      `router rollback manifest must contain exactly ${expectedKeys.join(', ')}`,
    );
  }
  const process = routerProcessInvocation(spec);
  if (
    !manifest.process
    || typeof manifest.process !== 'object'
    || Array.isArray(manifest.process)
    || manifest.process.command !== process.command
    || !Array.isArray(manifest.process.args)
    || manifest.process.args.join('\0') !== process.args.join('\0')
  ) {
    throw new Error(
      'router rollback manifest process command must match the RouterProcessSpec invocation',
    );
  }
  return manifest;
}
