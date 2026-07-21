import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

import { captureCheckedCommand } from '../lib/command-execution.mjs';
import {
  defaultInstanceConfig,
  defaultInstanceConfigText,
  instanceSummary,
  readInstanceConfig,
} from '../lib/local-instance-config.mjs';

const testDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(testDir, '..', '..');
const instanceScript = join(skiffRoot, 'scripts', 'skiff-instance.mjs');

test('instance config defaults environment to dev and exposes it in the summary', () => {
  const config = defaultInstanceConfig({
    configPath: '/tmp/skiff-instance/config.yml',
    repoRoot: skiffRoot,
  });

  assert.match(defaultInstanceConfigText(), /^environment: dev$/m);
  assert.equal(config.environment, 'dev');
  assert.equal(instanceSummary(config).environment, 'dev');
});

test('instance init writes the configured environment into router.yml', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-environment-'));
  const configPath = join(root, 'config.yml');
  const devHome = join(root, 'dev-home');
  try {
    await writeFile(configPath, instanceConfigText({
      environment: 'f04-host-test',
      devHome,
    }));

    await captureCheckedCommand(process.execPath, [instanceScript, 'init', configPath], {
      cwd: skiffRoot,
    });

    const config = await readInstanceConfig({ configPath, repoRoot: skiffRoot });
    assert.equal(config.environment, 'f04-host-test');
    assert.equal(instanceSummary(config).environment, 'f04-host-test');
    assert.match(
      await readFile(join(devHome, 'router.yml'), 'utf8'),
      /^environment: "f04-host-test"$/m,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('instance config rejects invalid environment names', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-invalid-environment-'));
  try {
    for (const [index, environment] of [
      '""',
      '"   "',
      '.',
      '..',
      'dev/test',
      'true',
      '42',
      '开发',
      `x${'a'.repeat(200)}`,
      '',
    ].entries()) {
      const configPath = join(root, `config-${index}.yml`);
      await writeFile(configPath, instanceConfigText({
        environment,
        devHome: join(root, 'dev-home'),
      }));
      await assert.rejects(
        readInstanceConfig({ configPath, repoRoot: skiffRoot }),
        /environment must be an ASCII token/,
        `expected environment ${JSON.stringify(environment)} to be rejected`,
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function instanceConfigText({ environment, devHome }) {
  return [
    `environment: ${environment}`,
    `devHome: ${JSON.stringify(devHome)}`,
    `cargoTargetDir: ${JSON.stringify(join(devHome, 'cargo-target'))}`,
    'components:',
    '  telemetry: disabled',
    '  mongo: disabled',
    '  watch: disabled',
    '',
  ].join('\n');
}
