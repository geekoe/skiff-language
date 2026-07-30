import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import test from 'node:test';

import { liveSelectorPhases } from '../lib/verify-live-plan.mjs';

test('runtime-live forwards the absolute repository root exactly once', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-platform-source-'));
  try {
    const fixtureRoot = join(root, 'runtime', 'live-tests');
    const artifactRoot = join(root, 'artifacts');
    const bin = join(root, 'bin');
    await mkdir(fixtureRoot, { recursive: true });
    await mkdir(artifactRoot);
    await mkdir(bin);
    await writeFile(
      join(fixtureRoot, 'package.yml'),
      'id: example.com/runtime-live\nversion: 1.0.0\n',
    );
    await writeFile(
      join(fixtureRoot, 'config.skiff-test.yml'),
      '"example.com/runtime-live": {}\n',
    );
    await writeFile(join(fixtureRoot, 'context.live.test.skiff'), 'test "context" {}\n');
    for (const executable of ['cargo', 'node']) {
      const path = join(bin, executable);
      await writeFile(path, '#!/bin/sh\nexit 0\n');
      await chmod(path, 0o755);
    }

    const [phase] = await liveSelectorPhases(root, 'runtime-live', {
      runtimeLiveActivationUrl: 'http://router.test:4101/__skiff/activate-assembly',
      runtimeLiveIngressUrl: 'http://router.test:4100',
      runtimeLiveArtifactRoot: artifactRoot,
      runtimeLiveEnvironment: 'runtime-live',
      runtimeLiveExpectedGeneration: '0',
      env: { PATH: `${bin}${delimiter}${process.env.PATH ?? ''}` },
    });
    const indexes = phase.args
      .map((value, index) => (value === '--platform-source-root' ? index : -1))
      .filter((index) => index >= 0);
    assert.deepEqual(indexes, [phase.args.indexOf('--platform-source-root')]);
    assert.equal(phase.args[indexes[0] + 1], root);
    assert.equal(phase.cwd, root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
