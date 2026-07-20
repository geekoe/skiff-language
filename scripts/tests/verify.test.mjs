import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createServer } from 'node:net';
import { delimiter, dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { parseVerifyArgs } from '../lib/verify-cli.mjs';
import { parseRuntimeReloadUrl } from '../lib/runtime-reload-url.mjs';
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
    '--runtime-live-config=config.json',
    '--runtime-live-reload-url',
    'http://router.test:4101',
    '--runtime-live-artifact-root=artifacts',
  ]);
  assert.equal(parsed.runtimeLiveConfig, 'config.json');
  assert.equal(parsed.runtimeLiveReloadUrl, 'http://router.test:4101');
  assert.equal(parsed.runtimeLiveArtifactRoot, 'artifacts');
  for (const args of [
    ['--runtime-live-config', 'one.json', '--runtime-live-config=two.json'],
    [
      '--runtime-live-reload-url=http://router.test:4101',
      '--runtime-live-reload-url',
      'http://other.test:4101',
    ],
    ['--runtime-live-artifact-root', 'one', '--runtime-live-artifact-root=two'],
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

test('tooling selector has no Cargo phase and discovers every scripts test', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['tooling'] });
  assert.equal(plan.phases.some((phase) => phase.command === 'cargo'), false);

  const scriptTestArgs = plan.phases
    .filter((phase) => phase.id.startsWith('implementation:tooling:scripts/tests/'))
    .map((phase) => phase.args.at(-1));
  assert.deepEqual(scriptTestArgs, await discoverScriptTests(root));
  assert.ok(scriptTestArgs.includes('scripts/tests/runtime-stack-deploy.test.mjs'));
  assert.ok(plan.phases.some((phase) =>
    phase.id === 'implementation:tooling:dev-sync-fixture'));
});

test('compiler boundary selector is canonical and deduplicated across checks combinations', async () => {
  const focused = await buildVerifyPlan({ root, selectors: ['compiler-boundaries'] });
  assert.deepEqual(
    focused.phases.map(({ id, args }) => ({ id, args })),
    [
      {
        id: 'checks:compiler-boundaries',
        args: ['scripts/check-compiler-boundaries.mjs'],
      },
    ],
  );

  const checks = await buildVerifyPlan({ root, selectors: ['checks'] });
  assert.equal(
    checks.phases.filter((phase) => phase.id === 'checks:compiler-boundaries').length,
    1,
  );
  const combined = await buildVerifyPlan({
    root,
    selectors: ['checks', 'compiler-boundaries'],
  });
  assert.equal(
    combined.phases.filter((phase) => phase.id === 'checks:compiler-boundaries').length,
    1,
  );
});

test('runtime artifact boundary checker belongs to the runtime subject without duplicating Cargo', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['runtime'] });
  const boundaryPhases = plan.phases.filter((phase) =>
    phase.args.includes('scripts/check-runtime-artifact-boundaries.mjs'));

  assert.deepEqual(
    boundaryPhases.map(({ id, command, args, kind }) => ({ id, command, args, kind })),
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
  assert.equal(plan.phases.filter((phase) => phase.command === 'cargo').length, 1);
});

test('runtime execution boundary checker belongs exactly once to checks', async () => {
  const checks = await buildVerifyPlan({ root, selectors: ['checks'] });
  const executionPhases = checks.phases.filter((phase) =>
    phase.args.includes('scripts/check-runtime-execution-boundaries.mjs'));
  assert.deepEqual(
    executionPhases.map(({ id, command, args, kind }) => ({ id, command, args, kind })),
    [
      {
        id: 'checks:runtime-execution-boundaries',
        command: 'node',
        args: ['scripts/check-runtime-execution-boundaries.mjs'],
        kind: 'default verify',
      },
    ],
  );

  const runtime = await buildVerifyPlan({ root, selectors: ['runtime'] });
  assert.equal(
    runtime.phases.some((phase) =>
      phase.args.includes('scripts/check-runtime-execution-boundaries.mjs')),
    false,
  );
  assert.equal(
    runtime.phases.filter((phase) =>
      phase.args.includes('scripts/check-runtime-artifact-boundaries.mjs')).length,
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

test('verify checks list expands the runtime execution boundary checker once', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'checks', '--list'],
    { cwd: root },
  );
  assert.equal(result.code, 0, result.stderr);
  assert.equal(
    (result.stdout.match(/scripts\/check-runtime-execution-boundaries\.mjs/g) ?? []).length,
    1,
  );
  assert.equal(
    (result.stdout.match(/scripts\/check-runtime-artifact-boundaries\.mjs/g) ?? []).length,
    0,
  );
});

test('runtime-live lists every missing explicit input in one blocked phase', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'runtime-live', '--list'],
    { cwd: root, env: withoutRuntimeLiveConfig() },
  );
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /live:runtime:inputs/);
  assert.match(
    result.stdout,
    /\[blocked: runtime-live is missing required explicit input\(s\):/,
  );
  assert.match(result.stdout, /SKIFF_RUNTIME_LIVE_CONFIG/);
  assert.match(result.stdout, /SKIFF_RUNTIME_LIVE_RELOAD_URL/);
  assert.match(result.stdout, /SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT/);
  assert.doesNotMatch(result.stdout, /\| node(?:\s|$)/);
  assert.doesNotMatch(result.stdout, /SKIP/);
});

test('runtime-live fails closed without config and never reports success', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'runtime-live'],
    { cwd: root, env: withoutRuntimeLiveConfig() },
  );
  assert.notEqual(result.code, 0, result.stdout);
  assert.match(
    `${result.stderr}\n${result.stdout}`,
    /live:runtime:inputs: runtime-live is missing required explicit input/,
  );
  assert.doesNotMatch(result.stdout, /All selected Skiff verification phases passed/);
  assert.doesNotMatch(result.stdout, /SKIP/);
});

test('runtime-live blocks for every nonempty subset of missing required inputs', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-missing-matrix-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    const artifactRoot = join(fixture, 'artifacts');
    await writeFile(configPath, '{}\n');
    await mkdir(artifactRoot);
    const values = {
      runtimeLiveConfig: configPath,
      runtimeLiveReloadUrl: 'http://router.test:4101',
      runtimeLiveArtifactRoot: artifactRoot,
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
        root,
        selectors: ['runtime-live'],
        env: {},
        ...inputs,
      });
      assert.equal(plan.phases.length, 1);
      assert.match(plan.phases[0].preconditionError, /missing required explicit input/);
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live blocker prevents an earlier selected Cargo phase from starting', async () => {
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
          ...withoutRuntimeLiveConfig(),
          PATH: `${bin}${delimiter}${process.env.PATH ?? ''}`,
          SKIFF_VERIFY_MARKER: marker,
        },
      },
    );
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /live:runtime:inputs/);
    await assert.rejects(access(marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live fails closed for invalid paths or missing live fixtures', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-plan-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    const artifactRoot = join(fixture, 'artifacts');
    await mkdir(artifactRoot);
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        runtimeLiveConfig: configPath,
        runtimeLiveReloadUrl: 'http://router.test:4101',
        runtimeLiveArtifactRoot: artifactRoot,
        env: { PATH: fixture },
      }),
      /runtime-live config path must be an existing file/,
    );

    await writeFile(configPath, '{}\n');
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        runtimeLiveConfig: configPath,
        runtimeLiveReloadUrl: 'http://router.test:4101',
        runtimeLiveArtifactRoot: artifactRoot,
        env: { PATH: fixture },
      }),
      /runtime-live found no \*\.live\.test\.skiff fixtures/,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live builds executable Cargo phases when config and fixtures exist', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-positive-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    const artifactRoot = join(fixture, 'artifacts');
    await writeFile(configPath, '{}\n');
    await mkdir(artifactRoot);
    await write(fixture, 'runtime/live-tests/example.live.test.skiff');

    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      runtimeLiveConfig: configPath,
      runtimeLiveReloadUrl: 'http://router.test:4101/',
      runtimeLiveArtifactRoot: artifactRoot,
    });
    assert.equal(plan.phases.length, 1);
    const [{ executionPreflight, ...phase }] = plan.phases;
    assert.equal(typeof executionPreflight, 'function');
    assert.equal(executionPreflight(), undefined);
    assert.deepEqual([phase], [
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
          '--allow-network',
          '--config',
          configPath,
          '--router-reload-url',
          'http://router.test:4101/__skiff/reload-artifacts',
          '--artifact-root',
          artifactRoot,
          '--deny-skips',
          '--require-tests',
        ],
        cwd: fixture,
        displayArgs: [
          'run',
          '--manifest-path',
          'test-runner/Cargo.toml',
          '--',
          join(fixture, 'runtime', 'live-tests', 'example.live.test.skiff'),
          '--live',
          '--allow-network',
          '--config',
          configPath,
          '--router-reload-url',
          'http://router.test:4101',
          '--artifact-root',
          artifactRoot,
          '--deny-skips',
          '--require-tests',
        ],
      },
    ]);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live execution preflight catches target TOCTOU before any command starts', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-preflight-toctou-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    const artifactRoot = join(fixture, 'artifacts');
    await writeFile(configPath, '{}\n');
    await mkdir(artifactRoot);
    await write(fixture, 'runtime/live-tests/first.live.test.skiff');
    await write(fixture, 'runtime/live-tests/second.live.test.skiff');

    const built = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      runtimeLiveConfig: configPath,
      runtimeLiveReloadUrl: 'http://router.test:4101',
      runtimeLiveArtifactRoot: artifactRoot,
    });
    assert.equal(built.phases.length, 2);
    assert.ok(built.phases.every((phase) => typeof phase.executionPreflight === 'function'));
    assert.equal(new Set(built.phases.map((phase) => phase.executionPreflight)).size, 1);

    const markers = [
      join(fixture, 'earlier-command-ran'),
      ...built.phases.map((_, index) => join(fixture, `runtime-command-${index}-ran`)),
      join(fixture, 'later-command-ran'),
    ];
    const markerPhase = (id, marker, executionPreflight) => ({
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
      phases: [
        markerPhase('earlier-must-not-run', markers[0]),
        ...built.phases.map((phase, index) => markerPhase(
          phase.id,
          markers[index + 1],
          phase.executionPreflight,
        )),
        markerPhase('later-must-not-run', markers.at(-1)),
      ],
    };

    await rm(configPath);
    await rm(artifactRoot, { recursive: true });
    await writeFile(artifactRoot, 'replacement file\n');

    await assert.rejects(
      runVerifyPlan(plan, fixture),
      (error) => {
        for (const phase of built.phases) {
          assert.match(
            error.message,
            new RegExp(`${escapeRegExp(phase.id)}: runtime-live config path is no longer`),
          );
          assert.match(
            error.message,
            new RegExp(`${escapeRegExp(phase.id)}: runtime-live artifact root is no longer`),
          );
        }
        return true;
      },
    );
    for (const marker of markers) {
      await assert.rejects(access(marker), { code: 'ENOENT' });
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('Node reload URL parser matches the shared Rust contract fixture and redacts rejects', async () => {
  const cases = JSON.parse(await readFile(
    join(root, 'test-runner', 'tests', 'fixtures', 'runtime-reload-url-cases.json'),
    'utf8',
  ));
  assert.equal(cases.version, 1);
  for (const entry of cases.accepted) {
    assert.deepEqual(parseRuntimeReloadUrl(entry.input), {
      baseUrl: entry.display,
      display: entry.display,
      normalized: entry.normalized,
    });
  }
  for (const entry of cases.rejected) {
    assert.throws(
      () => parseRuntimeReloadUrl(entry.input),
      (error) => {
        assert.equal(error.code, entry.reason);
        if (entry.input) {
          assert.doesNotMatch(error.message, new RegExp(escapeRegExp(entry.input)));
        }
        return true;
      },
    );
  }
});

test('runtime-live rejects unsafe reload URLs and wrong artifact-root types before execution', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-invalid-input-'));
  let connections = 0;
  const listener = createServer((socket) => {
    connections += 1;
    socket.destroy();
  });
  try {
    await new Promise((resolvePromise, reject) => {
      listener.once('error', reject);
      listener.listen(0, '127.0.0.1', resolvePromise);
    });
    const address = listener.address();
    assert.notEqual(address, null);
    assert.equal(typeof address, 'object');
    const configPath = join(fixture, 'runtime-live.json');
    const artifactFile = join(fixture, 'not-a-directory');
    await writeFile(configPath, '{}\n');
    await writeFile(artifactFile, 'file\n');
    await write(fixture, 'runtime/live-tests/example.live.test.skiff');

    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        catalogRoot: root,
        selectors: ['runtime-live'],
        runtimeLiveConfig: configPath,
        runtimeLiveReloadUrl: 'http://router.test:4101',
        runtimeLiveArtifactRoot: artifactFile,
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
        runtimeLiveConfig: configPath,
        runtimeLiveReloadUrl: `http://127.0.0.1:${address.port}/?token=${sentinel}`,
        runtimeLiveArtifactRoot: fixture,
      });
    } catch (caught) {
      error = caught;
    }
    assert.match(error?.message ?? '', /reload_url_query/);
    assert.doesNotMatch(error?.message ?? '', new RegExp(sentinel));
    await new Promise((resolvePromise) => setImmediate(resolvePromise));
    assert.equal(connections, 0);
  } finally {
    await new Promise((resolvePromise) => listener.close(resolvePromise));
    await rm(fixture, { recursive: true, force: true });
  }
});

test('generic development target environment cannot unlock runtime-live', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-env-boundary-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    await writeFile(configPath, '{}\n');
    await write(fixture, 'runtime/live-tests/example.live.test.skiff');
    const plan = await buildVerifyPlan({
      root: fixture,
      catalogRoot: root,
      selectors: ['runtime-live'],
      runtimeLiveConfig: configPath,
      env: {
        SKIFF_DEV_RELOAD_URL: 'http://127.0.0.1:4001/__skiff/reload-artifacts',
        SKIFF_TEST_ARTIFACT_ROOT: '/stable/artifacts',
      },
    });
    assert.equal(plan.phases.length, 1);
    assert.match(plan.phases[0].preconditionError, /SKIFF_RUNTIME_LIVE_RELOAD_URL/);
    assert.match(plan.phases[0].preconditionError, /SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT/);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('real runtime-live discovery renders every fixture once with strict explicit arguments', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-real-plan-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    const artifactRoot = join(fixture, 'artifacts');
    await writeFile(configPath, '{}\n');
    await mkdir(artifactRoot);
    const plan = await buildVerifyPlan({
      root,
      selectors: ['runtime-live'],
      runtimeLiveConfig: configPath,
      runtimeLiveReloadUrl: 'http://router.test:4101',
      runtimeLiveArtifactRoot: artifactRoot,
    });
    assert.equal(plan.phases.length, 4);
    assert.equal(new Set(plan.phases.map((phase) => phase.id)).size, 4);
    for (const phase of plan.phases) {
      assert.equal(typeof phase.executionPreflight, 'function');
      assert.ok(phase.args.includes('--deny-skips'));
      assert.ok(phase.args.includes('--require-tests'));
      assert.equal(phase.args[phase.args.indexOf('--config') + 1], configPath);
      assert.equal(
        phase.args[phase.args.indexOf('--router-reload-url') + 1],
        'http://router.test:4101/__skiff/reload-artifacts',
      );
      assert.equal(phase.args[phase.args.indexOf('--artifact-root') + 1], artifactRoot);
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime-live CLI list uses redacted display arguments and invalid URL errors hide raw sentinels', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-list-display-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    const artifactRoot = join(fixture, 'artifacts');
    await writeFile(configPath, '{}\n');
    await mkdir(artifactRoot);
    const listed = await runProcess(process.execPath, [
      verifyPath,
      '--only',
      'runtime-live',
      '--runtime-live-config',
      configPath,
      '--runtime-live-reload-url',
      'http://router.test:4101/__skiff/reload-artifacts',
      '--runtime-live-artifact-root',
      artifactRoot,
      '--list',
    ], { cwd: root });
    assert.equal(listed.code, 0, listed.stderr);
    assert.equal((listed.stdout.match(/live:runtime:/g) ?? []).length, 4);
    assert.match(listed.stdout, /--router-reload-url http:\/\/router\.test:4101/);
    assert.doesNotMatch(listed.stdout, /router\.test:4101\/__skiff\/reload-artifacts/);

    const sentinel = 'verify-runtime-url-sentinel';
    const rejected = await runProcess(process.execPath, [
      verifyPath,
      '--only',
      'runtime-live',
      '--runtime-live-config',
      configPath,
      '--runtime-live-reload-url',
      `http://router.test:4101/?token=${sentinel}`,
      '--runtime-live-artifact-root',
      artifactRoot,
      '--list',
    ], { cwd: root });
    assert.notEqual(rejected.code, 0);
    assert.match(rejected.stderr, /reload_url_query/);
    assert.doesNotMatch(`${rejected.stdout}\n${rejected.stderr}`, new RegExp(sentinel));
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('duplicate phase IDs are rejected', () => {
  const duplicate = {
    id: 'duplicate',
    kind: 'test',
    command: 'node',
    args: [],
    cwd: root,
  };
  assert.throws(
    () => assertPlanIntegrity([duplicate, { ...duplicate }]),
    /duplicate verify phase id: duplicate/,
  );
});

test('empty plans and empty selector leaves fail closed even beside nonempty work', () => {
  assert.throws(() => assertPlanIntegrity([]), /at least one phase/);
  assert.throws(() => assertNonEmptyLeaf('empty', []), /empty produced no phases/);
  assert.throws(
    () => assertNonEmptyLeaf('empty', undefined),
    /empty produced no phases/,
  );
  assert.doesNotThrow(() => assertNonEmptyLeaf('nonempty', [{ id: 'phase' }]));
});

test('duplicate command executions are rejected even when phase IDs differ', () => {
  const first = {
    id: 'first',
    kind: 'test',
    command: 'node',
    args: ['scripts/check-artifact-identity-single-source.mjs'],
    cwd: root,
  };
  assert.throws(
    () => assertPlanIntegrity([first, { ...first, id: 'renamed-duplicate' }]),
    /duplicate verify phase execution: first and renamed-duplicate/,
  );
});

test('blocked phases require a reason and cannot masquerade as executable phases', () => {
  assert.throws(
    () => assertPlanIntegrity([{
      id: 'blocked-without-reason',
      kind: 'test',
      cwd: root,
      preconditionError: '  ',
    }]),
    /invalid blocked verify phase/,
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
    /invalid blocked verify phase/,
  );
  assert.throws(
    () => assertPlanIntegrity([{
      id: 'blocked-with-preflight',
      kind: 'test',
      cwd: root,
      preconditionError: 'missing config',
      executionPreflight: () => undefined,
    }]),
    /invalid blocked verify phase/,
  );
});

test('executable phases validate displayArgs and executionPreflight contracts', () => {
  const phase = {
    id: 'valid',
    kind: 'test',
    cwd: root,
    command: 'node',
    args: ['secret'],
  };
  assert.throws(
    () => assertPlanIntegrity([{ ...phase, displayArgs: [] }]),
    /invalid verify phase displayArgs/,
  );
  assert.throws(
    () => assertPlanIntegrity([{ ...phase, executionPreflight: 'not-a-function' }]),
    /invalid verify phase executionPreflight/,
  );
  assert.doesNotThrow(() => assertPlanIntegrity([{
    ...phase,
    displayArgs: ['<redacted>'],
    executionPreflight: () => undefined,
  }]));
});

test('runner aggregates static blockers before spawning any earlier or later work', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-blocked-runner-'));
  const marker = join(fixture, 'later-phase-ran');
  try {
    const plan = {
      selectors: ['test'],
      phases: [
        {
          id: 'earlier-must-not-run',
          kind: 'test',
          command: process.execPath,
          args: [
            '--eval',
            'require("node:fs").writeFileSync(process.argv[1], "ran")',
            marker,
          ],
          cwd: fixture,
        },
        {
          id: 'blocked-phase',
          kind: 'test',
          cwd: fixture,
          preconditionError: 'missing required config',
        },
        {
          id: 'later-must-not-run',
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
    await assert.rejects(
      runVerifyPlan(plan, fixture),
      /blocked-phase: missing required config/,
    );
    await assert.rejects(access(marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runner executes every read-only preflight, aggregates failures, and starts no command', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-execution-preflight-'));
  const marker = join(fixture, 'phase-ran');
  const visited = [];
  try {
    const plan = {
      selectors: ['test'],
      phases: [
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
          id: 'static-blocker',
          kind: 'test',
          cwd: fixture,
          preconditionError: 'static input missing',
        },
      ],
    };
    await assert.rejects(
      runVerifyPlan(plan, fixture),
      (error) => {
        assert.match(error.message, /would-run-first: first external prerequisite disappeared/);
        assert.match(error.message, /also-preflighted: second prerequisite is unreadable/);
        assert.match(error.message, /static-blocker: static input missing/);
        return true;
      },
    );
    assert.deepEqual(visited, ['first', 'second']);
    await assert.rejects(access(marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('plan and execution logs use displayArgs without leaking execution-only sentinels', async () => {
  const sentinel = 'verify-display-secret-sentinel';
  const script = 'if (process.argv[1] !== "verify-display-secret-sentinel") process.exit(9)';
  const plan = {
    selectors: ['test'],
    phases: [{
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

test('runner is fail-fast after the first failed phase', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-runner-'));
  const marker = join(fixture, 'second-phase-ran');
  try {
    const plan = {
      selectors: ['test'],
      phases: [
        {
          id: 'first-fails',
          kind: 'test',
          command: process.execPath,
          args: ['--eval', 'process.exit(7)'],
          cwd: fixture,
        },
        {
          id: 'second-must-not-run',
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
    await assert.rejects(runVerifyPlan(plan, fixture), /first-fails failed with 7/);
    await assert.rejects(access(marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('missing phase executable keeps phase identity and stops later commands', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-missing-command-'));
  const marker = join(fixture, 'later-phase-ran');
  try {
    const plan = {
      selectors: ['test'],
      phases: [
        {
          id: 'missing-executable-phase',
          kind: 'test',
          command: `skiff-missing-executable-${process.pid}`,
          args: ['safe-test-arg'],
          cwd: fixture,
        },
        {
          id: 'later-must-not-run',
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
    await assert.rejects(
      runVerifyPlan(plan, fixture),
      /missing-executable-phase failed with ENOENT/,
    );
    await assert.rejects(access(marker), { code: 'ENOENT' });
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
      write(fixture, '.skiff-instance/ignored.mjs'),
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
    assert.ok(plan.phases.some((phase) => phase.id.endsWith('new.test.mjs')));
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
    defaultPlan.phases.filter((phase) =>
      phase.args.includes('scripts/check-compiler-boundaries.mjs')).length,
    1,
  );
  assert.equal(
    defaultPlan.phases.filter((phase) =>
      phase.args.includes('--all-configured')).length,
    1,
  );
  assert.equal(
    defaultPlan.phases.filter((phase) =>
      phase.args.includes('scripts/check-command-execution-policy.mjs')).length,
    1,
  );
  const toolingPlan = await buildVerifyPlan({ root, selectors: ['tooling'] });
  assert.equal(
    toolingPlan.phases.filter((phase) =>
      phase.id === 'implementation:tooling:scripts/tests/command-execution-policy.test.mjs')
      .length,
    1,
  );
  assert.equal(defaultPlan.phases.some((phase) => phase.id.startsWith('live:')), false);
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

function withoutRuntimeLiveConfig() {
  const env = { ...process.env };
  delete env.SKIFF_RUNTIME_LIVE_CONFIG;
  delete env.SKIFF_RUNTIME_LIVE_RELOAD_URL;
  delete env.SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT;
  delete env.SKIFF_DEV_RELOAD_URL;
  delete env.SKIFF_TEST_ARTIFACT_ROOT;
  return env;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
