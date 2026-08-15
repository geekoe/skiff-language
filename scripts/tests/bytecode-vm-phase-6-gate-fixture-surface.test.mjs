import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const fixture = (name, file = 'main.skiff') => fileURLToPath(new URL(
  `../../runtime/host/tests/fixtures/bytecode-vm-phase-6/${name}/${file}`,
  import.meta.url,
));

const CAPABILITIES = [
  'service',
  'interface',
  'callback',
  'recoverable',
  'db',
  'task',
  'actor',
];

const hostRustSource = fileURLToPath(new URL(
  '../../runtime/host/tests/bytecode_vm_phase_6.rs',
  import.meta.url,
));
const routerRustSource = fileURLToPath(new URL(
  '../../router/tests/bytecode_vm_phase_6.rs',
  import.meta.url,
));

test('every Phase 6 capability has positive and negative source fixtures', async () => {
  for (const capability of CAPABILITIES) {
    for (const polarity of ['positive', 'negative']) {
      const source = await readFile(fixture(`${capability}-${polarity}`), 'utf8');
      assert.equal(source.length > 0, true, `${capability}-${polarity} must not be empty`);
    }
  }
  for (const polarity of ['positive', 'negative']) {
    await readFile(fixture(`containment-${polarity}`), 'utf8');
  }
});

test('positive fixtures drive real Phase 6 source semantics, not test seams', async () => {
  const entries = await Promise.all(
    CAPABILITIES.map(async (capability) => [
      capability,
      await readFile(fixture(`${capability}-positive`), 'utf8'),
    ]),
  );
  const sources = Object.fromEntries(entries);
  const service = await sources.service;
  const interfaceFixture = await sources.interface;
  const callback = await sources.callback;
  const recoverable = await sources.recoverable;
  const db = await sources.db;
  const task = await sources.task;
  const actor = await sources.actor;
  assert.match(service, /payments\/echo\(/);
  assert.match(interfaceFixture, / as [A-Za-z0-9_./]+/);
  assert.match(callback, /fn\(/);
  assert.match(recoverable, /dispatch |std\.task\./);
  assert.match(db, /db transaction/);
  assert.match(task, /dispatch /);
  assert.match(actor, /std\.actor\.get</);
  for (const source of [service, interfaceFixture, callback, recoverable, db, task, actor]) {
    assert.doesNotMatch(source, /phase6\.test|SKIFF_BYTECODE_VM_PHASE6/);
  }
});

test('Rust matrices expose every registered prefix with the exact expected red count', async () => {
  const host = await readFile(hostRustSource, 'utf8');
  const router = await readFile(routerRustSource, 'utf8');
  const expectedStageTests = new Map([
    ['service_', 6],
    ['interface_local_', 6],
    ['interface_remote_', 6],
    ['callback_', 6],
    ['recoverable_', 6],
    ['db_', 6],
    ['task_', 6],
    ['actor_', 6],
  ]);
  for (const [prefix, count] of expectedStageTests) {
    const pattern = new RegExp(`\\b${escapeRegExp(prefix)}s[1-6]\\b`, 'g');
    const actual = [...host.matchAll(pattern)].length;
    assert.equal(actual, count, `host prefix ${prefix} test count`);
  }
  for (const [prefix, count] of [
    ['task_', 6],
    ['actor_', 6],
  ]) {
    const pattern = new RegExp(`\\b${escapeRegExp(prefix)}s[1-6]\\b`, 'g');
    const actual = [...router.matchAll(pattern)].length;
    assert.equal(actual, count, `router prefix ${prefix} test count`);
  }
  assert.equal([...host.matchAll(/\bcontainment_[a-z0-9_]+\b/g)].length, 2,
    'host containment test count');
  assert.equal([...host.matchAll(/\bphase_6_kernel_[a-z0-9_]+\b/g)].length, 6,
    'host kernel-focused test count');
  for (const file of [hostRustSource, routerRustSource]) {
    const source = await readFile(file, 'utf8');
    assert.doesNotMatch(source, /#\s*\[(?:ignore|skip)\s*\]/, `${file} ignore/skip`);
    assert.doesNotMatch(source, /\btodo!\s*\(/, `${file} todo`);
    assert.doesNotMatch(source, /\bunimplemented!\s*\(/, `${file} unimplemented`);
  }
});

test('every fixture directory contains the canonical authoring and HTTP surface files', async () => {
  for (const capability of [...CAPABILITIES, 'containment']) {
    for (const polarity of ['positive', 'negative']) {
      const base = `${capability}-${polarity}`;
      for (const file of ['main.skiff', 'http.yml', 'api.yml', 'service.yml', 'package.yml']) {
        const source = await readFile(fixture(base, file), 'utf8');
        assert.equal(source.length > 0, true, `${base}/${file} must not be empty`);
      }
    }
  }
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
