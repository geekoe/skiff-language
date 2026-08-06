import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import test from 'node:test';

import {
  isolatedTestInstanceYml,
  isolatedTestRunnerEnvironment,
} from '../lib/isolated-test-runtime-instance.mjs';

const root = resolve(import.meta.dirname, '..', '..');

test('non-live runner receives canonical artifact and Host-ingress targets', () => {
  const environment = isolatedTestRunnerEnvironment({
    baseEnv: {
      PATH: '/bin',
      CARGO_TARGET_DIR: 'hostile-relative-target',
      SKIFF_TEST_PLATFORM_SOURCE_ROOT: '/tmp/hostile-platform-root',
    },
    skiffRoot: '/checkout/skiff',
    cargoTarget: '/checkout/cargo-target',
    devHome: '/tmp/skiff-owned/dev-home',
    controlPort: 46101,
    routerHttpPort: 46100,
    profile: 'test-environment',
  });

  assert.equal(environment.SKIFF_DEV_HOME, '/tmp/skiff-owned/dev-home');
  assert.equal(
    environment.SKIFF_TEST_RUNTIME_ARTIFACT_ROOT,
    '/tmp/skiff-owned/dev-home/artifacts',
  );
  assert.equal(environment.SKIFF_TEST_INGRESS_URL, 'http://127.0.0.1:46100');
  assert.equal(environment.SKIFF_TEST_ENVIRONMENT, 'test-environment');
  assert.equal(environment.SKIFF_TEST_ACTIVATION_URL, undefined);
  assert.equal(environment.SKIFF_TEST_EXPECTED_GENERATION, undefined);
  assert.equal(environment.CARGO_TARGET_DIR, '/checkout/cargo-target');
  assert.equal(environment.SKIFF_TEST_PLATFORM_SOURCE_ROOT, '/checkout/skiff');
  assert.equal(environment.SKIFF_DEV_RELOAD_URL, undefined);
});

test('isolated instance spec is rooted in its temporary dev home and dynamic ports', () => {
  const config = isolatedTestInstanceYml({
    devHome: '/tmp/skiff-owned/dev-home',
    basePort: 46100,
    mongoPort: 46103,
    routerBinary: '/checkout/skiff/build/runtime-stack/bin/skiff-router',
    runtimeBinary: '/checkout/skiff/build/runtime-stack/bin/skiff-runtime',
  });

  assert.match(config, /devHome: "\/tmp\/skiff-owned\/dev-home"/);
  assert.match(config, /^schemaVersion: skiff-instance-v1$/m);
  assert.match(config, /^  - name: mongo$/m);
  assert.match(config, /^  - name: router$/m);
  assert.match(config, /^  - name: runtime$/m);
  assert.match(config, /46100/);
  assert.match(config, /46103/);
  assert.doesNotMatch(config, /27017/);
  assert.match(config, /profile: "skiff-test"/);
  assert.doesNotMatch(config, /\.skiff-instance/);
  assert.doesNotMatch(config, /__skiff\/reload-artifacts/);
});

test('Cargo owns the current ungated integration targets and no recursive wrapper', async () => {
  const manifest = await readFile(join(root, 'test-runner', 'Cargo.toml'), 'utf8');
  const targets = manifest.split('[[test]]').slice(1).map(parseTestTarget);

  assert.deepEqual(targets, [
    { name: 'test_service_flow' },
    { name: 'canonical_std_seed_bootstrap' },
    { name: 'http_entry_test_service' },
  ]);
  assert.doesNotMatch(manifest, /runtime-integration-worker/);
  assert.doesNotMatch(manifest, /test_runner_runtime_isolation/);
});

function parseTestTarget(block) {
  const name = block.match(/^name = "([^"]+)"$/m)?.[1];
  assert.ok(name, `test target is missing a name:${block}`);
  return { name };
}
