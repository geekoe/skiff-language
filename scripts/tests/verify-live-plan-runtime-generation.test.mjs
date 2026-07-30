import assert from 'node:assert/strict';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import test from 'node:test';

import { liveSelectorPhases } from '../lib/verify-live-plan.mjs';

const RUNTIME_FIXTURES = [
  'db_live.live.test.skiff',
  'file_live.live.test.skiff',
  'http_adapter.live.test.skiff',
  'operation.live.test.skiff',
];

test('runtime-live assigns a consecutive generation to each canonical fixture', async () => {
  const fixture = await runtimeFixture();
  try {
    const phases = await liveSelectorPhases(fixture.root, 'runtime-live', {
      ...fixture.inputs,
      runtimeLiveExpectedGeneration: '9',
    });

    assert.deepEqual(
      phases.map((phase) => optionValue(phase.args, '--expected-generation')),
      ['9', '10', '11', '12'],
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('runtime-live preserves the caller generation for a single fixture', async () => {
  const fixture = await runtimeFixture({
    fixtureNames: ['operation.live.test.skiff'],
  });
  try {
    const [phase] = await liveSelectorPhases(fixture.root, 'runtime-live', {
      ...fixture.inputs,
      runtimeLiveExpectedGeneration: '41',
    });

    assert.equal(optionValue(phase.args, '--expected-generation'), '41');
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('runtime-live retains canonical runner policy across every generation phase', async () => {
  const fixture = await runtimeFixture();
  try {
    const phases = await liveSelectorPhases(fixture.root, 'runtime-live', {
      ...fixture.inputs,
      runtimeLiveExpectedGeneration: '0',
    });

    assert.equal(phases.length, RUNTIME_FIXTURES.length);
    for (const phase of phases) {
      assert.equal(optionIndexes(phase.args, '--platform-source-root').length, 1);
      assert.equal(optionValue(phase.args, '--platform-source-root'), fixture.root);
      assert.equal(phase.args.includes('--base-assembly'), false);
      assert.equal(phase.args.filter((arg) => arg === '--deny-skips').length, 1);
      assert.equal(phase.args.filter((arg) => arg === '--require-tests').length, 1);
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('runtime-live increments large generations without Number precision loss', async () => {
  const fixture = await runtimeFixture();
  try {
    const phases = await liveSelectorPhases(fixture.root, 'runtime-live', {
      ...fixture.inputs,
      runtimeLiveExpectedGeneration: '9007199254740987',
    });

    assert.deepEqual(
      phases.map((phase) => optionValue(phase.args, '--expected-generation')),
      [
        '9007199254740987',
        '9007199254740988',
        '9007199254740989',
        '9007199254740990',
      ],
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('runtime-live rejects invalid or overflowing generation sequences before planning', async () => {
  const fixture = await runtimeFixture();
  try {
    for (const value of ['-1', '+1', '01', '1.0', '1e2', ' 1', '1 ']) {
      await assert.rejects(
        liveSelectorPhases(fixture.root, 'runtime-live', {
          ...fixture.inputs,
          runtimeLiveExpectedGeneration: value,
        }),
        /expected generation must be a non-negative integer/,
      );
    }
    for (const value of [
      '9007199254740988',
      '9007199254740991',
      '18446744073709551615',
    ]) {
      await assert.rejects(
        liveSelectorPhases(fixture.root, 'runtime-live', {
          ...fixture.inputs,
          runtimeLiveExpectedGeneration: value,
        }),
        /expected generation sequence.*must not exceed 9007199254740990/,
      );
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

async function runtimeFixture({ fixtureNames = RUNTIME_FIXTURES } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-generation-'));
  const packageRoot = join(root, 'runtime', 'live-tests');
  const internalRoot = join(packageRoot, 'internal');
  const artifactRoot = join(root, 'artifacts');
  const bin = join(root, 'bin');
  await mkdir(internalRoot, { recursive: true });
  await mkdir(artifactRoot);
  await mkdir(bin);
  await writeFile(
    join(packageRoot, 'package.yml'),
    'id: example.com/runtime-live\nversion: 1.0.0\n',
  );
  await writeFile(
    join(packageRoot, 'config.skiff-test.yml'),
    '"example.com/runtime-live": {}\n',
  );
  await Promise.all(fixtureNames.map((name) =>
    writeFile(join(internalRoot, name), 'test defaultRun false\n')));
  await Promise.all(['cargo', 'node'].map(async (executable) => {
    const path = join(bin, executable);
    await writeFile(path, '#!/bin/sh\nexit 0\n');
    await chmod(path, 0o755);
  }));
  return {
    root,
    inputs: {
      runtimeLiveActivationUrl:
        'http://router.test:4101/__skiff/activate-assembly',
      runtimeLiveIngressUrl: 'http://router.test:4100',
      runtimeLiveArtifactRoot: artifactRoot,
      runtimeLiveEnvironment: 'runtime-live',
      env: { PATH: `${bin}${delimiter}${process.env.PATH ?? ''}` },
    },
  };
}

function optionValue(args, option) {
  const indexes = optionIndexes(args, option);
  assert.notEqual(indexes.length, 0, `missing ${option}`);
  return args[indexes[0] + 1];
}

function optionIndexes(args, option) {
  return args.flatMap((arg, index) => (arg === option ? [index] : []));
}
