import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  assertCleanHostBundle,
  assertNoPnpmOrTsxOnPath,
  buildCleanHostBundle,
  cleanHostEnv,
} from '../lib/clean-host-bundle.mjs';
import { resolveRouterProcessSpec } from '../lib/dev-runtime-paths.mjs';
import {
  assertRouterRollbackSwitchPlan,
  assertTsRollbackUnitManifest,
  buildRouterRollbackSwitchPlan,
  buildTsRollbackUnitManifest,
  resolveRouterRollbackUnitProcess,
  routerRollbackUnitProcessRelative,
} from '../lib/rollback-manifest.mjs';

const sha = (hex) => hex.repeat(64);

async function withTemp(fn) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-rollback-final-test-'));
  try {
    return await fn(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test('rollback unit process is unit-relative and resolves against the unit root', () => {
  const configPath = join('/tmp/dev-home', 'router.yml');
  const processSpec = routerRollbackUnitProcessRelative({ configPath });
  assert.deepEqual(processSpec, {
    command: 'node-runtime/bin/node',
    args: [
      'router/node_modules/tsx/dist/cli.mjs',
      'router/src/router/server.ts',
      '--config',
      configPath,
    ],
  });
  const resolved = resolveRouterRollbackUnitProcess(processSpec, '/tmp/unit');
  assert.equal(resolved.command, '/tmp/unit/node-runtime/bin/node');
  assert.equal(resolved.args[0], '/tmp/unit/router/node_modules/tsx/dist/cli.mjs');
  assert.equal(resolved.args[1], '/tmp/unit/router/src/router/server.ts');
  assert.equal(resolved.args[2], '--config');
  assert.equal(resolved.args[3], configPath);
  assert.throws(
    () => routerRollbackUnitProcessRelative({ configPath: 'router.yml' }),
    /absolute config_path/,
  );
});

test('rollback switch plan records both directions with the canonical commands', () => {
  const devHome = '/tmp/rollback-final-dev-home';
  const repoRoot = '/tmp/rollback-final-checkout';
  const tsSpec = resolveRouterProcessSpec({ devHome, implementation: 'ts', repoRoot });
  const rustSpec = resolveRouterProcessSpec({ devHome, implementation: 'rust', repoRoot });
  const unitProcess = routerRollbackUnitProcessRelative({
    configPath: join(devHome, 'router.yml'),
  });
  const plan = buildRouterRollbackSwitchPlan({
    tsSpec,
    rustSpec,
    tsUnitProcess: unitProcess,
  });
  assertRouterRollbackSwitchPlan(plan);
  assert.equal(plan.phases.join(','), 'ts,rust,ts');
  assert.equal(plan.transitions['ts->rust'].from, 'ts');
  assert.equal(plan.transitions['ts->rust'].to, 'rust');
  assert.equal(plan.transitions['ts->rust'].stop.signal, 'SIGTERM');
  assert.equal(plan.transitions['ts->rust'].start.command, rustSpec.rust_binary_path);
  assert.deepEqual(plan.transitions['rust->ts'].start, unitProcess);
  assert.throws(
    () => assertRouterRollbackSwitchPlan({ ...plan, schemaVersion: 'v2' }),
    /schema must be/,
  );
  assert.throws(
    () => assertRouterRollbackSwitchPlan({
      ...plan,
      transitions: { ...plan.transitions, 'ts->rust': { ...plan.transitions['ts->rust'], to: 'ts' } },
    }),
    /must be ts->rust/,
  );
});

test('rollback unit manifest builder/validator round-trip and drift rejection', () => {
  const configPath = '/tmp/rollback-final-dev-home/router.yml';
  const devHome = '/tmp/rollback-final-dev-home';
  const repoRoot = '/tmp/rollback-final-checkout';
  const tsSpec = resolveRouterProcessSpec({ devHome, implementation: 'ts', repoRoot });
  const rustSpec = resolveRouterProcessSpec({ devHome, implementation: 'rust', repoRoot });
  const unitProcess = routerRollbackUnitProcessRelative({ configPath });
  const files = {
    'node-runtime/bin/node': sha('a'),
    'node-runtime/LICENSE': sha('b'),
    'router/package.json': sha('c'),
    'router/pnpm-lock.yaml': sha('d'),
    'router/pnpm-workspace.yaml': sha('e'),
    'router/tsconfig.json': sha('f'),
    'router/src/router/server.ts': sha('0'),
    'router/node_modules/tsx/dist/cli.mjs': sha('1'),
    'router/node_modules/ws/index.js': sha('2'),
    'router/node_modules/tsx': sha('7'),
  };
  const symlinks = {
    'router/node_modules/tsx': '.pnpm/tsx@4.22.3/node_modules/tsx',
  };
  const manifest = buildTsRollbackUnitManifest({
    sourceCommit: 'edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a',
    configPath,
    pinnedNode: {
      version: 'v22.17.0',
      platform: 'darwin',
      arch: 'arm64',
      bin_path: 'node-runtime/bin/node',
      sha256: sha('a'),
    },
    routerSource: {
      root: 'router',
      file_count: 4,
      sha256_tree: sha('3'),
    },
    dependencies: {
      mode: 'materialized',
      root: 'router/node_modules',
      install_command: ['pnpm', '--dir', 'router', 'install', '--frozen-lockfile'],
      install_offline: true,
      file_count: 3,
      sha256_tree: sha('4'),
      symlink_count: 1,
    },
    lockfiles: {
      'router/package.json': sha('c'),
      'router/pnpm-lock.yaml': sha('d'),
      'router/pnpm-workspace.yaml': sha('e'),
    },
    files,
    symlinks,
    fileCount: Object.keys(files).length,
    symlinkCount: Object.keys(symlinks).length,
    sha256Tree: sha('5'),
    process: unitProcess,
    switchCommands: buildRouterRollbackSwitchPlan({
      tsSpec,
      rustSpec,
      tsUnitProcess: unitProcess,
    }),
  });
  assertTsRollbackUnitManifest(manifest);
  assert.throws(
    () => assertTsRollbackUnitManifest({ ...manifest, schemaVersion: 'v2' }),
    /schema must be/,
  );
  assert.throws(
    () => assertTsRollbackUnitManifest({
      ...manifest,
      files: { ...files, 'router/extra.ts': sha('6') },
    }),
    /file_count must equal files map size/,
  );
  assert.throws(
    () => assertTsRollbackUnitManifest({
      ...manifest,
      process: { ...unitProcess, args: [...unitProcess.args.slice(0, 3), '/other.yml'] },
    }),
    /process\.args must match/,
  );
  assert.throws(
    () => assertTsRollbackUnitManifest({ ...manifest, config_path: 'router.yml' }),
    /config_path must be an absolute path/,
  );
});

test('clean-host bundle builds, verifies and detects tampering', async () => {
  await withTemp(async (root) => {
    const routerBinary = join(root, 'router-bin');
    const runtimeBinary = join(root, 'runtime-bin');
    const artifactRoot = join(root, 'artifacts');
    await writeFile(routerBinary, 'router-binary-payload');
    await writeFile(runtimeBinary, 'runtime-binary-payload');
    await chmod(routerBinary, 0o755);
    await chmod(runtimeBinary, 0o755);
    await mkdir(join(artifactRoot, 'records'), { recursive: true });
    await writeFile(join(artifactRoot, 'records', 'hello.txt'), 'hello');

    const bundleRoot = join(root, 'bundle');
    const bundle = await buildCleanHostBundle({
      bundleRoot,
      routerBinary,
      runtimeBinary,
      routerConfigText: 'http:\n  port: 1\n',
      runtimeConfigText: 'router: ws://127.0.0.1:1/runtime\n',
      artifactRoot,
    });
    assert.equal(bundle.manifest.schemaVersion, 'skiff-router-clean-host-bundle-v1');
    assert.equal(bundle.manifest.process_commands.router.exec.command, 'bin/skiff-router');
    await assertCleanHostBundle(bundleRoot);

    await writeFile(join(bundleRoot, 'config', 'router.yml'), 'tampered');
    await assert.rejects(
      () => assertCleanHostBundle(bundleRoot),
      /identity drift/,
    );
  });
});

test('clean-host env strips pnpm/tsx from PATH and the probe detects pollution', async () => {
  await withTemp(async (root) => {
    const home = join(root, 'empty-home');
    const env = cleanHostEnv(process.env, { home });
    assert.equal(env.PATH, '/usr/bin:/bin:/usr/sbin:/sbin');
    assert.equal(env.HOME, home);
    assert.equal(env.npm_config_offline, 'true');
    assert.equal(env.PNPM_HOME, undefined);
    await assertNoPnpmOrTsxOnPath({ env });

    const fakeBin = join(root, 'fake-bin');
    await mkdir(fakeBin);
    await writeFile(join(fakeBin, 'pnpm'), '#!/bin/sh\nexit 0\n');
    await chmod(join(fakeBin, 'pnpm'), 0o755);
    await assert.rejects(
      () => assertNoPnpmOrTsxOnPath({ env: { ...env, PATH: fakeBin } }),
      /must not expose pnpm\/tsx/,
    );
  });
});
