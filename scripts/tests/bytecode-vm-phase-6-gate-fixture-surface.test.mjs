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
