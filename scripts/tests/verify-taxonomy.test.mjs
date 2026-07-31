import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { buildVerifyPlan, PUBLIC_SELECTORS } from '../lib/verify-plan.mjs';
import { VERIFY_SELECTOR_GRAPH } from '../lib/verify-selector-graph.mjs';
import {
  RUST_IMPLEMENTATION_SUBJECTS,
  RUST_IMPLEMENTATION_SUBJECT_SELECTORS,
  assertRustWorkspaceOwnership,
} from '../lib/verify-rust-subjects.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const verifyPath = join(root, 'scripts', 'verify.mjs');

test('ordinary selectors expose the two test domains and implementation subjects', () => {
  for (const selector of [
    'verify',
    'tests',
    'skiff-tests',
    'implementation-tests',
    'foundation',
    'compiler',
    'runtime',
    'test-runner',
    'router',
    'telemetry',
    'tooling',
  ]) {
    assert.ok(PUBLIC_SELECTORS.includes(selector), selector);
  }
  assert.equal(PUBLIC_SELECTORS.includes('rust'), false);
  assert.equal(PUBLIC_SELECTORS.includes('node'), false);
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions.tests, [
    'skiff-tests',
    'implementation-tests',
  ]);
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions['implementation-tests'], [
    ...RUST_IMPLEMENTATION_SUBJECT_SELECTORS,
    'router',
    'telemetry',
    'tooling',
  ]);
});

test('Skiff source tests have one canonical command and remain deduplicated', async () => {
  const focused = await buildVerifyPlan({ root, selectors: ['skiff-tests'] });
  assert.deepEqual(
    focused.phases.map(({ id, kind, command, args, cwd }) => ({
      id,
      kind,
      command,
      args,
      cwd,
    })),
    [{
      id: 'skiff-tests:canonical',
      kind: 'skiff-tests',
      command: 'node',
      args: ['scripts/run-skiff-tests.mjs'],
      cwd: root,
    }],
  );

  const combined = await buildVerifyPlan({
    root,
    selectors: ['tests', 'skiff-tests', 'implementation-tests', 'compiler'],
  });
  assert.equal(
    combined.phases.filter((phase) => phase.id === 'skiff-tests:canonical').length,
    1,
  );
  assert.equal(
    combined.phases.filter((phase) => phase.id === 'implementation:compiler:rust').length,
    1,
  );
  assert.equal(new Set(combined.phases.map(({ id }) => id)).size, combined.phases.length);
});

test('implementation tests expand by subject without static or live phases', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['implementation-tests'] });
  for (const id of [
    'implementation:foundation:rust',
    'implementation:compiler:rust',
    'implementation:runtime:rust',
    'implementation:test-runner:rust',
    'implementation:router',
    'implementation:telemetry',
    'implementation:tooling:dev-sync-fixture',
    'implementation:tooling:vscode-grammar',
  ]) {
    assert.equal(plan.phases.filter((phase) => phase.id === id).length, 1, id);
  }
  const compilerBoundaryPhases = plan.phases.filter(
    (phase) => phase.id === 'checks:compiler-boundaries',
  );
  assert.equal(
    compilerBoundaryPhases.length,
    1,
    'the compiler subject owns its canonical boundary check',
  );
  assert.ok(
    plan.phases.every(
      (phase) =>
        phase.kind.startsWith('implementation:') ||
        phase.id === 'checks:compiler-boundaries',
    ),
  );
  assert.equal(plan.phases.some((phase) => phase.id === 'skiff-tests:canonical'), false);
  assert.equal(plan.phases.some((phase) => phase.id.startsWith('rust-quality:')), false);
  assert.equal(plan.phases.some((phase) => phase.id.includes(':type-check')), false);
  assert.equal(
    plan.phases.some(
      (phase) => phase.id.startsWith('checks:') && phase.id !== 'checks:compiler-boundaries',
    ),
    false,
  );
  assert.equal(plan.phases.some((phase) => phase.id.startsWith('live:')), false);
});

test('each focused implementation subject is independently usable', async () => {
  for (const selector of [
    ...RUST_IMPLEMENTATION_SUBJECT_SELECTORS,
    'router',
    'telemetry',
    'tooling',
  ]) {
    const plan = await buildVerifyPlan({ root, selectors: [selector] });
    assert.ok(plan.phases.length > 0, selector);
    assert.ok(
      plan.phases.every(
        (phase) =>
          phase.kind === `implementation:${selector}` ||
          (selector === 'compiler' && phase.id === 'checks:compiler-boundaries'),
      ),
      selector,
    );
    assert.equal(
      plan.phases.filter((phase) => phase.id === 'checks:compiler-boundaries').length,
      selector === 'compiler' ? 1 : 0,
      selector,
    );
  }
});

test('every current Rust workspace package belongs to exactly one subject', async () => {
  const cargoToml = await readFile(join(root, 'Cargo.toml'), 'utf8');
  const workspaceMembers = parseWorkspaceMembers(cargoToml);
  const ownership = assertRustWorkspaceOwnership(workspaceMembers);
  assert.equal(ownership.size, workspaceMembers.length);

  await Promise.all(RUST_IMPLEMENTATION_SUBJECTS.flatMap((subject) =>
    subject.packages.map(async ({ workspaceMember, packageName }) => {
      assert.equal(ownership.get(workspaceMember), subject.selector);
      const manifest = await readFile(
        join(root, workspaceMember, 'Cargo.toml'),
        'utf8',
      );
      assert.equal(parsePackageName(manifest), packageName, workspaceMember);
    })));

  assert.throws(
    () => assertRustWorkspaceOwnership([...workspaceMembers, 'new-unowned-crate']),
    /unowned Rust workspace member\(s\): new-unowned-crate/,
  );
});

test('default tests and verify plans exclude explicit live/manual selectors', async () => {
  for (const selectors of [['tests'], ['verify']]) {
    const plan = await buildVerifyPlan({ root, selectors });
    assert.equal(plan.phases.some((phase) => phase.id.startsWith('live:')), false);
  }
});

test('verify help explains subject-oriented test domains', async () => {
  const result = await runProcess(process.execPath, [verifyPath, '--help']);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /test domains:/);
  assert.match(result.stdout, /skiff-tests\s+canonical Skiff source suite/);
  assert.match(result.stdout, /implementation-tests\s+all implementation subjects/);
  assert.match(result.stdout, /foundation\s+shared artifact-model/);
  assert.doesNotMatch(result.stdout, /cargo test --workspace|Node\/TypeScript plan/);
});

function parseWorkspaceMembers(cargoToml) {
  const members = cargoToml.match(/\bmembers\s*=\s*\[([\s\S]*?)\]/)?.[1];
  assert.ok(members, 'Cargo.toml must declare workspace members');
  return [...members.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

function parsePackageName(cargoToml) {
  const name = cargoToml.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
  assert.ok(name, 'package Cargo.toml must declare a name');
  return name;
}

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
