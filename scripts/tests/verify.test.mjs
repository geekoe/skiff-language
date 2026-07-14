import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { parseVerifyArgs } from '../lib/verify-cli.mjs';
import {
  CHECKER_CLASSIFICATIONS,
  CHECKER_REGISTRY,
  assertCheckerRegistryComplete,
} from '../lib/verify-checkers.mjs';
import {
  discoverJavaScriptFiles,
  discoverScriptTests,
  repoRelative,
} from '../lib/verify-discovery.mjs';
import {
  assertPlanIntegrity,
  buildVerifyPlan,
} from '../lib/verify-plan.mjs';
import { runVerifyPlan } from '../lib/verify-runner.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifyPath = join(root, 'scripts', 'verify.mjs');

test('CLI defaults to verify and accepts a package-manager argument separator', () => {
  assert.deepEqual(parseVerifyArgs([]).selectors, ['verify']);
  const parsed = parseVerifyArgs(['--only', 'node', '--', '--list']);
  assert.deepEqual(parsed.selectors, ['node']);
  assert.equal(parsed.list, true);
  assert.deepEqual(parseVerifyArgs(['--only', 'scripts-syntax']).selectors, [
    'scripts-syntax',
  ]);
  assert.throws(
    () => parseVerifyArgs(['--only', 'node', '--', '--only', 'rust']),
    /--only may be specified only once/,
  );
});

test('package scripts only forward to canonical verify selectors', async () => {
  const rootPackage = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  assert.equal(rootPackage.scripts.test, 'node scripts/verify.mjs --only node');
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

test('CI runs exactly the three canonical non-live scopes with frozen package installs', async () => {
  const workflow = await readFile(
    join(root, '.github', 'workflows', 'verify.yml'),
    'utf8',
  );
  const commands = [...workflow.matchAll(/^            command: (.+)$/gm)]
    .map((match) => match[1]);
  assert.deepEqual(commands, [
    'cargo test --workspace --no-fail-fast',
    'pnpm test',
    'node scripts/verify.mjs --only checks',
  ]);
  const installedPackages = [
    ...workflow.matchAll(/pnpm --dir (\S+) install --frozen-lockfile/g),
  ].map((match) => match[1]);
  assert.deepEqual(installedPackages, ['router', 'telemetry', 'scripts', 'vscode']);
  assert.match(workflow, /uses: actions\/checkout@v6\n\s+with:\n\s+persist-credentials: false/);
  assert.match(workflow, /uses: actions\/setup-node@v6/);
  assert.match(workflow, /package-manager-cache: false/);
  assert.doesNotMatch(workflow, /pnpm verify/);
  assert.doesNotMatch(workflow, /--manifest-path|scripts\/tests|node scripts\/check-/);
  assert.doesNotMatch(
    workflow,
    /runtime-live|db-encrypted-storage-live|compiler-boundaries|loop-risk/,
  );
});

test('rust selector contains exactly the workspace Cargo authority', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['rust'] });
  assert.deepEqual(
    plan.phases.map(({ id, command, args }) => ({ id, command, args })),
    [
      {
        id: 'rust:workspace',
        command: 'cargo',
        args: ['test', '--workspace', '--no-fail-fast'],
      },
    ],
  );
});

test('node selector has no top-level Cargo phase and discovers every scripts test', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['node'] });
  assert.equal(plan.phases.some((phase) => phase.command === 'cargo'), false);

  const scriptTestArgs = plan.phases
    .filter((phase) => phase.id.startsWith('scripts:test:'))
    .map((phase) => phase.args.at(-1));
  assert.deepEqual(scriptTestArgs, await discoverScriptTests(root));
  assert.ok(scriptTestArgs.includes('scripts/tests/runtime-stack-deploy.test.mjs'));
  assert.ok(plan.phases.some((phase) => phase.id === 'scripts:dev-sync-fixture'));
});

test('default verify has one Rust workspace and one operation ABI check, without live phases', async () => {
  const plan = await buildVerifyPlan({ root });
  assert.equal(plan.phases.filter((phase) => phase.id === 'rust:workspace').length, 1);
  assert.equal(
    plan.phases.filter((phase) => phase.id === 'checks:operation-abi-identity').length,
    1,
  );
  assert.equal(plan.phases.some((phase) => phase.id.startsWith('live:')), false);
});

test('runtime-live lists a blocked phase when its required config is missing', async () => {
  const result = await runProcess(
    process.execPath,
    [verifyPath, '--only', 'runtime-live', '--list'],
    { cwd: root, env: withoutRuntimeLiveConfig() },
  );
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /live:runtime:config/);
  assert.match(
    result.stdout,
    /\[blocked: set SKIFF_RUNTIME_LIVE_CONFIG or pass --runtime-live-config <path>/,
  );
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
    /live:runtime:config cannot run: set SKIFF_RUNTIME_LIVE_CONFIG/,
  );
  assert.doesNotMatch(result.stdout, /All selected Skiff verification phases passed/);
  assert.doesNotMatch(result.stdout, /SKIP/);
});

test('runtime-live fails closed for a missing config path or missing live fixtures', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-runtime-live-plan-'));
  try {
    const configPath = join(fixture, 'runtime-live.json');
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        selectors: ['runtime-live'],
        runtimeLiveConfig: configPath,
      }),
      /runtime-live config path does not exist/,
    );

    await writeFile(configPath, '{}\n');
    await assert.rejects(
      buildVerifyPlan({
        root: fixture,
        selectors: ['runtime-live'],
        runtimeLiveConfig: configPath,
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
    await writeFile(configPath, '{}\n');
    await write(fixture, 'runtime/live-tests/example.live.test.skiff');

    const plan = await buildVerifyPlan({
      root: fixture,
      selectors: ['runtime-live'],
      runtimeLiveConfig: configPath,
    });
    assert.deepEqual(plan.phases, [
      {
        id: 'live:runtime:runtime/live-tests/example.live.test.skiff',
        kind: 'live/manual',
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
        ],
        cwd: fixture,
      },
    ]);
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

test('duplicate command executions are rejected even when phase IDs differ', () => {
  const first = {
    id: 'first',
    kind: 'test',
    command: 'node',
    args: ['scripts/check-operation-abi-identity-single-source.mjs'],
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
});

test('runner rejects a blocked phase without spawning later work', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-verify-blocked-runner-'));
  const marker = join(fixture, 'later-phase-ran');
  try {
    const plan = {
      selectors: ['test'],
      phases: [
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
      /blocked-phase cannot run: missing required config/,
    );
    await assert.rejects(access(marker), { code: 'ENOENT' });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
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

test('package arguments cannot extend pnpm test beyond the node selector', async () => {
  const result = await runProcess(
    'pnpm',
    ['test', '--', '--only', 'rust', '--list'],
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

    const plan = await buildVerifyPlan({ root: fixture, selectors: ['scripts'] });
    assert.ok(plan.phases.some((phase) => phase.id.endsWith('new.test.mjs')));
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('every checker is classified once and known-red/live checks are not default', async () => {
  await assertCheckerRegistryComplete(root);
  const compilerBoundaries = CHECKER_REGISTRY.find((entry) =>
    entry.path.endsWith('check-compiler-boundaries.mjs'),
  );
  assert.equal(compilerBoundaries?.classification, CHECKER_CLASSIFICATIONS.KNOWN_RED);
  const defaultPlan = await buildVerifyPlan({ root, selectors: ['checks'] });
  assert.equal(
    defaultPlan.phases.some((phase) => phase.args.includes('scripts/check-compiler-boundaries.mjs')),
    false,
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

    const rustResult = await runProcess(
      process.execPath,
      [verifyPath, '--only', 'rust', '--list'],
      { cwd },
    );
    assert.equal(rustResult.code, 0, rustResult.stderr);
    assert.match(rustResult.stdout, /rust:workspace/);
    assert.match(rustResult.stdout, /cargo test --workspace --no-fail-fast/);
    assert.match(rustResult.stdout, /cwd=\./);
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
  return env;
}
