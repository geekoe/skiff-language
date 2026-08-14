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
  const dropLeftOffset = source.indexOf('function dropLeft(');
  assert.notEqual(dropLeftOffset, -1);
  const runSource = source.slice(0, dropLeftOffset);
  const dropLeftSource = source.slice(dropLeftOffset);
  assert.match(runSource, /emit\(\{ tag: "start", status: 207, headers: headers\(\) \}\)/);
  assert.equal((runSource.match(/emit\(\{ tag: "chunk", value:/g) ?? []).length, 6);
  assert.equal((runSource.match(/emit\(\{ tag: "end" \}\)/g) ?? []).length, 1);
  assert.match(dropLeftSource, /emit\(\{ tag: "start", status: 208, headers: headers\(\) \}\)/);
  assert.equal((dropLeftSource.match(/emit\(\{ tag: "chunk", value:/g) ?? []).length, 2);
  assert.equal((dropLeftSource.match(/emit\(\{ tag: "end" \}\)/g) ?? []).length, 1);
  assert.doesNotMatch(source, /std\.http\.stream(?:Start|Chunk|End)\(/);
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
