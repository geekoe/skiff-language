import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  buildComponent,
  componentSpec,
  copyBinary,
  expandComponents,
  installBinary,
  parseBuildArgs,
  parseTargetDirectory,
  resolveTargetDirectory,
  runBuild,
  sha256Hex,
} from '../skiff-build.mjs';

const skiffRoot = join(import.meta.dirname, '..', '..');

test('componentSpec maps the three components onto crates, bins, and manifests', () => {
  assert.deepEqual(componentSpec('router'), {
    crate: 'skiff-router',
    bin: 'skiff-router',
    manifest: 'router/Cargo.toml',
  });
  assert.deepEqual(componentSpec('runtime'), {
    crate: 'runtime',
    bin: 'runtime',
    manifest: 'runtime/Cargo.toml',
  });
  assert.deepEqual(componentSpec('compiler'), {
    crate: 'skiff-compiler',
    bin: 'skiff-compiler',
    manifest: 'compiler/Cargo.toml',
  });
  assert.throws(() => componentSpec('bogus'), /unknown component bogus/);
});

test('expandComponents expands all, validates names, and deduplicates', () => {
  assert.deepEqual(
    expandComponents(['all']).map((spec) => spec.bin),
    ['skiff-router', 'runtime', 'skiff-compiler'],
  );
  assert.deepEqual(
    expandComponents(['router', 'runtime']).map((spec) => spec.bin),
    ['skiff-router', 'runtime'],
  );
  assert.deepEqual(
    expandComponents(['all', 'router']).map((spec) => spec.bin),
    ['skiff-router', 'runtime', 'skiff-compiler'],
  );
  assert.throws(() => expandComponents(['bogus']), /unknown component bogus/);
});

test('parseBuildArgs collects positional components and defaults profile to debug', () => {
  assert.deepEqual(parseBuildArgs(['router', 'runtime']), {
    components: ['router', 'runtime'],
    profile: 'debug',
    help: false,
  });
  assert.deepEqual(parseBuildArgs(['all', '--profile', 'release']), {
    components: ['all'],
    profile: 'release',
    help: false,
  });
  assert.deepEqual(parseBuildArgs(['--profile', 'release', 'compiler']), {
    components: ['compiler'],
    profile: 'release',
    help: false,
  });
});

test('parseBuildArgs supports --help and rejects unknown or incomplete arguments', () => {
  assert.equal(parseBuildArgs(['--help']).help, true);
  assert.equal(parseBuildArgs(['-h']).help, true);
  assert.throws(() => parseBuildArgs(['--bogus']), /unknown argument --bogus/);
  assert.throws(() => parseBuildArgs(['router', '--profile']), /--profile requires a value/);
  assert.throws(() => parseBuildArgs(['router', '--profile', '--help']), /--profile requires a value/);
});

test('parseTargetDirectory extracts target_directory and rejects malformed output', () => {
  assert.equal(
    parseTargetDirectory(JSON.stringify({ target_directory: '/cache/skiff-target' })),
    '/cache/skiff-target',
  );
  assert.throws(() => parseTargetDirectory('not json'), /invalid cargo metadata output/);
  assert.throws(
    () => parseTargetDirectory(JSON.stringify({ packages: [] })),
    /missing target_directory/,
  );
});

test('resolveTargetDirectory honors CARGO_TARGET_DIR without running cargo', async () => {
  let invoked = false;
  const targetDir = await resolveTargetDirectory({
    skiffRoot,
    manifest: 'router/Cargo.toml',
    env: { CARGO_TARGET_DIR: join(tmpdir(), 'skiff-shared-target') },
    runCommand: async () => {
      invoked = true;
    },
  });
  assert.equal(targetDir, join(tmpdir(), 'skiff-shared-target'));
  assert.equal(invoked, false);
});

test('resolveTargetDirectory falls back to parsing cargo metadata output', async () => {
  const targetDir = await resolveTargetDirectory({
    skiffRoot,
    manifest: 'router/Cargo.toml',
    env: {},
    runCommand: async (command, args, options) => {
      assert.equal(command, 'cargo');
      assert.deepEqual(args, [
        'metadata',
        '--format-version',
        '1',
        '--manifest-path',
        join(skiffRoot, 'router', 'Cargo.toml'),
      ]);
      assert.equal(options.cwd, skiffRoot);
      return { stdout: JSON.stringify({ target_directory: '/cache/skiff-target' }) };
    },
  });
  assert.equal(targetDir, '/cache/skiff-target');
});

test('sha256Hex matches a direct hash of the file bytes', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-build-hash-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const file = join(root, 'blob');
  const payload = 'fake binary payload\n';
  await writeFile(file, payload);
  assert.equal(
    await sha256Hex(file),
    createHash('sha256').update(payload).digest('hex'),
  );
});

test('copyBinary copies the source and is idempotent on overwrite', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-build-copy-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'src');
  const destination = join(root, 'out');
  await writeFile(source, 'first');
  await copyBinary(source, destination);
  assert.equal(await readFile(destination, 'utf8'), 'first');
  await writeFile(source, 'second');
  await copyBinary(source, destination);
  assert.equal(await readFile(destination, 'utf8'), 'second');
});

test('installBinary copies the binary and writes a git-style sha256 file', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-build-install-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const source = join(root, 'fake-bin');
  const payload = '#!/bin/sh\nexit 0\n';
  await writeFile(source, payload);
  const destination = join(root, 'build', 'bin', 'skiff-router');
  const hashFile = `${destination}.sha256`;

  const installed = await installBinary({ source, destination, hashFile });

  assert.equal(installed.destination, destination);
  assert.equal(await readFile(destination, 'utf8'), payload);
  assert.equal(await readFile(hashFile, 'utf8'), `${installed.sha256} skiff-router`);
});

test('buildComponent runs cargo build then installs the debug binary', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-build-component-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const targetDir = join(root, 'target');
  await mkdir(join(targetDir, 'debug'), { recursive: true });
  const payload = 'fake router binary';
  await writeFile(join(targetDir, 'debug', 'skiff-router'), payload);

  const calls = [];
  const result = await buildComponent({
    spec: componentSpec('router'),
    skiffRoot: root,
    profile: 'debug',
    targetDir,
    runCommand: async (command, args, options) => {
      calls.push({ command, args, options });
    },
  });

  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].args, [
    'build',
    '--manifest-path',
    'router/Cargo.toml',
    '--bin',
    'skiff-router',
  ]);
  assert.equal(calls[0].options.cwd, root);
  assert.equal(result.destination, join(root, 'build', 'bin', 'skiff-router'));
  assert.equal(await readFile(result.destination, 'utf8'), payload);
  assert.equal(
    await readFile(`${result.destination}.sha256`, 'utf8'),
    `${result.sha256} skiff-router`,
  );
});

test('buildComponent adds --release and reads from the release directory', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-build-component-release-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const targetDir = join(root, 'target');
  await mkdir(join(targetDir, 'release'), { recursive: true });
  await writeFile(join(targetDir, 'release', 'runtime'), 'fake runtime binary');

  const calls = [];
  await buildComponent({
    spec: componentSpec('runtime'),
    skiffRoot: root,
    profile: 'release',
    targetDir,
    runCommand: async (command, args) => {
      calls.push({ command, args });
    },
  });

  assert.deepEqual(calls[0].args, [
    'build',
    '--manifest-path',
    'runtime/Cargo.toml',
    '--bin',
    'runtime',
    '--release',
  ]);
});

test('runBuild builds every expanded component into build/bin', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-build-run-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const targetDir = join(root, 'target');
  await mkdir(join(targetDir, 'debug'), { recursive: true });
  for (const bin of ['skiff-router', 'runtime', 'skiff-compiler']) {
    await writeFile(join(targetDir, 'debug', bin), `fake ${bin}`);
  }

  const results = await runBuild({
    skiffRoot: root,
    components: ['all'],
    profile: 'debug',
    env: { CARGO_TARGET_DIR: targetDir },
    runCommand: async () => {},
  });

  assert.deepEqual(
    results.map((result) => result.spec.bin),
    ['skiff-router', 'runtime', 'skiff-compiler'],
  );
  for (const result of results) {
    assert.equal(
      await readFile(result.destination, 'utf8'),
      `fake ${result.spec.bin}`,
    );
    assert.equal(
      await readFile(`${result.destination}.sha256`, 'utf8'),
      `${result.sha256} ${result.spec.bin}`,
    );
  }
});

test('runBuild rejects an invalid profile before running anything', async () => {
  let invoked = false;
  await assert.rejects(
    runBuild({
      skiffRoot,
      components: ['router'],
      profile: 'fancy',
      env: { CARGO_TARGET_DIR: join(tmpdir(), 'target') },
      runCommand: async () => {
        invoked = true;
      },
    }),
    /build profile must be "debug" or "release"; got fancy/,
  );
  assert.equal(invoked, false);
});

test('runBuild rejects unknown components and empty components without running cargo', async () => {
  let invoked = false;
  const options = {
    skiffRoot,
    profile: 'debug',
    env: { CARGO_TARGET_DIR: join(tmpdir(), 'target') },
    runCommand: async () => {
      invoked = true;
    },
  };
  await assert.rejects(
    runBuild({ ...options, components: ['bogus'] }),
    /unknown component bogus/,
  );
  await assert.rejects(
    runBuild({ ...options, components: [] }),
    /no components given/,
  );
  assert.equal(invoked, false);
});
