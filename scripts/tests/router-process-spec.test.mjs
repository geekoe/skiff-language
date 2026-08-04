import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const testDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(testDir, '..', '..');
const instanceScript = join(skiffRoot, 'scripts', 'skiff-instance.mjs');

test('instance paths accepts the default rust router implementation', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-process-spec-'));
  try {
    for (const implementation of [undefined, 'rust']) {
      const configPath = join(root, `${implementation}.yml`);
      await writeFile(configPath, instanceConfigText({
        devHome: join(root, `${implementation}-home`),
        implementation,
      }));
      const outcome = await runInstance('paths', configPath, ['--json']);
      assert.equal(outcome.code, 0, outcome.stderr);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('instance paths rejects the retired TS implementation and non-mapping router', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-process-spec-invalid-'));
  try {
    const devHome = join(root, 'dev-home');
    const invalid = join(root, 'invalid.yml');
    await writeFile(invalid, instanceConfigText({
      devHome,
      implementation: 'ts',
    }));
    let outcome = await runInstance('paths', invalid, ['--json']);
    assert.notEqual(outcome.code, 0);
    assert.match(
      outcome.stderr,
      /router\.implementation is no longer selectable/,
    );

    const scalar = join(root, 'scalar.yml');
    await writeFile(
      scalar,
      `${instanceConfigText({ devHome })}\nrouter: ts\n`,
    );
    outcome = await runInstance('paths', scalar, ['--json']);
    assert.notEqual(outcome.code, 0);
    assert.match(outcome.stderr, /router must be a mapping/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function instanceConfigText({ devHome, implementation }) {
  return [
    'profile: dev',
    `devHome: ${JSON.stringify(devHome)}`,
    `cargoTargetDir: ${JSON.stringify(join(devHome, 'cargo-target'))}`,
    ...(implementation === undefined
      ? []
      : ['router:', `  implementation: ${implementation}`]),
    'http:',
    '  maxRequestBytes: 67108864',
    '  maxResponseBytes: 8388608',
    'components:',
    '  telemetry: disabled',
    '  mongo: disabled',
    '  watch: disabled',
    '',
  ].join('\n');
}

function runInstance(subcommand, configPath, extraArgs = []) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(
      process.execPath,
      [instanceScript, subcommand, configPath, ...extraArgs],
      { cwd: skiffRoot },
    );
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
