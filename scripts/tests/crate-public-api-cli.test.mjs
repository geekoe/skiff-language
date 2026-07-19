import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  parseCratePublicApiArgs,
  renderCratePublicApiUsage,
  runCratePublicApiCli,
} from '../lib/crate-public-api-cli.mjs';
import { MANAGED_CRATE_HELP_NAMES } from '../lib/crate-public-api-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const entry = join(root, 'scripts', 'check-crate-public-api.mjs');
const cli = join(root, 'scripts', 'lib', 'crate-public-api-cli.mjs');

test('entry and CLI imports have no output, Cargo, or exit-code side effects', () => {
  for (const path of [entry, cli]) {
    const imported = spawnSync(process.execPath, [
      '--input-type=module',
      '--eval',
      [
        'const before = process.exitCode;',
        `await import(${JSON.stringify(pathToFileURL(path).href)});`,
        "if (process.exitCode !== before) throw new Error('exit code changed');",
        "process.stdout.write('IMPORT_OK');",
      ].join('\n'),
    ], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, PATH: '' },
    });
    assert.equal(imported.status, 0, imported.stderr);
    assert.equal(imported.stdout, 'IMPORT_OK');
    assert.equal(imported.stderr, '');
  }
});

test('help text is policy-derived while remaining byte-for-byte stable', async () => {
  const output = sink();
  const exitCode = await runCratePublicApiCli({
    argv: ['--help'],
    root,
    stderr: sink(),
    stdout: output,
  });
  assert.equal(exitCode, 0);
  assert.equal(output.text, `${renderCratePublicApiUsage()}\n`);
  assert.deepEqual(
    output.text
      .split('\n')
      .filter((line) => MANAGED_CRATE_HELP_NAMES.includes(line.trim())),
    MANAGED_CRATE_HELP_NAMES.map((name) => `  ${name}`),
  );
});

test('CLI parser preserves aliases, normalized allow input, and current precedence', () => {
  assert.deepEqual(
    parseCratePublicApiArgs([
      '--crate', 'crate',
      '--allow', 'split',
      '--allow-crate=inline',
      '--allow-list=a,, b ',
    ]),
    {
      crateName: 'crate',
      extraAllowedCrates: ['split', 'inline', 'a', 'b'],
      allConfigured: false,
      help: false,
      selfTest: false,
    },
  );
  assert.equal(parseCratePublicApiArgs(['--help', '--self-test']).help, true);
  assert.throws(
    () => parseCratePublicApiArgs(['--all-configured', '--allow=x']),
    /cannot be combined/,
  );
  assert.throws(() => parseCratePublicApiArgs(['--']), /unknown option/);
  assert.throws(
    () => parseCratePublicApiArgs(['--crate', 'one', '--crate=two']),
    /specified more than once/,
  );
});

test('CLI renders gate skip through injected streams and returns the gate classification', async () => {
  const stdout = sink();
  const stderr = sink();
  const exitCode = await runCratePublicApiCli({
    argv: ['not-present'],
    gateDependencies: {
      async cargoMetadata() { return { packages: [] }; },
    },
    root,
    stderr,
    stdout,
  });
  assert.equal(exitCode, 0);
  assert.equal(
    stdout.text,
    'SKIP public API check for not-present: package is not present in this workspace yet.\n',
  );
  assert.equal(stderr.text, '');
});

function sink() {
  return {
    text: '',
    write(chunk) { this.text += chunk; },
  };
}
