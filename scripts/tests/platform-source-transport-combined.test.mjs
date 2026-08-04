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
const rootDeployment = {
  contractVersion: '1.0.0',
  deploymentArtifactIdentity:
    `skiff-deployment-artifact-v2:sha256:${'1'.repeat(64)}`,
  deploymentRevision: 'revision-1',
  serviceId: 'example.com/service',
};

test('package and test transports share the platform root while assembly stays fileless', async () => {
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
      await runCompilerAuthoring({
        skiffRoot,
        kind: 'package',
        action: 'build',
        root: join(tempRoot, 'package'),
        artifactRoot,
      });
      await runCompilerAuthoring({
        skiffRoot,
        kind: 'assembly',
        action: 'build',
        artifactRoot,
        profile: 'combined-transport',
        rootDeployments: [rootDeployment],
      });
      const skiffResult = await runProcess(process.execPath, [
        join(skiffRoot, 'scripts', 'skiff.mjs'),
        'test',
        join(skiffRoot, 'runtime', 'live-tests', 'internal', 'operation.live.test.skiff'),
        '--artifact-root', artifactRoot,
        '--live',
        '--activation-url', 'http://router.test:4101/__skiff/activate-assembly',
        '--ingress-url', 'http://router.test:4100',
        '--profile', 'combined-transport',
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
    assertPlatformRoot(captured[0], 'package-authoring');
    assert.equal(captured[1].includes('--platform-source-root'), false);
    assert.deepEqual(
      captured[1].flatMap((value, index) => (
        value === '--root-deployment' ? [JSON.parse(captured[1][index + 1])] : []
      )),
      [rootDeployment],
    );
    const argv = [
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
          baseAssembly: `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`,
          baseConfigSnapshot:
            `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`,
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
          profile: 'combined-transport',
        }),
      },
      {
        label: 'isolated-bootstrap',
        args: bootstrapCanonicalArgs({
          skiffRoot,
          artifactRoot,
          profile: 'combined-transport',
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
          artifactRoot,
          baseAssembly:
            `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`,
          baseConfigSnapshot:
            `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`,
          activationUrl:
            'http://router.test:4101/__skiff/activate-assembly',
          ingressUrl: 'http://router.test:4100',
          profile: 'dev',
          expectedGeneration: 0,
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
      {
        id: 'actor-cross-package-top-level-alias',
        root: 'test-runner/fixtures/actor-cross-package-consumer-tests',
        subjectRoot: 'test-runner/fixtures/actor-cross-package-provider',
      },
      {
        id: 'actor-test-effect-capability',
        root: 'test-runner/fixtures/actor-full-chain-acceptance',
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
      '--profile', 'combined-transport',
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
