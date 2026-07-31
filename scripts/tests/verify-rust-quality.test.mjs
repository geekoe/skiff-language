import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { buildVerifyPlan } from '../lib/verify-plan.mjs';
import { VERIFY_SELECTOR_GRAPH } from '../lib/verify-selector-graph.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifyPath = join(root, 'scripts', 'verify.mjs');

test('tests and rust-quality retain separate canonical ownership', async () => {
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions.verify, [
    'tests',
    'rust-quality',
    'type-check',
    'checks',
  ]);
  const tests = await buildVerifyPlan({ root, selectors: ['tests'] });
  assert.ok(tests.tasks.some((task) => task.id === 'skiff-tests:canonical'));
  assert.ok(tests.tasks.some((task) =>
    task.id === 'implementation:compiler:rust'));
  assert.equal(tests.tasks.some((task) => task.kind === 'rust-quality'), false);

  const quality = await buildVerifyPlan({ root, selectors: ['rust-quality'] });
  assert.deepEqual(
    quality.tasks.map(({ id, kind, command, args }) => ({ id, kind, command, args })),
    [
      {
        id: 'rust-quality:format',
        kind: 'rust-quality',
        command: 'cargo',
        args: ['fmt', '--all', '--', '--check'],
      },
      {
        id: 'rust-quality:file-lines',
        kind: 'rust-quality',
        command: 'node',
        args: ['scripts/check-rust-file-lines.mjs'],
      },
    ],
  );
});

test('default verify includes both Rust quality tasks exactly once and no live work', async () => {
  const plan = await buildVerifyPlan({ root });
  for (const id of [
    'skiff-tests:canonical',
    'implementation:foundation:rust',
    'implementation:compiler:rust',
    'implementation:runtime:rust',
    'implementation:test-runner:rust',
    'rust-quality:format',
    'rust-quality:file-lines',
    'checks:artifact-identity',
    'checks:compiler-boundaries',
    'checks:crate-public-api:self-test',
    'checks:crate-public-api:all-configured',
  ]) {
    assert.equal(plan.tasks.filter((task) => task.id === id).length, 1, id);
  }
  assert.equal(plan.tasks.some((task) => task.id.startsWith('live:')), false);
});

test('rust-quality CLI list exposes exactly the format and file-line tasks', async () => {
  const result = await runProcess(process.execPath, [
    verifyPath,
    '--only',
    'rust-quality',
    '--list',
  ]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /tasks: 2/);
  assert.equal((result.stdout.match(/rust-quality:format/g) ?? []).length, 1);
  assert.equal((result.stdout.match(/rust-quality:file-lines/g) ?? []).length, 1);
  assert.match(result.stdout, /cargo fmt --all -- --check/);
  assert.match(result.stdout, /node scripts\/check-rust-file-lines\.mjs/);
});

test('CI runs canonical test domains plus a distinct quality/check scope', async () => {
  const workflow = await readFile(join(root, '.github', 'workflows', 'verify.yml'), 'utf8');
  const commands = [...workflow.matchAll(/^            command: (.+)$/gm)]
    .map((match) => match[1]);
  assert.deepEqual(commands, [
    'node scripts/verify.mjs --only skiff-tests',
    'node scripts/verify.mjs --only implementation-tests',
    'node scripts/verify.mjs --only rust-quality,type-check,checks',
  ]);
  assert.equal(
    commands.filter((command) => command.includes('rust-quality')).length,
    1,
  );
  assert.match(workflow, /--profile minimal --component rustfmt --component clippy/);
  assert.doesNotMatch(workflow, /cargo fmt|cargo clippy|check-rust-file-lines/);

  const installedPackages = [
    ...workflow.matchAll(/pnpm --dir (\S+) install --frozen-lockfile/g),
  ].map((match) => match[1]);
  assert.deepEqual(installedPackages, ['router', 'telemetry', 'scripts', 'vscode']);
  assert.match(workflow, /uses: actions\/checkout@v6\n\s+with:\n\s+persist-credentials: false/);
  assert.match(workflow, /uses: actions\/setup-node@v6/);
  assert.match(workflow, /package-manager-cache: false/);
  assert.doesNotMatch(workflow, /pnpm (?:test|verify)/);
  assert.doesNotMatch(workflow, /cargo test --workspace/);
  assert.doesNotMatch(workflow, /--manifest-path|scripts\/tests/);
  assert.doesNotMatch(
    workflow,
    /runtime-live|db-encrypted-storage-live|compiler-boundaries|loop-risk/,
  );
});

function runProcess(command, args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd: root });
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
