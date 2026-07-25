import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { encryptedStorageTestRunnerArgs } from '../lib/encrypted-storage-live-harness.mjs';
import { bootstrapCanonicalArgs } from '../lib/isolated-test-runtime-instance.mjs';
import { runCompilerAuthoring } from '../lib/package-service-authoring.mjs';
import { canonicalSkiffSourceTestRegistry } from '../lib/skiff-source-test-registry.mjs';
import {
  packageServiceHostFixturePrepareCargoArgs,
  skiffSourceTestRunnerCargoArgs,
} from '../lib/skiff-source-test-suite.mjs';
import { runtimeLivePlatformSourceArgs } from '../lib/verify-live-plan.mjs';

const skiffRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('merged compiler and test transports share one absolute platform root', async () => {
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-platform-transport-combined-'));
  try {
    const marker = join(tempRoot, 'cargo-argv.jsonl');
    const fakeBin = join(tempRoot, 'bin');
    const artifactRoot = join(tempRoot, 'artifacts');
    await mkdir(fakeBin);
    await mkdir(artifactRoot);
    const fakeCargo = join(fakeBin, 'cargo');
    await writeFile(fakeCargo, [
      '#!/usr/bin/env node',
      "const fs = require('node:fs');",
      "fs.appendFileSync(process.env.SKIFF_COMBINED_CARGO_MARKER, `${JSON.stringify(process.argv.slice(2))}\\n`);",
      "process.stdout.write('{}\\n');",
    ].join('\n'));
    await chmod(fakeCargo, 0o755);

    const previousPath = process.env.PATH;
    const previousMarker = process.env.SKIFF_COMBINED_CARGO_MARKER;
    process.env.PATH = `${fakeBin}${delimiter}${previousPath ?? ''}`;
    process.env.SKIFF_COMBINED_CARGO_MARKER = marker;
    try {
      for (const kind of ['package', 'assembly']) {
        await runCompilerAuthoring({
          skiffRoot,
          kind,
          action: 'build',
          root: join(tempRoot, kind),
          artifactRoot,
        });
      }
      const skiffResult = await runProcess(process.execPath, [
        join(skiffRoot, 'scripts', 'skiff.mjs'),
        'test',
        join(skiffRoot, 'runtime', 'live-tests', 'internal', 'operation.live.test.skiff'),
        '--artifact-root', artifactRoot,
        '--live',
        '--activation-url', 'http://router.test:4101/__skiff/activate-assembly',
        '--ingress-url', 'http://router.test:4100',
        '--environment', 'combined-transport',
        '--expected-generation', '0',
      ], {
        cwd: tempRoot,
        env: process.env,
      });
      assert.equal(skiffResult.code, 0, skiffResult.stderr);
    } finally {
      if (previousPath === undefined) delete process.env.PATH;
      else process.env.PATH = previousPath;
      if (previousMarker === undefined) delete process.env.SKIFF_COMBINED_CARGO_MARKER;
      else process.env.SKIFF_COMBINED_CARGO_MARKER = previousMarker;
    }

    const captured = (await readFile(marker, 'utf8'))
      .trim()
      .split(/\r?\n/)
      .map((line) => JSON.parse(line));
    assert.equal(captured.length, 3);
    const argv = [
      ...captured.map((args, index) => ({ label: `authoring-${index}`, args })),
      {
        label: 'source-suite-std',
        args: skiffSourceTestRunnerCargoArgs({
          skiffRoot,
          root: join(skiffRoot, 'test-services', 'std'),
          artifactRoot,
        }),
      },
      {
        label: 'source-suite-host-consumer',
        args: skiffSourceTestRunnerCargoArgs({
          skiffRoot,
          root: join(skiffRoot, 'test-runner/fixtures/package-service-host/consumer-tests'),
          artifactRoot,
          baseAssembly: `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`,
        }),
      },
      {
        label: 'host-preparer',
        args: packageServiceHostFixturePrepareCargoArgs({
          skiffRoot,
          fixtureRoot: join(skiffRoot, 'test-runner/fixtures/package-service-host'),
          artifactRoot,
          workRoot: join(tempRoot, 'host-work'),
          receipt: join(tempRoot, 'host-receipt.json'),
          environment: 'combined-transport',
        }),
      },
      {
        label: 'isolated-bootstrap',
        args: bootstrapCanonicalArgs({
          skiffRoot,
          artifactRoot,
          environment: 'combined-transport',
        }),
      },
      {
        label: 'runtime-live',
        args: runtimeLivePlatformSourceArgs(skiffRoot),
      },
      {
        label: 'encrypted-storage',
        args: encryptedStorageTestRunnerArgs({
          testFile: '/tmp/encrypted.live.test.skiff',
          configPath: '/tmp/test-runner-live.json',
        }),
      },
    ];
    for (const entry of argv) {
      assertPlatformRoot(entry.args, entry.label);
    }
    assert.deepEqual(canonicalSkiffSourceTestRegistry, [
      { id: 'std', root: 'test-services/std' },
      {
        id: 'alias-return-catch-once',
        root: 'test-runner/fixtures/alias-return-catch-once-tests',
        subjectRoot: 'test-runner/fixtures/alias-return-catch-once',
      },
    ]);

    const omitted = await runProcess('cargo', [
      'run',
      '--quiet',
      '--locked',
      '--manifest-path', join(skiffRoot, 'test-runner', 'Cargo.toml'),
      '--bin', 'skiff-package-service-smoke-fixture',
      '--',
      '--bootstrap-only',
      '--artifact-root', artifactRoot,
      '--environment', 'combined-transport',
    ], { cwd: tempRoot, env: process.env });
    assert.notEqual(omitted.code, 0);
    assert.match(omitted.stderr, /missing --platform-source-root/);
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }
});

function assertPlatformRoot(args, label) {
  const indexes = args
    .map((value, index) => (value === '--platform-source-root' ? index : -1))
    .filter((index) => index >= 0);
  assert.equal(indexes.length, 1, `${label} must pass the platform root exactly once`);
  assert.equal(args[indexes[0] + 1], skiffRoot, `${label} must pass the module-owned root`);
}

function runProcess(command, args, { cwd, env }) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd, env });
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
