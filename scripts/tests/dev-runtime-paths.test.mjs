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

test('resolveRouterProcessSpec derives the canonical TS spec', () => {
  const spec = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot,
  });
  assert.deepEqual(spec, {
    implementation: 'ts',
    config_path: join(devHome, 'router.yml'),
    ts_source_root: join(repoRoot, 'router'),
  });
  assertRouterProcessSpec(spec);
});

test('resolveRouterProcessSpec derives the canonical Rust spec', () => {
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

test('resolveRouterProcessSpec requires an explicit implementation and devHome', () => {
  assert.throws(
    () => resolveRouterProcessSpec({ devHome }),
    /exactly "ts" or "rust"/,
  );
  assert.throws(
    () => resolveRouterProcessSpec({ implementation: 'ts' }),
    /explicit devHome/,
  );
  assert.throws(
    () => resolveRouterProcessSpec({ devHome, implementation: 'node' }),
    /exactly "ts" or "rust"/,
  );
});

test('assertRouterImplementation accepts only the migration values', () => {
  assert.equal(assertRouterImplementation('ts'), 'ts');
  assert.equal(assertRouterImplementation('rust'), 'rust');
  assert.throws(
    () => assertRouterImplementation('python'),
    /exactly "ts" or "rust"/,
  );
});

test('routerProcessInvocation derives TS and Rust commands from the spec', () => {
  const ts = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot,
  });
  assert.deepEqual(routerProcessInvocation(ts), {
    command: 'pnpm',
    args: [
      '--dir',
      join(repoRoot, 'router'),
      'dev',
      '--config',
      join(devHome, 'router.yml'),
    ],
  });

  const rust = resolveRouterProcessSpec({
    devHome,
    implementation: 'rust',
    repoRoot,
  });
  assert.deepEqual(routerProcessInvocation(rust), {
    command: join(devHome, 'bin', routerBinaryName()),
    args: [join(devHome, 'router.yml')],
  });
});

test('assertRouterProcessSpec enforces exact fields and absolute paths', () => {
  const ts = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot,
  });
  assert.throws(
    () => assertRouterProcessSpec({
      ...ts,
      rust_binary_path: join(devHome, 'bin', routerBinaryName()),
    }),
    /must contain exactly/,
  );
  assert.throws(
    () => assertRouterProcessSpec({ ...ts, ts_source_root: 'router' }),
    /absolute path/,
  );
  assert.throws(
    () => assertRouterProcessSpec({ ...ts, config_path: 'router.yml' }),
    /absolute path/,
  );
  assert.throws(
    () => assertRouterProcessSpec({
      implementation: 'rust',
      config_path: join(devHome, 'router.yml'),
      rust_binary_path: join(devHome, 'bin', routerBinaryName()),
      ts_source_root: join(repoRoot, 'router'),
    }),
    /must contain exactly/,
  );
});
