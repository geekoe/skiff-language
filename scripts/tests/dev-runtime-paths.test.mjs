import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  assertRouterImplementation,
  assertRouterProcessSpec,
  devRuntimePaths,
  resolveRouterProcessSpec,
  routerBinaryName,
  routerProcessInvocation,
} from '../lib/dev-runtime-paths.mjs';

const devHome = join(tmpdir(), 'skiff-router-spec-test-dev-home');
const repoRoot = join(tmpdir(), 'skiff-router-spec-test-checkout');

test('devRuntimePaths exposes the router binary dev path', () => {
  const paths = devRuntimePaths({ devHome, env: {} });
  assert.equal(paths.routerBinary, join(devHome, 'bin', routerBinaryName()));
  assert.equal(paths.routerConfig, join(devHome, 'router.yml'));
});

test('resolveRouterProcessSpec derives the canonical Rust spec by default', () => {
  const spec = resolveRouterProcessSpec({
    devHome,
    repoRoot,
  });
  assert.deepEqual(spec, {
    implementation: 'rust',
    config_path: join(devHome, 'router.yml'),
    rust_binary_path: join(devHome, 'bin', routerBinaryName()),
  });
  assertRouterProcessSpec(spec);
});

test('resolveRouterProcessSpec accepts an explicit rust implementation', () => {
  const spec = resolveRouterProcessSpec({
    devHome,
    implementation: 'rust',
    repoRoot,
  });
  assert.deepEqual(spec, {
    implementation: 'rust',
    config_path: join(devHome, 'router.yml'),
    rust_binary_path: join(devHome, 'bin', routerBinaryName()),
  });
  assertRouterProcessSpec(spec);
});

test('resolveRouterProcessSpec rejects the retired TS implementation and missing devHome', () => {
  assert.throws(
    () => resolveRouterProcessSpec({ devHome, implementation: 'ts' }),
    /no longer selectable/,
  );
  assert.throws(
    () => resolveRouterProcessSpec({}),
    /explicit devHome/,
  );
  assert.throws(
    () => resolveRouterProcessSpec({ devHome, implementation: 'node' }),
    /no longer selectable/,
  );
});

test('assertRouterImplementation accepts only rust', () => {
  assert.equal(assertRouterImplementation('rust'), 'rust');
  assert.throws(
    () => assertRouterImplementation('ts'),
    /no longer selectable/,
  );
});

test('routerProcessInvocation derives the Rust command from the spec', () => {
  const rust = resolveRouterProcessSpec({ devHome, repoRoot });
  assert.deepEqual(routerProcessInvocation(rust), {
    command: join(devHome, 'bin', routerBinaryName()),
    args: [join(devHome, 'router.yml')],
  });
});

test('assertRouterProcessSpec enforces exact fields and absolute paths', () => {
  const rust = resolveRouterProcessSpec({ devHome, repoRoot });
  assert.throws(
    () => assertRouterProcessSpec({
      ...rust,
      ts_source_root: join(repoRoot, 'router'),
    }),
    /must contain exactly/,
  );
  assert.throws(
    () => assertRouterProcessSpec({ ...rust, rust_binary_path: 'router' }),
    /absolute path/,
  );
  assert.throws(
    () => assertRouterProcessSpec({ ...rust, config_path: 'router.yml' }),
    /absolute path/,
  );
  assert.throws(
    () => assertRouterProcessSpec({
      implementation: 'ts',
      config_path: join(devHome, 'router.yml'),
      rust_binary_path: join(devHome, 'bin', routerBinaryName()),
    }),
    /no longer selectable/,
  );
});
