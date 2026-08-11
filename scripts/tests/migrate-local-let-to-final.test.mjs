import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';

import {
  assertScannerMatchesRegex,
  collectRegexLocalLetRanges,
  migrateLocalLetToFinal,
  replaceLocalLet,
  scanRustStringLiterals,
  scanSkiffLocalLetRanges,
} from '../migrate-local-let-to-final.mjs';

test('regex and scanner rewrite statement-head let without touching comments or strings', () => {
  const source = [
    '// let hidden = 1',
    '/* let hidden = 2',
    '   let hidden = 3',
    '*/',
    '"let string = 1"',
    'let visible = 1',
    '    let typed: number = 2',
    'var writable = 3',
    'letty = 4',
    '',
  ].join('\n');

  const visible = source.indexOf('let visible');
  const typed = source.indexOf('    let typed') + 4;
  const expected = [
    '// let hidden = 1',
    '/* let hidden = 2',
    '   let hidden = 3',
    '*/',
    '"let string = 1"',
    'final visible = 1',
    '    final typed: number = 2',
    'var writable = 3',
    'letty = 4',
    '',
  ].join('\n');

  assert.deepEqual(scanSkiffLocalLetRanges(source), [[visible, visible + 3], [typed, typed + 3]]);
  assert.deepEqual(assertScannerMatchesRegex(source).regexRanges, [[visible, visible + 3], [typed, typed + 3]]);
  assert.equal(replaceLocalLet(source), expected);
  assert.equal(collectRegexLocalLetRanges(source).length, 3);
});

test('scanner and masked regex fail closed on unterminated lexical states', () => {
  assert.throws(() => assertScannerMatchesRegex('/* let x = 1\n'), /unterminated block comment/);
  assert.throws(() => assertScannerMatchesRegex('"let x = 1\n'), /unterminated/);
});

async function createRepo(t, files) {
  const root = await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  execFileSync('git', ['init', '-q'], { cwd: root });
  execFileSync('git', ['config', 'user.email', 'test@example.com'], { cwd: root });
  execFileSync('git', ['config', 'user.name', 'Migration Test'], { cwd: root });
  for (const [relativePath, contents] of Object.entries(files)) {
    const absolutePath = path.join(root, relativePath);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    await writeFile(absolutePath, contents);
  }
  execFileSync('git', ['add', '-A'], { cwd: root });
  execFileSync('git', ['commit', '-q', '--allow-empty', '-m', 'fixture'], { cwd: root });
  return root;
}

function migrationArgs({ roots, mode, expects, manifestOut }) {
  return [
    '--skiff-root', roots.skiff,
    '--skiff-packages-root', roots.packages,
    '--internals-root', roots.internals,
    mode,
    ...Object.entries(expects).flatMap(([repo, count]) => ['--expect', `${repo}=${count}`]),
    '--manifest-out', manifestOut,
  ];
}

test('write is idempotent, writes the manifest, and preserves unmodified mtime', async (t) => {
  const skiffRoot = await createRepo(t, {
    'main.skiff': 'let a = 1\n// let b = 2\nlet c = 3\n',
  });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');
  const roots = { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot };

  const firstArgs = migrationArgs({
    roots,
    mode: '--write',
    expects: { skiff: 2, 'skiff-packages': 0, internals: 0 },
    manifestOut,
  });
  const first = await migrateLocalLetToFinal(firstArgs);
  assert.equal(first.changedFiles, 1);
  assert.equal(first.counts.skiff, 2);
  assert.equal(await readFile(path.join(skiffRoot, 'main.skiff'), 'utf8'), 'final a = 1\n// let b = 2\nfinal c = 3\n');

  const manifest = JSON.parse(await readFile(manifestOut, 'utf8'));
  assert.equal(manifest.schemaVersion, 'skiff-let-to-final-migration-v1');
  assert.equal(manifest.repos.skiff.head.length, 40);
  assert.equal(manifest.repos.skiff.files[0].path, 'main.skiff');
  assert.equal(manifest.repos.skiff.files[0].replacementCount, 2);
  assert.equal(manifest.repos.skiff.files[0].byteRanges.length, 2);
  assert.match(manifest.repos.skiff.files[0].beforeSha256, /^[0-9a-f]{64}$/);
  assert.match(manifest.repos.skiff.files[0].afterSha256, /^[0-9a-f]{64}$/);

  execFileSync('git', ['add', '-A'], { cwd: skiffRoot });
  execFileSync('git', ['commit', '-q', '-m', 'migrate'], { cwd: skiffRoot });

  const skiffFile = path.join(skiffRoot, 'main.skiff');
  const beforeSecond = await stat(skiffFile);
  const secondArgs = migrationArgs({
    roots,
    mode: '--write',
    expects: { skiff: 0, 'skiff-packages': 0, internals: 0 },
    manifestOut,
  });
  const second = await migrateLocalLetToFinal(secondArgs);
  assert.equal(second.changedFiles, 0);
  const afterSecond = await stat(skiffFile);
  assert.equal(afterSecond.mtimeMs, beforeSecond.mtimeMs);
  assert.equal(await readFile(skiffFile, 'utf8'), 'final a = 1\n// let b = 2\nfinal c = 3\n');
});

test('write preserves UTF-8 BOM, CRLF, and a missing final newline', async (t) => {
  const bom = Buffer.from([0xef, 0xbb, 0xbf]);
  const skiffRoot = await createRepo(t, {
    'main.skiff': Buffer.concat([bom, Buffer.from('let x = 1\r\nlet y = 2')]),
  });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  await migrateLocalLetToFinal(migrationArgs({
    roots: { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot },
    mode: '--write',
    expects: { skiff: 2, 'skiff-packages': 0, internals: 0 },
    manifestOut,
  }));

  const output = await readFile(path.join(skiffRoot, 'main.skiff'));
  assert.deepEqual([...output.subarray(0, 3)], [0xef, 0xbb, 0xbf]);
  assert.equal(output.subarray(3).toString('utf8'), 'final x = 1\r\nfinal y = 2');
});

test('normal migration rejects dirty tracked .skiff overlap', async (t) => {
  const skiffRoot = await createRepo(t, { 'main.skiff': 'let x = 1\n' });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  await writeFile(path.join(skiffRoot, 'main.skiff'), 'let x = 2\n');
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  await assert.rejects(
    migrateLocalLetToFinal(migrationArgs({
      roots: { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot },
      mode: '--check',
      expects: { skiff: 1, 'skiff-packages': 0, internals: 0 },
      manifestOut,
    })),
    /dirty overlap/,
  );
});

test('normal migration rejects count drift and requires --expect', async (t) => {
  const skiffRoot = await createRepo(t, { 'main.skiff': 'let x = 1\n' });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  await assert.rejects(
    migrateLocalLetToFinal(migrationArgs({
      roots: { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot },
      mode: '--check',
      expects: { skiff: 99, 'skiff-packages': 0, internals: 0 },
      manifestOut,
    })),
    /count drift/,
  );
});

test('normal migration never rewrites Rust files', async (t) => {
  const rustSource = 'fn main() { let host = 1; let text = "    let x = 1"; }\n';
  const skiffRoot = await createRepo(t, {
    'main.skiff': 'let x = 1\n',
    'fixture.rs': rustSource,
  });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  await migrateLocalLetToFinal(migrationArgs({
    roots: { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot },
    mode: '--write',
    expects: { skiff: 1, 'skiff-packages': 0, internals: 0 },
    manifestOut,
  }));

  assert.equal(await readFile(path.join(skiffRoot, 'fixture.rs'), 'utf8'), rustSource);
  assert.equal(await readFile(path.join(skiffRoot, 'main.skiff'), 'utf8'), 'final x = 1\n');
});

test('Rust lexical scanner maps decoded Skiff tokens back to original string bytes', () => {
  const rust = Buffer.from([
    'fn main() {',
    '  let host = 1;',
    '  let raw = r#"    let x = 1',
    '    let y = 2"#;',
    '  let escaped = "    let z = 1";',
    '}',
    '',
  ].join('\n'));
  const literals = scanRustStringLiterals(rust);
  assert.equal(literals.length, 2);
  const raw = literals.find((literal) => literal.rawDelimiter === 'r#"');
  const escaped = literals.find((literal) => literal.rawDelimiter === '"');
  assert.equal(scanSkiffLocalLetRanges(raw.decodedText).length, 2);
  assert.equal(scanSkiffLocalLetRanges(escaped.decodedText).length, 1);
});

test('embedded fixture check and write consume the reviewed manifest', async (t) => {
  const skiffRoot = await createRepo(t, {
    'fixture.rs': Buffer.from([
      'fn f() {',
      '  let host = 1;',
      '  let raw = r#"    let x = 1',
      '    let y = 2"#;',
      '  let escaped = "    let z = 1";',
      '}',
      '',
    ].join('\n')),
    'vscode/scripts/test-grammar.mjs': 'const sampleLines = ["    let active = true"];\n',
  });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'embedded.json');
  const roots = { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot };

  const checked = await migrateLocalLetToFinal([
    '--skiff-root', roots.skiff,
    '--skiff-packages-root', roots.packages,
    '--internals-root', roots.internals,
    '--embedded-fixtures-check',
    '--manifest-out', manifestOut,
  ]);
  assert.equal(checked.total, 4);
  assert.equal(checked.counts.skiff, 4);

  await migrateLocalLetToFinal([
    '--skiff-root', roots.skiff,
    '--skiff-packages-root', roots.packages,
    '--internals-root', roots.internals,
    '--embedded-fixtures-write',
    '--manifest-out', manifestOut,
  ]);

  const rust = await readFile(path.join(skiffRoot, 'fixture.rs'), 'utf8');
  assert.match(rust, /let host = 1;/);
  assert.match(rust, /final x = 1/);
  assert.match(rust, /final y = 2/);
  assert.match(rust, /final z = 1/);
  assert.doesNotMatch(rust, /let raw = r#"    let/);
  assert.doesNotMatch(rust, /let escaped = "    let/);

  const grammar = await readFile(path.join(skiffRoot, 'vscode/scripts/test-grammar.mjs'), 'utf8');
  assert.match(grammar, /final active = true/);
  assert.doesNotMatch(grammar, /let active = true/);
});

test('check mode inventories without writing source files', async (t) => {
  const skiffRoot = await createRepo(t, { 'main.skiff': 'let x = 1\n' });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  const result = await migrateLocalLetToFinal(migrationArgs({
    roots: { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot },
    mode: '--check',
    expects: { skiff: 1, 'skiff-packages': 0, internals: 0 },
    manifestOut,
  }));

  assert.equal(result.changedFiles, 0);
  assert.equal(await readFile(path.join(skiffRoot, 'main.skiff'), 'utf8'), 'let x = 1\n');
  const manifest = JSON.parse(await readFile(manifestOut, 'utf8'));
  assert.equal(manifest.repos.skiff.files[0].path, 'main.skiff');
  assert.equal(manifest.repos.skiff.files[0].replacementCount, 1);
});

test('normal migration rejects tracked symlink files', async (t) => {
  const skiffRoot = await createRepo(t, { 'target.txt': 'target' });
  await symlink('target.txt', path.join(skiffRoot, 'link.skiff'));
  execFileSync('git', ['add', 'link.skiff'], { cwd: skiffRoot });
  execFileSync('git', ['commit', '-q', '-m', 'add symlink'], { cwd: skiffRoot });
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  await assert.rejects(
    migrateLocalLetToFinal(migrationArgs({
      roots: { skiff: skiffRoot, packages: packagesRoot, internals: internalsRoot },
      mode: '--check',
      expects: { skiff: 0, 'skiff-packages': 0, internals: 0 },
      manifestOut,
    })),
    /must not be a symlink/,
  );
});

test('normal migration rejects a root that is not the git top-level', async (t) => {
  const skiffRoot = await createRepo(t, { 'nested/.keep': '' });
  const nestedRoot = path.join(skiffRoot, 'nested');
  const packagesRoot = await createRepo(t, {});
  const internalsRoot = await createRepo(t, {});
  const manifestOut = path.join(await mkdtemp(path.join(tmpdir(), 'skiff-final-migration-meta-')), 'inventory.json');

  await assert.rejects(
    migrateLocalLetToFinal([
      '--skiff-root', nestedRoot,
      '--skiff-packages-root', packagesRoot,
      '--internals-root', internalsRoot,
      '--check',
      '--expect', 'skiff=0',
      '--expect', 'skiff-packages=0',
      '--expect', 'internals=0',
      '--manifest-out', manifestOut,
    ]),
    /not the git top-level/,
  );
});
