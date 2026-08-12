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
    'tooling',
  ]) {
    assert.ok(PUBLIC_SELECTORS.includes(selector), selector);
  }
  assert.ok(PUBLIC_SELECTORS.includes('router-rust-process-smoke'));
  assert.equal(PUBLIC_SELECTORS.includes('router-rust'), false);
  assert.equal(PUBLIC_SELECTORS.includes('router-ts-tests'), false);
  assert.equal(PUBLIC_SELECTORS.includes('rust'), false);
  assert.equal(PUBLIC_SELECTORS.includes('node'), false);
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions.router, [
    'router-contracts',
    'router-rust-process-smoke',
  ]);
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions.tests, [
    'skiff-tests',
    'implementation-tests',
  ]);
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions['implementation-tests'], [
    ...RUST_IMPLEMENTATION_SUBJECT_SELECTORS,
    'tooling',
  ]);
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions['type-check'], [
    'scripts-syntax',
    'vscode-type-check',
  ]);
  assert.equal(
    Object.values(VERIFY_SELECTOR_GRAPH.expansions).some((children) =>
      children.includes('router-type-check'),
    ),
    false,
  );
});

test('Skiff source tests have one canonical command and remain deduplicated', async () => {
  const focused = await buildVerifyPlan({ root, selectors: ['skiff-tests'] });
  assert.deepEqual(
    focused.tasks.map(({ id, kind, command, args, cwd }) => ({
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
    combined.tasks.filter((task) => task.id === 'skiff-tests:canonical').length,
    1,
  );
  assert.equal(
    combined.tasks.filter((task) => task.id === 'implementation:compiler:rust').length,
    1,
  );
  assert.equal(new Set(combined.tasks.map(({ id }) => id)).size, combined.tasks.length);
});

test('bytecode VM phase 0 gate is independently selectable and excluded from defaults', async () => {
  const focused = await buildVerifyPlan({
    root,
    selectors: ['bytecode-vm-phase-0-gate'],
  });
  assert.deepEqual(
    focused.tasks.map(({ id, kind, command, args, cwd }) => ({
      id,
      kind,
      command,
      args,
      cwd,
    })),
    [{
      id: 'bytecode-vm-phase-0:gate',
      kind: 'implementation:runtime',
      command: 'node',
      args: ['scripts/run-bytecode-vm-phase-0-gate.mjs'],
      cwd: root,
    }],
  );
  for (const selectors of [['tests'], ['verify']]) {
    const plan = await buildVerifyPlan({ root, selectors });
    assert.equal(
      plan.tasks.some((task) => task.id === 'bytecode-vm-phase-0:gate'),
      false,
    );
  }
});

test('implementation tests expand by subject without static or live tasks', async () => {
  const plan = await buildVerifyPlan({ root, selectors: ['implementation-tests'] });
  for (const id of [
    'implementation:foundation:rust',
    'implementation:compiler:rust',
    'implementation:runtime:rust',
    'implementation:test-runner:rust',
    'router:contracts',
    'router-rust:process-smoke',
    'implementation:tooling:dev-sync-fixture',
    'implementation:tooling:vscode-grammar',
  ]) {
    assert.equal(plan.tasks.filter((task) => task.id === id).length, 1, id);
  }
  assert.equal(plan.tasks.some((task) => task.id === 'implementation:router'), false);
  assert.equal(
    plan.tasks.some((task) => task.id.startsWith('router-ts-')),
    false,
  );
  const compilerBoundaryTasks = plan.tasks.filter(
    (task) => task.id === 'checks:compiler-boundaries',
  );
  assert.equal(
    compilerBoundaryTasks.length,
    1,
    'the compiler subject owns its canonical boundary check',
  );
  assert.ok(
    plan.tasks.every(
      (task) =>
        task.kind.startsWith('implementation:') ||
        task.id === 'checks:compiler-boundaries',
    ),
  );
  assert.equal(plan.tasks.some((task) => task.id === 'skiff-tests:canonical'), false);
  assert.equal(plan.tasks.some((task) => task.id.startsWith('rust-quality:')), false);
  assert.equal(plan.tasks.some((task) => task.id.includes(':type-check')), false);
  assert.equal(
    plan.tasks.some(
      (task) => task.id.startsWith('checks:') && task.id !== 'checks:compiler-boundaries',
    ),
    false,
  );
  assert.equal(plan.tasks.some((task) => task.id.startsWith('live:')), false);
});

test('each focused implementation subject is independently usable', async () => {
  for (const selector of [
    ...RUST_IMPLEMENTATION_SUBJECT_SELECTORS,
    'tooling',
  ]) {
    const plan = await buildVerifyPlan({ root, selectors: [selector] });
    assert.ok(plan.tasks.length > 0, selector);
    assert.ok(
      plan.tasks.every(
        (task) =>
          task.kind === `implementation:${selector}` ||
          (selector === 'compiler' && task.id === 'checks:compiler-boundaries'),
      ),
      selector,
    );
    assert.equal(
      plan.tasks.filter((task) => task.id === 'checks:compiler-boundaries').length,
      selector === 'compiler' ? 1 : 0,
      selector,
    );
    if (selector === 'router') {
      assert.deepEqual(
        plan.tasks.map(({ id }) => id).sort(),
        ['router-rust:process-smoke', 'router:contracts'],
      );
    }
  }
});

test('registry transition keeps a single router subject owner with no dual ownership', () => {
  assert.equal(
    RUST_IMPLEMENTATION_SUBJECT_SELECTORS.filter(
      (selector) => selector === 'router',
    ).length,
    1,
  );
  const routerSubjects = RUST_IMPLEMENTATION_SUBJECTS.filter(
    (subject) => subject.selector === 'router',
  );
  assert.equal(routerSubjects.length, 1);
  const routerSubject = routerSubjects[0];
  assert.deepEqual(routerSubject, {
    selector: 'router',
    leafSelector: 'router-contracts',
    taskId: 'router:contracts',
    packages: [
      { workspaceMember: 'router', packageName: 'skiff-router' },
      { workspaceMember: 'task-control', packageName: 'skiff-task-control' },
    ],
  });
  const routerNames = new Set(['router', 'router-contracts', 'router:contracts']);
  assert.equal(
    RUST_IMPLEMENTATION_SUBJECTS.filter((subject) =>
      [subject.selector, subject.leafSelector, subject.taskId].some((name) =>
        routerNames.has(name),
      )).length,
    1,
  );
  assert.equal(
    RUST_IMPLEMENTATION_SUBJECTS.flatMap((subject) =>
      subject.packages.map(({ workspaceMember }) => workspaceMember),
    ).filter((member) => member === 'router').length,
    1,
  );
  assert.equal(
    RUST_IMPLEMENTATION_SUBJECTS.some(
      (subject) =>
        subject.selector === 'router-rust'
        || subject.leafSelector === 'router-rust-contracts'
        || subject.taskId === 'router-rust:contracts',
    ),
    false,
  );
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions.router, [
    'router-contracts',
    'router-rust-process-smoke',
  ]);
  assert.equal(
    Object.values(VERIFY_SELECTOR_GRAPH.expansions).some((children) =>
      children.includes('router-ts-tests'),
    ),
    false,
  );
  assert.deepEqual(VERIFY_SELECTOR_GRAPH.expansions['implementation-tests'], [
    ...RUST_IMPLEMENTATION_SUBJECT_SELECTORS,
    'tooling',
  ]);
});

test('implementation-tests, manual router, and Rust subject expansion deduplicate tasks', async () => {
  const plan = await buildVerifyPlan({
    root,
    selectors: ['implementation-tests', 'router'],
  });
  const ids = plan.tasks.map(({ id }) => id);
  for (const id of [
    'router:contracts',
    'router-rust:process-smoke',
  ]) {
    assert.equal(ids.filter((candidate) => candidate === id).length, 1, id);
  }
  assert.equal(ids.includes('implementation:router'), false);
  assert.equal(ids.some((id) => id.startsWith('router-ts-')), false);
  assert.equal(new Set(ids).size, ids.length);
  const executions = plan.tasks.map((task) => JSON.stringify([
    resolve(task.cwd),
    task.command,
    task.args,
  ]));
  assert.equal(new Set(executions).size, executions.length);
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
    assert.equal(plan.tasks.some((task) => task.id.startsWith('live:')), false);
  }
});

test('verify help explains subject-oriented test domains', async () => {
  const result = await runProcess(process.execPath, [verifyPath, '--help']);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /test domains:/);
  assert.match(result.stdout, /skiff-tests\s+canonical Skiff source suite/);
  assert.match(result.stdout, /implementation-tests\s+all implementation subjects/);
  assert.match(result.stdout, /foundation\s+shared artifact-model/);
  assert.match(result.stdout, /bytecode-vm-phase-0-gate\s+Phase 0 exact-candidate closure gate/);
  assert.match(result.stdout, /SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_COMMIT\s+literal 40-hex commit/);
  assert.match(result.stdout, /SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_TREE\s+literal 40-hex tree/);
  assert.match(result.stdout, /SKIFF_BYTECODE_VM_PHASE0_EVIDENCE_DIR\s+caller-chosen canonical absolute absent path/);
  assert.match(result.stdout, /does not choose them from HEAD or choose a temporary evidence/);
  assert.doesNotMatch(result.stdout, /Phase 1 VCP validation readiness gate/);
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
