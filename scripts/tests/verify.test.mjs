import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { parseVerifyArgs } from '../lib/verify-cli.mjs';
import {
  CHECKER_CLASSIFICATIONS,
  CHECKER_REGISTRY,
} from '../lib/verify-checkers.mjs';
import { assertVerifyCatalogComplete } from '../lib/verify-live-catalog.mjs';
import {
  discoverJavaScriptFiles,
  discoverScriptTests,
  repoRelative,
} from '../lib/verify-discovery.mjs';
import {
  assertPlanIntegrity,
  assertNonEmptyLeaf,
  buildVerifyPlan,
} from '../lib/verify-plan.mjs';
import { printVerifyPlan, runVerifyPlan } from '../lib/verify-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifyPath = join(root, 'scripts', 'verify.mjs');

test('CLI defaults to verify and accepts a package-manager argument separator', () => {
  assert.deepEqual(parseVerifyArgs([]).selectors, ['verify']);
  const parsed = parseVerifyArgs(['--only', 'tests', '--', '--list']);
  assert.deepEqual(parsed.selectors, ['tests']);
  assert.equal(parsed.list, true);
  assert.deepEqual(parseVerifyArgs(['--only', 'scripts-syntax']).selectors, [
    'scripts-syntax',
  ]);
  assert.throws(
    () => parseVerifyArgs(['--only', 'tests', '--', '--only', 'compiler']),
    /--only may be specified only once/,
  );
});

test('verify CLI rejects repeated runtime-live singleton inputs across split and inline forms', () => {
  const parsed = parseVerifyArgs([
    '--only=runtime-live',
    '--runtime-live-activation-url=http://router.test:4101/__skiff/activate-assembly',
    '--runtime-live-ingress-url',
    'http://router.test:4100',
    '--runtime-live-artifact-root=artifacts',
    '--runtime-live-profile=runtime-live',
    '--runtime-live-expected-generation',
    '7',
  ]);
  assert.equal(
    parsed.runtimeLiveActivationUrl,
    'http://router.test:4101/__skiff/activate-assembly',
  );
  assert.equal(parsed.runtimeLiveIngressUrl, 'http://router.test:4100');
  assert.equal(parsed.runtimeLiveArtifactRoot, 'artifacts');
  assert.equal(parsed.runtimeLiveProfile, 'runtime-live');
  assert.equal(parsed.runtimeLiveExpectedGeneration, '7');
  for (const args of [
    [
      '--runtime-live-activation-url',
      'http://one.test/__skiff/activate-assembly',
      '--runtime-live-activation-url=http://two.test/__skiff/activate-assembly',
    ],
    [
      '--runtime-live-ingress-url=http://router.test:4100',
      '--runtime-live-ingress-url',
      'http://other.test:4100',
    ],
    ['--runtime-live-artifact-root', 'one', '--runtime-live-artifact-root=two'],
    ['--runtime-live-profile', 'one', '--runtime-live-profile=two'],
    [
      '--runtime-live-expected-generation=1',
      '--runtime-live-expected-generation',
      '2',
    ],
    ['--list', '--dry-run'],
  ]) {
    assert.throws(() => parseVerifyArgs(args), /may be specified only once/);
  }
});

test('verify CLI accepts one loop-risk config and rejects split/inline duplicates', () => {
  assert.equal(
    parseVerifyArgs(['--loop-risk-config=config.json']).loopRiskConfig,
    'config.json',
  );
  for (const args of [
    ['--loop-risk-config', 'one.json', '--loop-risk-config=two.json'],
    ['--loop-risk-config=one.json', '--loop-risk-config', 'two.json'],
    ['--loop-risk-config='],
  ]) {
    assert.throws(
      () => parseVerifyArgs(args),
      /--loop-risk-config (?:may be specified only once|requires a value)/,
    );
  }
});

test('package scripts only forward to canonical verify selectors', async () => {
  const rootPackage = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  assert.equal(rootPackage.scripts.test, 'node scripts/verify.mjs --only tests');
  assert.equal(rootPackage.scripts.verify, 'node scripts/verify.mjs');
  assert.equal(rootPackage.scripts['type-check'], 'node scripts/verify.mjs --only type-check');

  const scriptsPackage = JSON.parse(
    await readFile(join(root, 'scripts', 'package.json'), 'utf8'),
  );
  assert.equal(scriptsPackage.scripts.test, 'node verify.mjs --only scripts');
  assert.equal(
    scriptsPackage.scripts['type-check'],
    'node verify.mjs --only scripts-syntax',
  );
  assert.equal(
    scriptsPackage.scripts['dev-sync:check-sync'],
    'node verify.mjs --only scripts-dev-sync',
  );
});

test('tooling selector has no Cargo task and discovers every scripts test', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['tooling'] });
  assert.equal(plan.tasks.some((task) => task.command === 'cargo'), false);

  const scriptTestTask = plan.tasks.find(
    (task) => task.id === 'implementation:tooling:scripts-tests',
  );
  assert.ok(scriptTestTask, 'tooling plan must contain one merged scripts-tests task');
  assert.deepEqual(scriptTestTask.args.slice(1), await discoverScriptTests(root));
  assert.ok(scriptTestTask.args.includes('scripts/tests/runtime-stack-deploy.test.mjs'));
  assert.ok(plan.tasks.some((task) =>
    task.id === 'implementation:tooling:dev-sync-fixture'));
});

test('compiler boundary selector is canonical and deduplicated across checks combinations', async () => {
  const focused = await buildVerifyPlan({ root, selectors: ['compiler-boundaries'] });
  assert.deepEqual(
    focused.tasks.map(({ id, args }) => ({ id, args })),
    [
      {
        id: 'checks:compiler-boundaries',
        args: ['scripts/check-compiler-boundaries.mjs'],
      },
    ],
  );

  const checks = await buildVerifyPlan({ root, selectors: ['checks'] });
  assert.equal(
    checks.tasks.filter((task) => task.id === 'checks:compiler-boundaries').length,
    1,
  );
  const combined = await buildVerifyPlan({
    root,
    selectors: ['checks', 'compiler-boundaries'],
  });
  assert.equal(
    combined.tasks.filter((task) => task.id === 'checks:compiler-boundaries').length,
    1,
  );

  const compiler = await buildVerifyPlan({ root, selectors: ['compiler'] });
  assert.equal(
    compiler.tasks.filter((task) => task.id === 'checks:compiler-boundaries').length,
    1,
  );
  assert.equal(compiler.tasks.filter((task) => task.command === 'cargo').length, 1);
});

test('runtime artifact boundary checker belongs to the runtime subject without duplicating Cargo', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['runtime'] });
  const boundaryTasks = plan.tasks.filter((task) =>
    task.args.includes('scripts/check-runtime-artifact-boundaries.mjs'));

  assert.deepEqual(
    boundaryTasks.map(({ id, command, args, kind }) => ({ id, command, args, kind })),
    [
      {
        id: 'implementation:runtime:artifact-boundaries:self-test',
        command: 'node',
        args: ['scripts/check-runtime-artifact-boundaries.mjs', '--self-test'],
        kind: 'implementation:runtime',
      },
      {
        id: 'implementation:runtime:artifact-boundaries',
        command: 'node',
        args: ['scripts/check-runtime-artifact-boundaries.mjs'],
        kind: 'implementation:runtime',
      },
    ],
  );
  assert.equal(plan.tasks.filter((task) => task.command === 'cargo').length, 1);
});

test('runtime execution and eval error boundary checkers belong to runtime and deduplicate with checks', async () => {
  const checks = await buildVerifyPlan({ root, selectors: ['checks'] });
  const executionTasks = checks.tasks.filter((task) =>
    task.args.includes('scripts/check-runtime-execution-boundaries.mjs'));
  assert.deepEqual(
    executionTasks.map(({ id, command, args, kind }) => ({ id, command, args, kind })),
    [
      {
        id: 'implementation:runtime:execution-boundaries:self-test',
        command: 'node',
        args: ['scripts/check-runtime-execution-boundaries.mjs', '--self-test'],
        kind: 'implementation:runtime',
      },
      {
        id: 'implementation:runtime:execution-boundaries',
        command: 'node',
        args: ['scripts/check-runtime-execution-boundaries.mjs'],
        kind: 'implementation:runtime',
      },
    ],
  );

  const runtime = await buildVerifyPlan({ root, selectors: ['runtime'] });
  assert.deepEqual(
    runtime.tasks
      .filter((task) =>
        task.args.includes('scripts/check-runtime-execution-boundaries.mjs')
        || task.args.includes('scripts/check-runtime-eval-error-boundary.mjs'))
      .map(({ id, kind, args }) => ({ id, kind, args })),
    [
      {
        id: 'implementation:runtime:execution-boundaries:self-test',
        kind: 'implementation:runtime',
        args: ['scripts/check-runtime-execution-boundaries.mjs', '--self-test'],
      },
      {
        id: 'implementation:runtime:execution-boundaries',
        kind: 'implementation:runtime',
        args: ['scripts/check-runtime-execution-boundaries.mjs'],
      },
      {
        id: 'implementation:runtime:eval-error-boundary:self-test',
        kind: 'implementation:runtime',
        args: ['scripts/check-runtime-eval-error-boundary.mjs', '--self-test'],
      },
      {
        id: 'implementation:runtime:eval-error-boundary',
        kind: 'implementation:runtime',
        args: ['scripts/check-runtime-eval-error-boundary.mjs'],
      },
    ],
  );
  assert.equal(
    runtime.tasks.filter((task) =>
      task.args.includes('scripts/check-runtime-artifact-boundaries.mjs')).length,
    2,
  );

  const combined = await buildVerifyPlan({ root, selectors: ['checks', 'runtime'] });
  assert.equal(
    combined.tasks.filter((task) =>
      task.args.includes('scripts/check-runtime-execution-boundaries.mjs')).length,
    2,
  );
  assert.equal(
    combined.tasks.filter((task) =>
      task.args.includes('scripts/check-runtime-eval-error-boundary.mjs')).length,
    2,
  );
});

test('verify list shows compiler boundaries once without known-red wording', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'checks,compiler-boundaries', '--list'],
    { cwd: root },
  );
  assert.equal(result.code, 0, result.stderr);
  assert.equal(
    (result.stdout.match(/scripts\/check-compiler-boundaries\.mjs/g) ?? []).length,
    1,
  );
  assert.doesNotMatch(result.stdout, /known-red|13 violations/);
});

test('verify checks list expands runtime execution boundary checker and self-test once', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'checks', '--list'],
    { cwd: root },
  );
  assert.equal(result.code, 0, result.stderr);
  assert.equal(
    (result.stdout.match(/scripts\/check-runtime-execution-boundaries\.mjs/g) ?? []).length,
    2,
  );
  assert.equal(
    (result.stdout.match(/scripts\/check-runtime-artifact-boundaries\.mjs/g) ?? []).length,
    0,
  );
});

test('canonical runtime-live plan aggregates every missing explicit input', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-missing-inputs-'));
  try {
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );
    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      env: {},
    });
    assert.equal(plan.tasks.length, 1);
    const [task] = plan.tasks;
    assert.equal(task.id, 'live:runtime:inputs');
    assert.match(task.preconditionError, /runtime-live is missing required explicit input/);
    for (const name of [
      'SKIFF_RUNTIME_LIVE_ACTIVATION_URL',
      'SKIFF_RUNTIME_LIVE_INGRESS_URL',
      'SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT',
      'SKIFF_RUNTIME_LIVE_ENVIRONMENT',
      'SKIFF_RUNTIME_LIVE_EXPECTED_GENERATION',
    ]) {
      assert.match(task.preconditionError, new RegExp(name));
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live CLI fails closed when canonical target inputs are absent', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'runtime-live'],
    { cwd: root, env: withoutRuntimeLiveTarget() },
  );
  assert.notEqual(result.code, 0, result.stdout);
  assert.match(
    `${result.stderr}\n${result.stdout}`,
    /runtime-live is missing required explicit input/,
  );
  assert.doesNotMatch(result.stdout, /All selected Skiff verification tasks passed/);
  assert.doesNotMatch(result.stdout, /SKIP/);
});

test('runtime-live blocks for every nonempty subset of missing required inputs', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-missing-matrix-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );
    const values = {
      runtimeLiveActivationUrl:
        'http://router.test:4101/__skiff/activate-assembly',
      runtimeLiveIngressUrl: 'http://router.test:4100',
      runtimeLiveArtifactRoot: artifactRoot,
      runtimeLiveProfile: 'runtime-live',
      runtimeLiveExpectedGeneration: '0',
    };
    const keys = Object.keys(values);
    for (let mask = 1; mask < (1 << keys.length); mask += 1) {
      const inputs = { ...values };
      for (let index = 0; index < keys.length; index += 1) {
        if ((mask & (1 << index)) !== 0) {
          delete inputs[keys[index]];
        }
      }
      const plan = await buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        env: {},
        ...inputs,
      });
      assert.equal(plan.tasks.length, 1);
      assert.match(plan.tasks[0].preconditionError, /missing required explicit input/);
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live blocked task never stops an earlier selected Cargo task', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-no-command-'));
  const bin = join(fixture, 'bin');
  const cargo = join(bin, 'cargo');
  const marker = join(fixture, 'cargo-ran');
  try {
    await mkdir(bin);
    await writeFile(cargo, [
      '#!/usr/bin/env node',
      "require('node:fs').writeFileSync(process.env.SKIFF_VERIFY_MARKER, 'ran');",
      '',
    ].join('\n'));
    await chmod(cargo, 0o755);
    const result = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'compiler,runtime-live'],
      {
        cwd: root,
        env: {
          ...withoutRuntimeLiveTarget(),
          PATH: `${bin}${delimiter}${process.env.PATH ?? ''}`,
          SKIFF_VERIFY_MARKER: marker,
        },
      },
    );
    assert.notEqual(result.code, 0);
    assert.match(
      `${result.stdout}\n${result.stderr}`,
      /runtime-live is missing required explicit input/,
    );
    await access(marker);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live fails closed for invalid paths or missing live fixtures', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-plan-'));
  try {
    const missingArtifactRoot = join(fixture, 'missing-artifacts');
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(missingArtifactRoot),
        env: { PATH: fixture },
      }),
      (error) => {
        assert.match(error.message, /found no \*\.live\.test\.skiff fixtures/);
        assert.match(error.message, /artifact root must be an existing directory/);
        return true;
      },
    );

    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(artifactRoot),
        env: { PATH: fixture },
      }),
      /runtime-live found no \*\.live\.test\.skiff fixtures/,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live requires the canonical package owner and fixed test profile', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-root-policy-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    const packageRoot = join(fixture, 'runtime', 'live-tests');
    await mkdir(artifactRoot);
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );

    await rm(join(packageRoot, 'package.yml'));
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(artifactRoot),
      }),
      /canonical source root must own package\.yml/,
    );

    await writeFile(
      join(packageRoot, 'package.yml'),
      'id: example.com/runtime-live-fixture\nversion: 1.0.0\n',
    );
    await rm(join(packageRoot, 'config.skiff-test.yml'));
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(artifactRoot),
      }),
      /canonical source root must own fixed config\.skiff-test\.yml/,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live builds executable Cargo tasks when config and fixtures exist', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-positive-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );

    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      ...runtimeLiveInputs(artifactRoot),
    });
    assert.equal(plan.tasks.length, 1);
    const [{ executionPreflight, ...task }] = plan.tasks;
    assert.equal(typeof executionPreflight, 'function');
    assert.equal(executionPreflight(), undefined);
    assert.deepEqual([task], [
      {
        id: 'live:runtime:runtime/live-tests/example.live.test.skiff',
        kind: 'live/manual',
        tier: 'live/manual',
        ownership: 'external',
        command: 'cargo',
        args: [
          'run',
          '--manifest-path',
          'test-runner/Cargo.toml',
          '--',
          join(fixture, 'runtime', 'live-tests', 'example.live.test.skiff'),
          '--live',
          '--artifact-root', artifactRoot,
          '--platform-source-root', fixture,
          '--activation-url',
          'http://router.test:4101/__skiff/activate-assembly',
          '--ingress-url',
          'http://router.test:4100',
          '--profile',
          'runtime-live',
          '--expected-generation',
          '0',
          '--deny-skips',
          '--require-tests',
        ],
        cwd: fixture,
      },
    ]);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live target profile does not select the source config profile', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-target-profile-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );
    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      ...runtimeLiveInputs(artifactRoot),
      runtimeLiveProfile: 'remote.prod',
    });

    assert.equal(plan.tasks.length, 1);
    const profileIndex = plan.tasks[0].args.indexOf('--profile');
    assert.equal(plan.tasks[0].args[profileIndex + 1], 'remote.prod');
    await assert.rejects(
      access(join(fixture, 'runtime', 'live-tests', 'config.remote.prod.yml')),
      { code: 'ENOENT' },
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live execution preflight fails only its tasks without stopping marker tasks', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-preflight-toctou-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    const removedFixture = join(
      fixture,
      'runtime/live-tests/first.live.test.skiff',
    );
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/first.live.test.skiff',
    );
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/second.live.test.skiff',
    );

    const built = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      ...runtimeLiveInputs(artifactRoot),
    });
    assert.equal(built.tasks.length, 2);
    assert.ok(built.tasks.every((task) => typeof task.executionPreflight === 'function'));
    assert.equal(new Set(built.tasks.map((task) => task.executionPreflight)).size, 1);

    const markers = [
      join(fixture, 'earlier-command-ran'),
      ...built.tasks.map((_, index) => join(fixture, `runtime-command-${index}-ran`)),
      join(fixture, 'later-command-ran'),
    ];
    const markerTask = (id, marker, executionPreflight) => ({
      id,
      kind: 'test',
      command: process.execPath,
      args: [
        '--eval',
        'require("node:fs").writeFileSync(process.argv[1], "ran")',
        marker,
      ],
      cwd: fixture,
      ...(executionPreflight === undefined ? {} : { executionPreflight }),
    });
    const plan = {
      selectors: ['test'],
      tasks: [
        markerTask('earlier-runs', markers[0]),
        ...built.tasks.map((task, index) => markerTask(
          task.id,
          markers[index + 1],
          task.executionPreflight,
        )),
        markerTask('later-runs', markers.at(-1)),
      ],
    };

    await rm(removedFixture);
    await rm(join(fixture, 'runtime', 'live-tests', 'package.yml'));
    await rm(join(fixture, 'runtime', 'live-tests', 'config.skiff-test.yml'));
    await rm(artifactRoot, { recursive: true });
    await writeFile(artifactRoot, 'replacement file\n');

    const summary = await runVerifyPlan(plan, fixture);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'earlier-runs', status: 'passed' },
        { id: built.tasks[0].id, status: 'failed' },
        { id: built.tasks[1].id, status: 'failed' },
        { id: 'later-runs', status: 'passed' },
      ],
    );
    for (const task of built.tasks) {
      const failed = summary.results.find((result) => result.id === task.id);
      assert.match(failed.reason, /runtime-live fixture is no longer/);
      assert.match(failed.reason, /runtime-live artifact root is no longer/);
      assert.match(failed.reason, /runtime-live package root is no longer canonical/);
      assert.match(failed.reason, /runtime-live package root no longer owns fixed config/);
    }
    await access(markers[0]);
    await assert.rejects(access(markers[1]), { code: 'ENOENT' });
    await assert.rejects(access(markers[2]), { code: 'ENOENT' });
    await access(markers[3]);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live rejects unsafe canonical URLs and wrong artifact-root types before execution', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-invalid-input-'));
  try {
    const artifactFile = join(fixture, 'not-a-directory');
    await writeFile(artifactFile, 'file\n');
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );

    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(artifactFile),
      }),
      /artifact root must be an existing directory/,
    );

    const sentinel = 'runtime-live-url-secret-sentinel';
    let error;
    try {
      await buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(fixture),
        runtimeLiveActivationUrl:
          `http://router.test:4101/__skiff/activate-assembly?token=${sentinel}`,
      });
    } catch (caught) {
      error = caught;
    }
    assert.match(error?.message ?? '', /must point exactly to \/__skiff\/activate-assembly/);
    assert.doesNotMatch(error?.message ?? '', new RegExp(sentinel));

    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        ...runtimeLiveInputs(fixture),
        runtimeLiveIngressUrl: 'http://router.test:4100/private',
      }),
      /runtime-live URL must point exactly to \//,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('generic development target profile cannot unlock runtime-live', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-profile-boundary-'));
  try {
    await writeCanonicalRuntimeLiveFixture(
      fixture,
      'runtime/live-tests/example.live.test.skiff',
    );
    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      env: {
        SKIFF_DEV_ACTIVATION_URL:
          'http://127.0.0.1:4001/__skiff/activate-assembly',
        SKIFF_TEST_RUNTIME_ARTIFACT_ROOT: '/stable/artifacts',
      },
    });
    assert.equal(plan.tasks.length, 1);
    assert.match(plan.tasks[0].preconditionError, /SKIFF_RUNTIME_LIVE_ACTIVATION_URL/);
    assert.match(plan.tasks[0].preconditionError, /SKIFF_RUNTIME_LIVE_INGRESS_URL/);
    assert.match(plan.tasks[0].preconditionError, /SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT/);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('real canonical runtime-live root renders exactly four ordered tasks', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-real-plan-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    const plan = await buildVerifyPlan({
      root,
      selectors: ['runtime-live'],
      ...runtimeLiveInputs(artifactRoot),
    });
    assert.deepEqual(
      plan.tasks.map((task) => task.id),
      [
        'live:runtime:runtime/live-tests/internal/db_live.live.test.skiff',
        'live:runtime:runtime/live-tests/internal/file_live.live.test.skiff',
        'live:runtime:runtime/live-tests/internal/http_adapter.live.test.skiff',
        'live:runtime:runtime/live-tests/internal/operation.live.test.skiff',
      ],
    );
    assert.deepEqual(
      plan.tasks.map((task) => {
        const index = task.args.indexOf('--expected-generation');
        return task.args[index + 1];
      }),
      ['0', '1', '2', '3'],
    );
    assert.ok(plan.tasks.every((task) => !task.args.includes('--base-assembly')));
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live CLI lists the canonical fixtures and hides invalid URL sentinels', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-list-display-'));
  try {
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    const listed = await runProcess(process.execPath, [
      verifyPath,
      '--only',
      'runtime-live',
      '--runtime-live-activation-url',
      'http://router.test:4101/__skiff/activate-assembly',
      '--runtime-live-ingress-url',
      'http://router.test:4100',
      '--runtime-live-artifact-root',
      artifactRoot,
      '--runtime-live-profile',
      'runtime-live',
      '--runtime-live-expected-generation',
      '0',
      '--list',
    ], { cwd: root });
    assert.equal(listed.code, 0, listed.stderr);
    for (const fixtureName of [
      'db_live.live.test.skiff',
      'file_live.live.test.skiff',
      'http_adapter.live.test.skiff',
      'operation.live.test.skiff',
    ]) {
      assert.match(listed.stdout, new RegExp(fixtureName.replaceAll('.', '\\.')));
    }
    assert.equal((listed.stdout.match(/--expected-generation/g) ?? []).length, 4);
    assert.doesNotMatch(listed.stdout, /--base-assembly/);

    const sentinel = 'verify-runtime-url-sentinel';
    const rejected = await runProcess(process.execPath, [
      verifyPath,
      '--only',
      'runtime-live',
      '--runtime-live-activation-url',
      `http://router.test:4101/__skiff/activate-assembly?token=${sentinel}`,
      '--runtime-live-ingress-url',
      'http://router.test:4100',
      '--runtime-live-artifact-root',
      artifactRoot,
      '--runtime-live-profile',
      'runtime-live',
      '--runtime-live-expected-generation',
      '0',
      '--list',
    ], { cwd: root });
    assert.notEqual(rejected.code, 0);
    assert.match(rejected.stderr, /must point exactly to \/__skiff\/activate-assembly/);
    assert.doesNotMatch(`${rejected.stdout}\n${rejected.stderr}`, new RegExp(sentinel));
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('duplicate task IDs are rejected', () => {
  const duplicate = {
    id: 'duplicate',
    kind: 'test',
    command: 'node',
    args: [],
    cwd: root,
  };
  assert.throws(
    () => assertPlanIntegrity([duplicate, { ...duplicate }]),
    /duplicate verify task id: duplicate/,
  );
});

test('empty plans and empty selector leaves fail closed even beside nonempty work', () => {
  assert.throws(() => assertPlanIntegrity([]), /at least one task/);
  assert.throws(() => assertNonEmptyLeaf('empty', []), /empty produced no tasks/);
  assert.throws(
    () => assertNonEmptyLeaf('empty', undefined),
    /empty produced no tasks/,
  );
  assert.doesNotThrow(() => assertNonEmptyLeaf('nonempty', [{ id: 'task' }]));
});

test('duplicate command executions are rejected even when task IDs differ', () => {
  const first = {
    id: 'first',
    kind: 'test',
    command: 'node',
    args: ['scripts/check-artifact-identity-single-source.mjs'],
    cwd: root,
  };
  assert.throws(
    () => assertPlanIntegrity([first, { ...first, id: 'renamed-duplicate' }]),
    /duplicate verify task execution: first and renamed-duplicate/,
  );
});

test('blocked tasks require a reason and cannot masquerade as executable tasks', () => {
  assert.throws(
    () => assertPlanIntegrity([{
      id: 'blocked-without-reason',
      kind: 'test',
      cwd: root,
      preconditionError: '  ',
    }]),
    /invalid blocked verify task/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      id: 'blocked-with-command',
      kind: 'test',
      cwd: root,
      preconditionError: 'missing config',
      command: 'node',
      args: [],
    }]),
    /invalid blocked verify task/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      id: 'blocked-with-preflight',
      kind: 'test',
      cwd: root,
      preconditionError: 'missing config',
      executionPreflight: () => undefined,
    }]),
    /invalid blocked verify task/,
  );
});

test('executable tasks validate displayArgs and executionPreflight contracts', () => {
  const task = {
    id: 'valid',
    kind: 'test',
    cwd: root,
    command: 'node',
    args: ['secret'],
  };
  assert.throws(
    () => assertPlanIntegrity([{ ...task, displayArgs: [] }]),
    /invalid verify task displayArgs/,
  );
  assert.throws(
    () => assertPlanIntegrity([{ ...task, executionPreflight: 'not-a-function' }]),
    /invalid verify task executionPreflight/,
  );
  assert.doesNotThrow(() => assertPlanIntegrity([{
    ...task,
    displayArgs: ['<redacted>'],
    executionPreflight: () => undefined,
  }]));
});

test('blocked tasks record blocked without stopping earlier or later work', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-blocked-runner-'));
  const markers = [
    join(fixture, 'earlier-task-ran'),
    join(fixture, 'later-task-ran'),
  ];
  try {
    const markerTask = (id, marker) => ({
      id,
      kind: 'test',
      command: process.execPath,
      args: [
        '--eval',
        'require("node:fs").writeFileSync(process.argv[1], "ran")',
        marker,
      ],
      cwd: fixture,
    });
    const plan = {
      selectors: ['test'],
      tasks: [
        markerTask('earlier-runs', markers[0]),
        {
          id: 'blocked-task',
          kind: 'test',
          cwd: fixture,
          preconditionError: 'missing required config',
        },
        markerTask('later-runs', markers[1]),
      ],
    };
    const summary = await runVerifyPlan(plan, fixture);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'earlier-runs', status: 'passed' },
        { id: 'blocked-task', status: 'blocked' },
        { id: 'later-runs', status: 'passed' },
      ],
    );
    assert.equal(summary.results[1].reason, 'missing required config');
    await Promise.all(markers.map((marker) => access(marker)));
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('preflight failures settle only their tasks and other tasks still run', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-execution-preflight-'));
  const marker = join(fixture, 'task-ran');
  const visited = [];
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        {
          id: 'would-run-first',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            'require("node:fs").writeFileSync(process.argv[1], "ran")',
            marker,
          ],
          cwd: fixture,
          executionPreflight: () => {
            visited.push('first');
            return 'first external prerequisite disappeared';
          },
        },
        {
          id: 'also-preflighted',
          kind: 'test',
          command: process.execPath,
          args: ['--eval', 'process.exit(0)'],
          cwd: fixture,
          executionPreflight: async () => {
            visited.push('second');
            throw new Error('second prerequisite is unreadable');
          },
        },
        {
          id: 'still-runs',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            'require("node:fs").writeFileSync(process.argv[1], "ran")',
            marker,
          ],
          cwd: fixture,
        },
      ],
    };
    const summary = await runVerifyPlan(plan, fixture);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'would-run-first', status: 'failed' },
        { id: 'also-preflighted', status: 'failed' },
        { id: 'still-runs', status: 'passed' },
      ],
    );
    assert.match(summary.results[0].reason, /first external prerequisite disappeared/);
    assert.match(summary.results[1].reason, /second prerequisite is unreadable/);
    assert.deepEqual(visited, ['first', 'second']);
    await access(marker);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('plan and execution logs use displayArgs without leaking execution-only sentinels', async () => {
  const sentinel = 'verify-display-secret-sentinel';
  const script = 'if (process.argv[1] !== "verify-display-secret-sentinel") process.exit(9)';
  const plan = {
    selectors: ['test'],
    tasks: [{
      id: 'redacted-display',
      kind: 'test',
      command: process.execPath,
      args: ['--eval', script, sentinel],
      displayArgs: ['--eval', '<redacted-script>', '<redacted-target>'],
      cwd: root,
    }],
  };
  const lines = [];
  const originalLog = console.log;
  console.log = (...values) => lines.push(values.join(' '));
  try {
    printVerifyPlan(plan, root);
    await runVerifyPlan(plan, root);
  } finally {
    console.log = originalLog;
  }
  const output = lines.join('\n');
  assert.doesNotMatch(output, new RegExp(sentinel));
  assert.match(output, /<redacted-target>/);
});

test('runner continues after the first failed task', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-runner-'));
  const marker = join(fixture, 'second-task-ran');
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        {
          id: 'first-fails',
          kind: 'test',
          command: process.execPath,
          args: ['--eval', 'process.exit(7)'],
          cwd: fixture,
        },
        {
          id: 'second-runs',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            'require("node:fs").writeFileSync(process.argv[1], "ran")',
            marker,
          ],
          cwd: fixture,
        },
      ],
    };
    const summary = await runVerifyPlan(plan, fixture);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'first-fails', status: 'failed' },
        { id: 'second-runs', status: 'passed' },
      ],
    );
    assert.match(summary.results[0].reason, /exited with 7/);
    await access(marker);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('missing task executable fails only that task and later commands still run', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-missing-command-'));
  const marker = join(fixture, 'later-task-ran');
  try {
    const plan = {
      selectors: ['test'],
      tasks: [
        {
          id: 'missing-executable-task',
          kind: 'test',
          command: `skiff-missing-executable-${process.pid}`,
          args: ['safe-test-arg'],
          cwd: fixture,
        },
        {
          id: 'later-runs',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            'require("node:fs").writeFileSync(process.argv[1], "ran")',
            marker,
          ],
          cwd: fixture,
        },
      ],
    };
    const summary = await runVerifyPlan(plan, fixture);
    assert.deepEqual(
      summary.results.map(({ id, status }) => ({ id, status })),
      [
        { id: 'missing-executable-task', status: 'failed' },
        { id: 'later-runs', status: 'passed' },
      ],
    );
    assert.match(summary.results[0].reason, /failed to spawn/);
    assert.match(summary.results[0].reason, /ENOENT/);
    await access(marker);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('package arguments cannot extend pnpm test beyond the tests selector', async () => {
  const result = await runProcess(
    'pnpm',
    ['test', '--', '--only', 'compiler', '--list'],
    { cwd: root },
  );
  assert.notEqual(result.code, 0, `pnpm test unexpectedly succeeded: ${result.stdout}`);
  assert.match(`${result.stderr}\n${result.stdout}`, /--only may be specified only once/);
});

test('filesystem discovery finds new tests and excludes generated or local directories', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-discovery-'));
  try {
    await Promise.all([
      write(fixture, 'scripts/tests/first.test.mjs'),
      write(fixture, 'scripts/tests/new.test.mjs'),
      write(fixture, 'scripts/tool.mjs'),
      write(fixture, 'nested/tool.cjs'),
      write(fixture, 'node_modules/ignored.js'),
      write(fixture, 'build/ignored.mjs'),
      write(fixture, 'target/ignored.js'),
      write(fixture, 'var/ignored.cjs'),
      write(fixture, '.stack/ignored.mjs'),
    ]);

    assert.deepEqual(await discoverScriptTests(fixture), [
      'scripts/tests/first.test.mjs',
      'scripts/tests/new.test.mjs',
    ]);
    assert.deepEqual(
      (await discoverJavaScriptFiles(fixture)).map((path) => repoRelative(fixture, path)),
      [
        'nested/tool.cjs',
        'scripts/tests/first.test.mjs',
        'scripts/tests/new.test.mjs',
        'scripts/tool.mjs',
      ],
    );

    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['scripts'],
    });
    assert.ok(
      plan.tasks.some((task) => task.args.includes('scripts/tests/new.test.mjs')),
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('every checker is classified once; compiler boundaries are default and live checks are not', async () => {
  await assertVerifyCatalogComplete(root);
  const compilerBoundaries = CHECKER_REGISTRY.find((entry) =>
    entry.path.endsWith('check-compiler-boundaries.mjs'),
  );
  assert.equal(compilerBoundaries?.classification, CHECKER_CLASSIFICATIONS.DEFAULT);
  const defaultPlan = await buildVerifyPlan({ root, selectors: ['checks'] });
  assert.equal(
    defaultPlan.tasks.filter((task) =>
      task.args.includes('scripts/check-compiler-boundaries.mjs')).length,
    1,
  );
  assert.equal(
    defaultPlan.tasks.filter((task) =>
      task.args.includes('--all-configured')).length,
    1,
  );
  assert.equal(
    defaultPlan.tasks.filter((task) =>
      task.args.includes('scripts/check-command-execution-policy.mjs')).length,
    1,
  );
  const toolingPlan = await buildVerifyPlan({ root, selectors: ['tooling'] });
  assert.equal(
    toolingPlan.tasks.filter((task) =>
      task.id === 'implementation:tooling:scripts-tests').length,
    1,
  );
  assert.ok(
    toolingPlan.tasks.some((task) =>
      task.args.includes('scripts/tests/command-execution-policy.test.mjs')),
  );
  assert.equal(defaultPlan.tasks.some((task) => task.id.startsWith('live:')), false);
});

test('direct CLI plans and filesystem discovery do not depend on the invocation cwd', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'skiff-verify-cwd-'));
  try {
    const scriptsResult = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'scripts', '--list'],
      { cwd },
    );
    assert.equal(scriptsResult.code, 0, scriptsResult.stderr);
    assert.match(scriptsResult.stdout, /scripts\/tests\/runtime-stack-deploy\.test\.mjs/);
    assert.match(
      scriptsResult.stdout,
      /scripts\/tests\/test-runner-runtime-isolation\.test\.mjs/,
    );

    const compilerResult = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'compiler', '--list'],
      { cwd },
    );
    assert.equal(compilerResult.code, 0, compilerResult.stderr);
    assert.match(compilerResult.stdout, /implementation:compiler:rust/);
    assert.match(compilerResult.stdout, /cargo test --no-fail-fast --package skiff-compiler-core/);
    assert.match(compilerResult.stdout, /cwd=\./);
  } finally {
    await rm(cwd, { recursive: true, force: true });
  }
});

async function write(rootPath, relativePath) {
  const path = join(rootPath, relativePath);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, 'export {};\n');
}

async function writeCanonicalRuntimeLiveFixture(rootPath, relativePath) {
  const path = join(rootPath, relativePath);
  const packageRoot = join(rootPath, 'runtime', 'live-tests');
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, 'test defaultRun false\n');
  await writeFile(
    join(packageRoot, 'package.yml'),
    'id: example.com/runtime-live-fixture\nversion: 1.0.0\n',
  );
  await writeFile(
    join(packageRoot, 'config.skiff-test.yml'),
    '"example.com/runtime-live-fixture": {}\n',
  );
}

function runProcess(command, args, { cwd, env = process.env }) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd, env });
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

function runtimeLiveInputs(artifactRoot) {
  return {
    runtimeLiveActivationUrl:
      'http://router.test:4101/__skiff/activate-assembly',
    runtimeLiveIngressUrl: 'http://router.test:4100',
    runtimeLiveArtifactRoot: artifactRoot,
    runtimeLiveProfile: 'runtime-live',
    runtimeLiveExpectedGeneration: '0',
  };
}

function withoutRuntimeLiveTarget() {
  const env = { ...process.env };
  delete env.SKIFF_RUNTIME_LIVE_ACTIVATION_URL;
  delete env.SKIFF_RUNTIME_LIVE_INGRESS_URL;
  delete env.SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT;
  delete env.SKIFF_RUNTIME_LIVE_ENVIRONMENT;
  delete env.SKIFF_RUNTIME_LIVE_EXPECTED_GENERATION;
  delete env.SKIFF_DEV_ACTIVATION_URL;
  delete env.SKIFF_TEST_RUNTIME_ARTIFACT_ROOT;
  return env;
}
