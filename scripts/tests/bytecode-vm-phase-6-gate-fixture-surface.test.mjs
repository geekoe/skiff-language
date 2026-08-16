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
const hostChainSource = fileURLToPath(new URL(
  '../../runtime/host/tests/bytecode_vm_phase_6/host_chain.rs',
  import.meta.url,
));
const recoverableCodecSource = fileURLToPath(new URL(
  '../../runtime/host/tests/bytecode_vm_phase_6/recoverable_codec.rs',
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
  const callbackProvider = await readFile(fixture('callback-provider'), 'utf8');
  const recoverable = await sources.recoverable;
  const db = await sources.db;
  const task = await sources.task;
  const actor = await sources.actor;
  assert.match(service, /payments\/echo\(/);
  assert.match(interfaceFixture, / as [A-Za-z0-9_./]+/);
  assert.match(callback, /payments\.Handler/);
  assert.match(callback, /payments\/invoke/);
  assert.doesNotMatch(callback, /fn\(/);
  assert.match(callbackProvider, /function invoke\(handler: any Handler/);
  assert.match(recoverable, /db transaction|db object/);
  assert.match(db, /db transaction/);
  assert.match(task, /dispatch /);
  assert.match(actor, /std\.actor\.get</);
  for (const source of [service, interfaceFixture, callback, recoverable, db, task, actor]) {
    assert.doesNotMatch(source, /phase6\.test|SKIFF_BYTECODE_VM_PHASE6/);
  }
});

test('task and actor positive fixtures use the admitted unary scalar surface', async () => {
  const task = await readFile(fixture('task-positive'), 'utf8');
  const taskHttp = await readFile(fixture('task-positive', 'http.yml'), 'utf8');
  const actor = await readFile(fixture('actor-positive'), 'utf8');
  const actorHttp = await readFile(fixture('actor-positive', 'http.yml'), 'utf8');
  assert.match(task, /dispatch work\(seed\)/);
  assert.doesNotMatch(task, /TaskRef|db object|rawHttp|Stream</);
  assert.match(taskHttp, /typedJson/);
  assert.doesNotMatch(taskHttp, /rawHttp/);
  assert.match(actor, /std\.actor\.get<Counter>/);
  assert.doesNotMatch(actor, /\.toString\(\)|rawHttp|Stream</);
  assert.match(actorHttp, /typedJson/);
  assert.doesNotMatch(actorHttp, /rawHttp/);
});

test('J2 focused fixtures use unary owner-internal production paths', async () => {
  const recoverable = await readFile(fixture('recoverable-positive'), 'utf8');
  const db = await readFile(fixture('db-positive'), 'utf8');
  const recoverableHttp = await readFile(
    fixture('recoverable-positive', 'http.yml'),
    'utf8',
  );
  const dbHttp = await readFile(fixture('db-positive', 'http.yml'), 'utf8');
  const localHttp = await readFile(
    fixture('interface-local-success', 'http.yml'),
    'utf8',
  );
  const hostChain = await readFile(hostChainSource, 'utf8');
  assert.match(recoverable, /db object/);
  assert.doesNotMatch(recoverable, /dispatch |rawHttp/);
  assert.match(recoverableHttp, /typedJson/);
  assert.doesNotMatch(recoverableHttp, /rawHttp/);
  assert.match(db, /db object/);
  assert.doesNotMatch(db, /rawHttp/);
  assert.match(dbHttp, /typedJson/);
  assert.doesNotMatch(dbHttp, /rawHttp/);
  assert.match(localHttp, /typedJson/);
  assert.doesNotMatch(localHttp, /rawHttp/);
  assert.match(localHttp, /path: \/phase-6\/interface\n/);
  assert.match(localHttp, /path: \/phase-6\/interface-local\n/);
  assert.match(
    hostChain,
    /Capability::Service\s*\|\s*Capability::InterfaceLocal\s*\|\s*Capability::InterfaceRemote\s*\|\s*Capability::Recoverable\s*\|\s*Capability::Db\s*\|\s*Capability::Task\s*\|\s*Capability::Callback\s*\|\s*Capability::Actor/,
  );
  assert.match(hostChain, /let mode = if unary_json \{ "unary" \}/);
  assert.match(hostChain, /let body = if unary_json \{\s*b"7"\.as_slice\(\)/);
});

test('every interface-local focused fixture has canonical authoring files', async () => {
  for (const directory of [
    'interface-local-success',
    'interface-local-throw',
    'interface-local-pending',
    'interface-local-bad-slot',
    'interface-local-bad-carrier',
    'interface-local-bad-signature',
  ]) {
    for (const file of ['main.skiff', 'http.yml', 'api.yml', 'service.yml', 'package.yml']) {
      const source = await readFile(fixture(directory, file), 'utf8');
      assert.equal(source.length > 0, true, `${directory}/${file} must not be empty`);
    }
  }
});

test('Rust matrices expose every registered prefix with the exact expected red count', async () => {
  const host = await readFile(hostRustSource, 'utf8');
  const recoverableCodec = await readFile(recoverableCodecSource, 'utf8');
  const router = await readFile(routerRustSource, 'utf8');
  const expectedStageTests = new Map([
    ['service_', 10],
    ['interface_local_', 12],
    ['interface_remote_', 10],
    ['callback_', 11],
    ['recoverable_', 10],
    ['db_', 10],
    ['task_', 11],
    ['actor_', 11],
  ]);
  for (const [prefix, count] of expectedStageTests) {
    const stagePattern = new RegExp(`\\b${escapeRegExp(prefix)}s[1-6]\\b`, 'g');
    const focusedPattern = new RegExp(
      `\\bfn\\s+${escapeRegExp(prefix)}(?!s[1-6]\\b)[a-z0-9_]+\\s*\\(`,
      'g',
    );
    let actual = [...host.matchAll(stagePattern)].length
      + [...host.matchAll(focusedPattern)].length;
    if (prefix === 'recoverable_') {
      actual += [...recoverableCodec.matchAll(
        /\bfn\s+recoverable_[a-z0-9_]+\s*\(/g,
      )].length;
    }
    if (prefix === 'task_') {
      actual -= [...host.matchAll(/\bfn\s+task_submit_request\s*\(/g)].length;
    }
    assert.equal(actual, count, `host prefix ${prefix} test count`);
  }
  for (const [prefix, count] of [
    ['task_', 8],
    ['actor_', 7],
  ]) {
    const stagePattern = new RegExp(`\\b${escapeRegExp(prefix)}s[1-6]\\b`, 'g');
    const focusedPattern = new RegExp(
      `\\bfn\\s+${escapeRegExp(prefix)}(?!s[1-6]\\b)[a-z0-9_]+\\s*\\(`,
      'g',
    );
    let actual = [...router.matchAll(stagePattern)].length
      + [...router.matchAll(focusedPattern)].length;
    if (prefix === 'task_') {
      actual -= [...router.matchAll(/\bfn\s+task_record\s*\(/g)].length;
    }
    assert.equal(actual, count, `router prefix ${prefix} test count`);
  }
  assert.equal([...host.matchAll(/\bcontainment_[a-z0-9_]+\b/g)].length, 8,
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
