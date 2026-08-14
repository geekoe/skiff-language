import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const fixture = (name, file = 'main.skiff') => fileURLToPath(new URL(
  `../../runtime/host/tests/fixtures/bytecode-vm-phase-5/${name}/${file}`,
  import.meta.url,
));

test('positive fixture is the rawHttp serverStream carrier consumed by Rust proof', async () => {
  const [source, http] = await Promise.all([
    readFile(fixture('positive'), 'utf8'),
    readFile(fixture('positive', 'http.yml'), 'utf8'),
  ]);
  assert.match(http, /kind: rawHttp/);
  assert.match(source, /request\.body\.toUtf8String\(\)/);
  assert.equal((source.match(/std\.http\.request\(/g) ?? []).length, 1);
  assert.equal((source.match(/std\.http\.stream\(/g) ?? []).length, 4);
  assert.match(source, /for chunk in left\.body/);
  assert.match(source, /for chunk in right\.body/);
  assert.match(source, /std\.http\.streamStart\(207/);
  assert.match(source, /std\.http\.streamEnd\(\)/);
  assert.doesNotMatch(source, /phase5\.test|127\.0\.0\.1:0/);
});

test('negative fixture set pins SSE, same-context Date.now, and illegal Stream placement', async () => {
  const [sse, date, placement] = await Promise.all([
    readFile(fixture('unsupported-sse'), 'utf8'),
    readFile(fixture('unsupported-date-now'), 'utf8'),
    readFile(fixture('illegal-stream-placement'), 'utf8'),
  ]);
  assert.match(sse, /std\.http\.sse\(/);
  assert.match(date, /Date\.now\(\)/);
  assert.match(placement, /function leak\(\) -> Stream<string>/);
});
