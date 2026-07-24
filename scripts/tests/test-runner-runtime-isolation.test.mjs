import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import test from 'node:test';

import {
  isolatedTestInstanceConfigText,
  isolatedTestRunnerEnvironment,
} from '../lib/isolated-test-runtime-instance.mjs';

const root = resolve(import.meta.dirname, '..', '..');

test('non-live runner receives canonical activation and Host-ingress targets', () => {
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
    environment: 'test-environment',
  });

  assert.equal(environment.SKIFF_DEV_HOME, '/tmp/skiff-owned/dev-home');
  assert.equal(
    environment.SKIFF_TEST_RUNTIME_ARTIFACT_ROOT,
    '/tmp/skiff-owned/dev-home/artifacts',
  );
  assert.equal(
    environment.SKIFF_TEST_ACTIVATION_URL,
    'http://127.0.0.1:46101/__skiff/activate-assembly',
  );
  assert.equal(environment.SKIFF_TEST_INGRESS_URL, 'http://127.0.0.1:46100');
  assert.equal(environment.SKIFF_TEST_ENVIRONMENT, 'test-environment');
  assert.equal(environment.SKIFF_TEST_EXPECTED_GENERATION, '0');
  assert.equal(environment.CARGO_TARGET_DIR, '/checkout/cargo-target');
  assert.equal(environment.SKIFF_TEST_PLATFORM_SOURCE_ROOT, '/checkout/skiff');
  assert.equal(environment.SKIFF_DEV_RELOAD_URL, undefined);
});

test('isolated config is rooted in its temporary dev home and dynamic ports', () => {
  const config = isolatedTestInstanceConfigText({
    devHome: '/tmp/skiff-owned/dev-home',
    cargoTarget: '/tmp/skiff-owned/cargo-target',
    basePort: 46100,
    mongoPort: 46103,
  });

  assert.match(config, /devHome: "\/tmp\/skiff-owned\/dev-home"/);
  assert.match(config, /cargoTargetDir: "\/tmp\/skiff-owned\/cargo-target"/);
  assert.match(config, /base: 46100/);
  assert.match(config, /mongo: 46103/);
  assert.match(config, /mongo: managed/);
  assert.doesNotMatch(config, /27017/);
  assert.match(config, /environment: "skiff-test"/);
  assert.doesNotMatch(config, /\.skiff-instance/);
  assert.doesNotMatch(config, /__skiff\/reload-artifacts/);
});

test('Cargo owns one ungated canonical cutover target and no recursive wrapper', async () => {
  const manifest = await readFile(join(root, 'test-runner', 'Cargo.toml'), 'utf8');
  const targets = manifest.split('[[test]]').slice(1).map(parseTestTarget);

  assert.deepEqual(targets, [{ name: 'package_service_contract_deployment' }]);
  assert.doesNotMatch(manifest, /runtime-integration-worker/);
  assert.doesNotMatch(manifest, /test_runner_runtime_isolation/);
});

function parseTestTarget(block) {
  const name = block.match(/^name = "([^"]+)"$/m)?.[1];
  assert.ok(name, `test target is missing a name:${block}`);
  return { name };
}
