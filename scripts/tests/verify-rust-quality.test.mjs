import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { RUST_CLIPPY_BASELINE_ARGS } from '../lib/rust-clippy-baseline-check.mjs';
import { buildVerifyPlan } from '../lib/verify-plan.mjs';
import { VERIFY_SELECTOR_GRAPH } from '../lib/verify-selector-graph.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifyPath = join(root, 'scripts', 'verify.mjs');

test('rust and rust-quality retain separate canonical ownership', async () => {
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions.verify, [
    'rust',
    'rust-quality',
    'node',
    'checks',
  ]);
  const rust = await buildVerifyPlan({ root, selectors: ['rust'] });
  assert.deepEqual(
    rust.phases.map(({ id, command, args }) => ({ id, command, args })),
    [{
      id: 'rust:workspace',
      command: 'cargo',
      args: ['test', '--workspace', '--no-fail-fast'],
    }],
  );

  const quality = await buildVerifyPlan({ root, selectors: ['rust-quality'] });
  assert.deepEqual(
    quality.phases.map(({ id, kind, command, args }) => ({ id, kind, command, args })),
    [
      {
        id: 'rust-quality:format',
        kind: 'rust-quality',
        command: 'cargo',
        args: ['fmt', '--all', '--', '--check'],
      },
      {
        id: 'rust-quality:clippy-baseline',
        kind: 'rust-quality',
        command: 'node',
        args: ['scripts/check-rust-clippy-baseline.mjs'],
      },
    ],
  );
  assert.deepEqual(RUST_CLIPPY_BASELINE_ARGS, [
    'clippy',
    '--workspace',
    '--all-targets',
    '--no-deps',
    '--message-format=json',
  ]);
});

test('default verify includes both Rust quality phases exactly once and no live work', async () => {
  const plan = await buildVerifyPlan({ root });
  for (const id of [
    'rust:workspace',
    'rust-quality:format',
    'rust-quality:clippy-baseline',
    'checks:artifact-identity',
    'checks:compiler-boundaries',
    'checks:crate-public-api:self-test',
    'checks:crate-public-api:all-configured',
  ]) {
    assert.equal(plan.phases.filter((phase) => phase.id === id).length, 1, id);
  }
  assert.equal(plan.phases.some((phase) => phase.id.startsWith('live:')), false);
});

test('rust-quality CLI list exposes exactly the format and baseline-aware Clippy phases', async () => {
  const result = await runProcess(process.execPath, [
    verifyPath,
    '--only',
    'rust-quality',
    '--list',
  ]);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /phases: 2/);
  assert.equal((result.stdout.match(/rust-quality:format/g) ?? []).length, 1);
  assert.equal((result.stdout.match(/rust-quality:clippy-baseline/g) ?? []).length, 1);
  assert.match(result.stdout, /cargo fmt --all -- --check/);
  assert.match(result.stdout, /node scripts\/check-rust-clippy-baseline\.mjs/);
});

test('CI runs a distinct canonical Rust Quality scope and installs required Rust components', async () => {
  const workflow = await readFile(join(root, '.github', 'workflows', 'verify.yml'), 'utf8');
  const commands = [...workflow.matchAll(/^            command: (.+)$/gm)]
    .map((match) => match[1]);
  assert.deepEqual(commands, [
    'cargo test --workspace --no-fail-fast',
    'node scripts/verify.mjs --only rust-quality',
    'pnpm test',
    'node scripts/verify.mjs --only checks',
  ]);
  assert.equal(
    commands.filter((command) => command === 'node scripts/verify.mjs --only rust-quality').length,
    1,
  );
  assert.match(workflow, /--profile minimal --component rustfmt --component clippy/);
  assert.doesNotMatch(workflow, /cargo fmt|cargo clippy|check-rust-clippy-baseline/);

  const installedPackages = [
    ...workflow.matchAll(/pnpm --dir (\S+) install --frozen-lockfile/g),
  ].map((match) => match[1]);
  assert.deepEqual(installedPackages, ['router', 'telemetry', 'scripts', 'vscode']);
  assert.match(workflow, /uses: actions\/checkout@v6\n\s+with:\n\s+persist-credentials: false/);
  assert.match(workflow, /uses: actions\/setup-node@v6/);
  assert.match(workflow, /package-manager-cache: false/);
  assert.doesNotMatch(workflow, /pnpm verify/);
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
