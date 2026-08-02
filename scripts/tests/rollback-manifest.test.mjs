import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  resolveRouterProcessSpec,
  routerProcessInvocation,
} from '../lib/dev-runtime-paths.mjs';
import {
  ROUTER_ROLLBACK_MANIFEST_SCHEMA,
  assertRouterRollbackManifest,
  buildRouterRollbackManifest,
} from '../lib/rollback-manifest.mjs';

const devHome = join(tmpdir(), 'skiff-router-rollback-test-dev-home');
const repoRoot = join(tmpdir(), 'skiff-router-rollback-test-checkout');

test('rollback builder emits a TS unit manifest with the TS process command', () => {
  const spec = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot,
  });
  const manifest = buildRouterRollbackManifest(spec);
  assert.deepEqual(manifest, {
    schemaVersion: ROUTER_ROLLBACK_MANIFEST_SCHEMA,
    implementation: 'ts',
    config_path: join(devHome, 'router.yml'),
    ts_source_root: join(repoRoot, 'router'),
    process: routerProcessInvocation(spec),
  });
  assertRouterRollbackManifest(manifest);
});

test('rollback builder emits a Rust unit manifest with the binary process command', () => {
  const spec = resolveRouterProcessSpec({
    devHome,
    implementation: 'rust',
    repoRoot,
  });
  const manifest = buildRouterRollbackManifest(spec);
  assert.equal(manifest.implementation, 'rust');
  assert.equal(manifest.rust_binary_path, join(devHome, 'bin', 'skiff-router'));
  assert.equal(manifest.ts_source_root, undefined);
  assert.deepEqual(manifest.process, routerProcessInvocation(spec));
  assertRouterRollbackManifest(manifest);
});

test('rollback builder rejects input that is not a RouterProcessSpec', () => {
  assert.throws(
    () => buildRouterRollbackManifest({
      implementation: 'ts',
      config_path: 'router.yml',
    }),
    /absolute path/,
  );
  assert.throws(
    () => buildRouterRollbackManifest({
      implementation: 'go',
      config_path: join(devHome, 'router.yml'),
    }),
    /exactly "ts" or "rust"/,
  );
});

test('rollback validator rejects schema, field, and process command drift', () => {
  const spec = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot,
  });
  const manifest = buildRouterRollbackManifest(spec);

  assert.throws(
    () => assertRouterRollbackManifest({
      ...manifest,
      schemaVersion: 'skiff-router-rollback-unit-v2',
    }),
    /schema must be/,
  );
  assert.throws(
    () => assertRouterRollbackManifest({
      ...manifest,
      config_path: join(devHome, 'other.yml'),
    }),
    /invocation/,
  );
  assert.throws(
    () => assertRouterRollbackManifest({ ...manifest, extra: true }),
    /must contain exactly/,
  );
  const { process: _process, ...withoutProcess } = manifest;
  assert.throws(
    () => assertRouterRollbackManifest(withoutProcess),
    /must contain exactly/,
  );
});
