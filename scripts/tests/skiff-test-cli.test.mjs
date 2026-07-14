import assert from 'node:assert/strict';
import { access, chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, delimiter, join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const skiffPath = join(root, 'scripts', 'skiff.mjs');
const input = join(root, 'runtime', 'live-tests', 'internal', 'operation.live.test.skiff');

test('skiff test --live forwards explicit runtime target and strict result flags', async () => {
  const fixture = await fakeCargoFixture();
  try {
    const configPath = join(fixture.root, 'runtime-live.json');
    const artifactRoot = join(fixture.root, 'artifacts');
    await writeFile(configPath, '{}\n');
    await mkdir(artifactRoot);
    const reloadUrl = 'http://router.test:4101/__skiff/reload-artifacts';
    const result = await runProcess(process.execPath, [
      skiffPath,
      'test',
      input,
      '--live',
      '--allow-network',
      '--config',
      configPath,
      '--router-reload-url',
      reloadUrl,
      '--artifact-root',
      artifactRoot,
      '--deny-skips',
      '--require-tests',
    ], { env: fixture.env });
    assert.equal(result.code, 0, result.stderr);
    const args = JSON.parse(await readFile(fixture.marker, 'utf8'));
    assert.deepEqual(args.slice(args.indexOf('--') + 1), [
      input,
      '--live',
      '--allow-network',
      '--config',
      configPath,
      '--router-reload-url',
      reloadUrl,
      '--artifact-root',
      artifactRoot,
      '--deny-skips',
      '--require-tests',
    ]);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('non-live skiff test rejects caller-owned runtime targets before starting Cargo', async () => {
  const fixture = await fakeCargoFixture();
  try {
    for (const option of [
      ['--router-reload-url', 'http://router.test:4101'],
      ['--artifact-root', fixture.root],
    ]) {
      const result = await runProcess(
        process.execPath,
        [skiffPath, 'test', input, ...option],
        { env: fixture.env },
      );
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /non-live skiff test owns an isolated runtime target/);
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('live skiff test rejects missing and non-directory artifact roots before starting Cargo', async () => {
  const fixture = await fakeCargoFixture();
  try {
    const missingRoot = join(fixture.root, 'missing-artifacts');
    const artifactFile = join(fixture.root, 'artifact-file');
    await writeFile(artifactFile, 'not a directory\n');
    for (const artifactRoot of [missingRoot, artifactFile]) {
      const result = await runProcess(
        process.execPath,
        [
          skiffPath,
          'test',
          input,
          '--live',
          '--router-reload-url',
          'http://router.test:4101',
          '--artifact-root',
          artifactRoot,
        ],
        { env: fixture.env },
      );
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /skiff test --artifact-root must be an existing directory/);
      await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('live skiff test validates an explicit reload URL before starting Cargo', async () => {
  const fixture = await fakeCargoFixture();
  try {
    const artifactRoot = join(fixture.root, 'artifacts');
    await mkdir(artifactRoot);
    const sentinel = 'skiff-cli-reload-secret-sentinel';
    const result = await runProcess(
      process.execPath,
      [
        skiffPath,
        'test',
        input,
        '--live',
        '--router-reload-url',
        `http://router.test:4101/?token=${sentinel}`,
        '--artifact-root',
        artifactRoot,
      ],
      { env: fixture.env },
    );
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /reload_url_query/);
    assert.doesNotMatch(result.stderr, new RegExp(sentinel));
    await assert.rejects(access(fixture.marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('skiff test rejects duplicate singleton options and flags across split and inline forms', async () => {
  const fixture = await fakeCargoFixture();
  try {
    const cases = [
      [
        '--live',
        '--router-reload-url',
        'http://router.test:4101',
        '--router-reload-url=http://other.test:4101',
      ],
      ['--live', '--artifact-root=one', '--artifact-root', 'two'],
      ['--live', '--deny-skips', '--deny-skips'],
      ['--live', '--require-tests', '--require-tests'],
    ];
    for (const args of cases) {
      const result = await runProcess(
        process.execPath,
        [skiffPath, 'test', input, ...args],
        { env: fixture.env },
      );
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /provided more than once/);
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
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
}
