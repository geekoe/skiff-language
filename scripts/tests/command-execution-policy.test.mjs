import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  COMMAND_EXECUTION_LEDGER,
  COMMAND_OWNER_CLASSES,
} from '../lib/command-execution-ledger.mjs';
import {
  assertCommandExecutionPolicy,
  commandExecutionPolicyViolations,
  validateCommandExecutionLedger,
} from '../lib/command-execution-policy.mjs';
import { scanCommandExecutionSource } from '../lib/command-execution-scanner.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('actual production ledger passes with exactly twenty explicit lifecycle owners', async () => {
  await assertCommandExecutionPolicy(root);
  assert.equal(COMMAND_EXECUTION_LEDGER.length, 20);
  assert.equal(new Set(COMMAND_EXECUTION_LEDGER.map((entry) => entry.ownerId)).size, 20);
  assert.equal(
    COMMAND_EXECUTION_LEDGER.filter((entry) => entry.importedSymbol === 'spawn').length,
    14,
  );
  assert.equal(
    COMMAND_EXECUTION_LEDGER.filter((entry) => entry.importedSymbol === 'execFile').length,
    3,
  );
  assert.equal(
    COMMAND_EXECUTION_LEDGER.filter((entry) => entry.importedSymbol === 'execFileSync').length,
    3,
  );
  assert.equal(Object.values(COMMAND_OWNER_CLASSES).includes('migration-pending'), false);
});

test('a valid call-level ledger binds import, alias, marker, owner function, and call count', async () => {
  await withFixture(validSource(), async (fixture) => {
    await assertCommandExecutionPolicy(fixture, { ledger: [validLedger()] });
  });
});

test('unknown imports and extra calls in an already approved file fail closed', async () => {
  await withFixture(validSource(), async (fixture) => {
    const unknown = await commandExecutionPolicyViolations(fixture, { ledger: [] });
    assert.match(unknown.join('\n'), /unregistered child_process import/);

    await write(fixture, 'scripts/tool.mjs', validSource().replace(
      '}\n',
      "  // child-process-owner: copied-owner\n  spawnCommandChild('node', [], {});\n}\n",
    ));
    const extra = await commandExecutionPolicyViolations(fixture, {
      ledger: [validLedger()],
    });
    assert.match(extra.join('\n'), /expected 1 direct call\(s\).*found 2/);
    assert.match(extra.join('\n'), /unused child-process owner marker copied-owner|owner marker mismatch/);
  });
});

test('symbol, alias, marker, and owner function drift are rejected independently', async () => {
  const cases = [
    {
      source: validSource().replace('spawn as spawnCommandChild', 'execFile as spawnCommandChild'),
      ledger: validLedger(),
      expected: /imported symbol mismatch|expected exactly one import spawn/,
    },
    {
      source: validSource().replaceAll('spawnCommandChild', 'renamedChild'),
      ledger: validLedger(),
      expected: /unregistered child_process import|expected exactly one import/,
    },
    {
      source: validSource().replace('child-process-owner: attached-owner', 'child-process-owner: wrong-owner'),
      ledger: validLedger(),
      expected: /owner marker mismatch/,
    },
    {
      source: validSource().replace('function attachedOwner', 'function wrongFunction'),
      ledger: validLedger(),
      expected: /owner function mismatch/,
    },
  ];
  for (const fixtureCase of cases) {
    await withFixture(fixtureCase.source, async (fixture) => {
      const violations = await commandExecutionPolicyViolations(fixture, {
        ledger: [fixtureCase.ledger],
      });
      assert.match(violations.join('\n'), fixtureCase.expected);
    });
  }
});

test('stale and duplicate ledger entries plus illegal owner classes are rejected', async () => {
  const base = validLedger();
  const duplicate = validateCommandExecutionLedger([base, { ...base }]);
  assert.match(duplicate.join('\n'), /duplicate command execution ledger entry/);

  const illegal = validateCommandExecutionLedger([{ ...base, ownerClass: 'whole-file-exception' }]);
  assert.match(illegal.join('\n'), /invalid owner class/);
  const pending = validateCommandExecutionLedger([{ ...base, ownerClass: 'migration-pending' }]);
  assert.match(pending.join('\n'), /invalid owner class/);

  await withFixture('export {};\n', async (fixture) => {
    const stale = await commandExecutionPolicyViolations(fixture, { ledger: [base] });
    assert.match(stale.join('\n'), /expected exactly one import|expected 1 direct call/);
  });

  const missingPath = { ...base, path: 'scripts/missing.mjs', ownerId: 'missing-owner' };
  await withFixture(validSource(), async (fixture) => {
    const stale = await commandExecutionPolicyViolations(fixture, { ledger: [missingPath] });
    assert.match(stale.join('\n'), /stale ledger path/);
  });
});

test('comments, strings, regexes, and template text are ignored while template expressions are scanned', async () => {
  const source = [
    "import { spawn as spawnCommandChild } from 'node:child_process';",
    'function attachedOwner(command, args, options) {',
    "  // spawnCommandChild('fake-comment') require('node:child_process')",
    "  const plain = \"spawnCommandChild('fake-string')\";",
    "  const regex = /spawnCommandChild\\('fake-regex'\\)/;",
    "  const staticTemplate = `require('node:child_process')`;",
    "  const template = `spawnCommandChild('fake-template') ${(() => {",
    '    // child-process-owner: attached-owner',
    '    return spawnCommandChild(command, args, options);',
    '  })()}`;',
    '  return { plain, regex, staticTemplate, template };',
    '}',
    '',
  ].join('\n');
  await withFixture(source, async (fixture) => {
    await assertCommandExecutionPolicy(fixture, { ledger: [validLedger()] });
  });
});

test('tests and generated directories reuse discovery exclusions and do not need ledger entries', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-command-policy-exclusions-'));
  try {
    const bypass = "const child = require('node:child_process'); child.spawn('node');\n";
    await Promise.all([
      write(fixture, 'scripts/tests/ignored.test.mjs', bypass),
      write(fixture, 'scripts/node_modules/pkg/ignored.js', bypass),
      write(fixture, 'scripts/build/ignored.mjs', bypass),
      write(fixture, 'scripts/target/ignored.cjs', bypass),
      write(fixture, 'scripts/var/ignored.js', bypass),
    ]);
    await assertCommandExecutionPolicy(fixture, { ledger: [] });
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('namespace, default, bare, require, dynamic, re-export, and escaped bypasses all fail', async () => {
  const sources = [
    "import * as childProcess from 'node:child_process';\n",
    "import childProcess from 'node:child_process';\n",
    "import 'node:child_process';\n",
    "import { spawn as spawnChild } from 'child_process';\n",
    "const childProcess = require('node:child_process');\n",
    'const childProcess = require(`node:child_process`);\n',
    "const childProcess = await import('node:child_process');\n",
    'const childProcess = await import(`node:child_process`);\n',
    "export { spawn } from 'node:child_process';\n",
    "export * from 'node:child_process';\n",
    String.raw`import { spawn } from "node:\x63hild_process";` + '\n',
    String.raw`const childProcess = require("node:\x63hild_process");` + '\n',
    String.raw`const childProcess = import("node:\x63hild_process");` + '\n',
    String.raw`export { spawn } from "node:\x63hild_process";` + '\n',
    "const childProcess = require(`node:\\x63hild_process`);\n",
    "const childProcess = import(`node:\\x63hild_process`);\n",
  ];
  for (const source of sources) {
    await withFixture(source, async (fixture) => {
      const violations = await commandExecutionPolicyViolations(fixture, { ledger: [] });
      assert.ok(violations.length > 0, source);
      assert.match(violations.join('\n'), /child_process/);
    });
  }
});

test('a semicolonless approved import cannot swallow a later unregistered import and call', async () => {
  const source = [
    "import { spawn as spawnCommandChild } from 'node:child_process'",
    "import { execFile as hiddenChild } from 'node:child_process'",
    'function attachedOwner(command, args, options) {',
    '  // child-process-owner: attached-owner',
    '  return spawnCommandChild(command, args, options)',
    '}',
    'function hiddenOwner(command, args, options) {',
    '  return hiddenChild(command, args, options)',
    '}',
    '',
  ].join('\n');
  const scan = scanCommandExecutionSource(source, 'scripts/tool.mjs');
  assert.deepEqual(scan.imports.map(({ importedSymbol, localAlias }) => ({
    importedSymbol,
    localAlias,
  })), [
    { importedSymbol: 'spawn', localAlias: 'spawnCommandChild' },
    { importedSymbol: 'execFile', localAlias: 'hiddenChild' },
  ]);
  assert.deepEqual(scan.calls.map(({ localAlias }) => localAlias), [
    'spawnCommandChild',
    'hiddenChild',
  ]);

  await withFixture(source, async (fixture) => {
    const violations = await commandExecutionPolicyViolations(fixture, {
      ledger: [validLedger()],
    });
    assert.match(violations.join('\n'), /unregistered child_process import execFile as hiddenChild/);
  });
});

test('imported aliases cannot be assigned, passed, exported, or otherwise referenced', async () => {
  const extraReferences = [
    '  spawnCommandChild = replacement;',
    '  consume(spawnCommandChild);',
    '  return spawnCommandChild;',
  ];
  for (const reference of extraReferences) {
    const source = validSource().replace(
      '  // child-process-owner: attached-owner',
      `${reference}\n  // child-process-owner: attached-owner`,
    );
    await withFixture(source, async (fixture) => {
      const violations = await commandExecutionPolicyViolations(fixture, {
        ledger: [validLedger()],
      });
      assert.match(violations.join('\n'), /unregistered non-call reference/);
    });
  }
});

function validSource() {
  return [
    "import { spawn as spawnCommandChild } from 'node:child_process';",
    'function attachedOwner(command, args, options) {',
    '  // child-process-owner: attached-owner',
    '  return spawnCommandChild(command, args, options);',
    '}',
    '',
  ].join('\n');
}

function validLedger() {
  return {
    path: 'scripts/tool.mjs',
    importedSymbol: 'spawn',
    localAlias: 'spawnCommandChild',
    ownerId: 'attached-owner',
    ownerFunction: 'attachedOwner',
    callCount: 1,
    ownerClass: COMMAND_OWNER_CLASSES.ATTACHED_PRIMITIVE,
    reason: 'fixture canonical attached boundary',
  };
}

async function withFixture(source, callback, { path = 'scripts/tool.mjs' } = {}) {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-command-policy-'));
  try {
    await write(fixture, path, source);
    await callback(fixture);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
}

async function write(rootPath, relativePath, contents) {
  const path = join(rootPath, relativePath);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents);
}
