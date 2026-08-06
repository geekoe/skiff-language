import assert from 'node:assert/strict';
import { access, chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, delimiter, join, relative, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const skiffPath = join(root, 'scripts', 'skiff.mjs');
const input = join(root, 'runtime', 'live-tests', 'internal', 'operation.live.test.skiff');

test('skiff test selects the canonical binary once for absolute and relative roots', async () => {
  const manifestPath = join(root, 'test-runner', 'Cargo.toml');
  const manifest = await readFile(manifestPath, 'utf8');
  assert.deepEqual(
    [...manifest.matchAll(/^\[\[bin\]\]\nname = "([^"]+)"/gm)].map((match) => match[1]),
    ['skiff-test-runner', 'skiff-package-service-smoke-fixture'],
  );
  assert.doesNotMatch(manifest, /^default-run\s*=/m);

  const fixture = await fakeCargoFixture();
  try {
    const artifactRoot = join(fixture.root, 'artifacts');
    await mkdir(artifactRoot);
    const assembly = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
    const configSnapshot =
      `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`;
    for (const testRoot of [input, relative(root, input)]) {
      const result = await runProcess(process.execPath, [
        skiffPath,
        'test',
        testRoot,
        '--artifact-root',
        artifactRoot,
        '--base-assembly',
        assembly,
        '--base-config-snapshot',
        configSnapshot,
        '--live',
        '--ingress-url',
        'http://router.test:4100',
        '--profile',
        'test-live',
        '--deny-skips',
        '--require-tests',
      ], {
        env: {
          ...fixture.env,
          SKIFF_TEST_RUNNER_BIN: 'hostile-profile-fallback',
        },
      });
      assert.equal(result.code, 0, result.stderr);
      const args = JSON.parse(await readFile(fixture.marker, 'utf8'));
      assert.equal(args.filter((arg) => arg === '--bin').length, 1);
      assert.deepEqual(args, [
        'run',
        '--locked',
        '--quiet',
        '--manifest-path',
        manifestPath,
        '--bin',
        'skiff-test-runner',
        '--',
        input,
        '--live',
        '--artifact-root',
        artifactRoot,
        '--platform-source-root',
        root,
        '--base-assembly',
        assembly,
        '--base-config-snapshot',
        configSnapshot,
        '--ingress-url',
        'http://router.test:4100',
        '--profile',
        'test-live',
        '--deny-skips',
        '--require-tests',
      ]);
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('skiff test treats package.yml as the root and external files as optional service controls', async () => {
  for (const { withServiceRole, externalFiles } of [
    { withServiceRole: false, externalFiles: [] },
    { withServiceRole: true, externalFiles: [] },
    { withServiceRole: true, externalFiles: ['http.yml'] },
    { withServiceRole: true, externalFiles: ['websocket.yml'] },
    { withServiceRole: true, externalFiles: ['http.yml', 'websocket.yml'] },
  ]) {
    const fixture = await fakeCargoFixture();
    try {
      const packageRoot = join(
        fixture.root,
        withServiceRole ? `service-package-${externalFiles.length}` : 'package',
      );
      const artifactRoot = join(fixture.root, 'artifacts');
      await mkdir(packageRoot);
      await mkdir(artifactRoot);
      await writeFile(
        join(packageRoot, 'package.yml'),
        'id: example.com/root-detection\nversion: 1.0.0\n',
      );
      if (withServiceRole) {
        await writeFile(join(packageRoot, 'service.yml'), 'id: example.com/root-detection\n');
      }
      for (const externalFile of externalFiles) {
        await writeFile(
          join(packageRoot, externalFile),
          externalFile === 'http.yml' ? '{}\n' : 'path: /socket\n',
        );
      }

      const result = await runProcess(process.execPath, [
        skiffPath, 'test', packageRoot, '--artifact-root', artifactRoot,
        '--live',
        '--ingress-url', 'http://router.test:4100',
        '--profile', 'root-detection',
      ], { env: fixture.env });

      assert.equal(result.code, 0, result.stderr);
      const args = JSON.parse(await readFile(fixture.marker, 'utf8'));
      assert.equal(args[args.indexOf('--') + 1], packageRoot);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

test('skiff test rejects external-only and ordinary package external files before Cargo', async () => {
  for (const { withPackage, externalFiles } of [
    { withPackage: false, externalFiles: ['http.yml'] },
    { withPackage: false, externalFiles: ['websocket.yml'] },
    { withPackage: true, externalFiles: ['http.yml'] },
    { withPackage: true, externalFiles: ['websocket.yml'] },
    { withPackage: true, externalFiles: ['http.yml', 'websocket.yml'] },
  ]) {
    const fixture = await fakeCargoFixture();
    try {
      const sourceRoot = join(fixture.root, withPackage ? 'ordinary-package' : 'external-only');
      const artifactRoot = join(fixture.root, 'artifacts');
      await mkdir(sourceRoot);
      await mkdir(artifactRoot);
      if (withPackage) {
        await writeFile(
          join(sourceRoot, 'package.yml'),
          'id: example.com/root-detection\nversion: 1.0.0\n',
        );
      }
      for (const externalFile of externalFiles) {
        await writeFile(
          join(sourceRoot, externalFile),
          externalFile === 'http.yml' ? '{}\n' : 'path: /socket\n',
        );
      }

      const result = await runProcess(process.execPath, [
        skiffPath, 'test', sourceRoot, '--artifact-root', artifactRoot,
      ], { env: fixture.env });

      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /external service control file.*require service\.yml/);
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

test('skiff test rejects service-only and manifest-less directories before Cargo', async () => {
  for (const manifest of ['service.yml', undefined]) {
    const fixture = await fakeCargoFixture();
    try {
      const sourceRoot = join(fixture.root, manifest ? 'legacy-service' : 'missing-manifest');
      const artifactRoot = join(fixture.root, 'artifacts');
      await mkdir(sourceRoot);
      await mkdir(artifactRoot);
      if (manifest) {
        await writeFile(join(sourceRoot, manifest), 'id: example.com/legacy-service\n');
      }

      const result = await runProcess(process.execPath, [
        skiffPath, 'test', sourceRoot, '--artifact-root', artifactRoot,
      ], { env: fixture.env });

      assert.notEqual(result.code, 0);
      assert.match(
        result.stderr,
        manifest
          ? /service\.yml adds a service role to a Package and cannot define a source root/
          : /must contain package\.yml/,
      );
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

test('skiff test rejects the retired external test config literal input', async () => {
  const fixture = await fakeCargoFixture();
  try {
    const result = await runProcess(process.execPath, [
      skiffPath,
      'test',
      input,
      '--artifact-root',
      fixture.root,
      '--test-config-literals',
      join(fixture.root, 'retired.json'),
    ], { env: fixture.env });

    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /unknown option --test-config-literals/);
    await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('non-live skiff test rejects caller-owned live targets before Cargo', async () => {
  const fixture = await fakeCargoFixture();
  try {
    for (const option of [
      ['--ingress-url', 'http://router.test:4100'],
      ['--profile', 'caller-owned'],
    ]) {
      const result = await runProcess(process.execPath, [
        skiffPath, 'test', input, '--artifact-root', fixture.root, ...option,
      ], { env: fixture.env });
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /non-live skiff test owns ingress and profile targets/);
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
    for (const option of [
      ['--activation-url', 'http://router.test:4101/__skiff/activate-assembly'],
      ['--expected-generation', '1'],
    ]) {
      const result = await runProcess(process.execPath, [
        skiffPath, 'test', input, '--artifact-root', fixture.root, ...option,
      ], { env: fixture.env });
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /unknown option --activation-url|unknown option --expected-generation/);
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('skiff test rejects every retired test-runner option without an alias', async () => {
  const fixture = await fakeCargoFixture();
  try {
    for (const option of [
      '--service-artifact-root',
      '--config',
      '--package-test-concurrency',
      '--router-reload-url',
      '--packages-dir',
      '--allow-network',
      '--platform-source-root',
    ]) {
      const result = await runProcess(process.execPath, [
        skiffPath, 'test', input, '--artifact-root', fixture.root, option,
      ], { env: fixture.env });
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, new RegExp(`unknown option ${option}`));
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('live skiff test requires an existing artifact root and every live target field', async () => {
  const fixture = await fakeCargoFixture();
  try {
    const missing = await runProcess(process.execPath, [
      skiffPath, 'test', input, '--artifact-root', join(fixture.root, 'missing'), '--live',
    ], { env: fixture.env });
    assert.notEqual(missing.code, 0);
    assert.match(missing.stderr, /skiff test --artifact-root must be an existing directory/);

    const incomplete = await runProcess(process.execPath, [
      skiffPath, 'test', input, '--artifact-root', fixture.root, '--live',
    ], { env: fixture.env });
    assert.notEqual(incomplete.code, 0);
    assert.match(incomplete.stderr, /live skiff test requires --ingress-url/);
    await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('skiff test rejects duplicate singleton options and flags', async () => {
  const fixture = await fakeCargoFixture();
  try {
    for (const args of [
      ['--artifact-root', fixture.root, '--artifact-root', fixture.root],
      ['--artifact-root', fixture.root, '--base-assembly', 'one', '--base-assembly=two'],
      [
        '--artifact-root',
        fixture.root,
        '--base-config-snapshot',
        'one',
        '--base-config-snapshot=two',
      ],
      ['--artifact-root', fixture.root, '--live', '--live'],
      ['--artifact-root', fixture.root, '--deny-skips', '--deny-skips'],
    ]) {
      const result = await runProcess(process.execPath, [skiffPath, 'test', input, ...args], {
        env: fixture.env,
      });
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /provided more than once/);
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('skiff test requires base assembly and config snapshot together', async () => {
  const fixture = await fakeCargoFixture();
  try {
    for (const args of [
      ['--base-assembly', 'assembly'],
      ['--base-config-snapshot', 'snapshot'],
    ]) {
      const result = await runProcess(process.execPath, [
        skiffPath,
        'test',
        input,
        '--artifact-root',
        fixture.root,
        ...args,
      ], { env: fixture.env });
      assert.notEqual(result.code, 0);
      assert.match(
        result.stderr,
        /--base-assembly and --base-config-snapshot must be provided together/,
      );
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

async function fakeCargoFixture() {
  const fixtureRoot = await mkdtemp(join(tmpdir(), 'skiff-test-cli-'));
  const bin = join(fixtureRoot, 'bin');
  const marker = join(fixtureRoot, 'cargo-args.json');
  const cargo = join(bin, 'cargo');
  await mkdir(bin);
  await writeFile(cargo, [
    '#!/usr/bin/env node',
    "const fs = require('node:fs');",
    "fs.writeFileSync(process.env.SKIFF_FAKE_CARGO_MARKER, JSON.stringify(process.argv.slice(2)));",
    '',
  ].join('\n'));
  await chmod(cargo, 0o755);
  return {
    root: fixtureRoot,
    marker,
    env: {
      ...process.env,
      PATH: `${bin}${delimiter}${process.env.PATH ?? ''}`,
      SKIFF_FAKE_CARGO_MARKER: marker,
    },
  };
}

function runProcess(command, args, { env }) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd: root, env });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
}
