import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

import { captureCheckedCommand } from '../lib/command-execution.mjs';
import { isolatedTestInstanceConfigText } from '../lib/isolated-test-runtime-instance.mjs';
import {
  defaultInstanceConfig,
  defaultInstanceConfigText,
  instanceSummary,
  readInstanceConfig,
} from '../lib/local-instance-config.mjs';

const testDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(testDir, '..', '..');
const instanceScript = join(skiffRoot, 'scripts', 'skiff-instance.mjs');

test('instance config defaults profile to dev and exposes it in the summary', () => {
  const configPath = '/tmp/skiff-instance/config.yml';
  const config = defaultInstanceConfig({
    configPath,
    repoRoot: skiffRoot,
  });

  assert.match(defaultInstanceConfigText(), /^profile: dev$/m);
  assert.match(defaultInstanceConfigText(), /^cargoTargetDir: \.\.\/build\/cargo-target$/m);
  assert.equal(config.profile, 'dev');
  assert.equal(config.paths.cargoTargetDir, resolve(dirname(configPath), '../build/cargo-target'));
  assert.equal(instanceSummary(config).profile, 'dev');
});

test('instance init writes the configured profile and root into router/runtime YAML', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-profile-'));
  const configPath = join(root, 'config.yml');
  const devHome = join(root, 'dev-home');
  try {
    await writeFile(configPath, instanceConfigText({
      profile: 'f04-host-test',
      devHome,
    }));

    await captureCheckedCommand(process.execPath, [instanceScript, 'init', configPath], {
      cwd: skiffRoot,
    });

    const config = await readInstanceConfig({ configPath, repoRoot: skiffRoot });
    assert.equal(config.profile, 'f04-host-test');
    assert.equal(instanceSummary(config).profile, 'f04-host-test');
    const expectedCompiler = join(
      devHome,
      'bin',
      process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler',
    );
    assert.equal(config.paths.ecosystemStoreCli, expectedCompiler);
    assert.equal(instanceSummary(config).ecosystemStoreCli, expectedCompiler);
    const routerConfig = await readFile(join(devHome, 'router.yml'), 'utf8');
    assert.match(
      routerConfig,
      /^profile: f04-host-test$/m,
    );
    assert.doesNotMatch(routerConfig, /^environment:/m);
    assert.doesNotMatch(routerConfig, /^ecosystemStoreCliPath:/m);
    assert.match(
      routerConfig,
      new RegExp(`^artifactsPath: ${JSON.stringify(join(devHome, 'artifacts'))}$`, 'm'),
    );
    assert.match(routerConfig, /^  mongoUrl: "mongodb:\/\/127\.0\.0\.1:27017/m);
    assert.match(routerConfig, /^  maxRequestBytes: 67108864$/m);
    assert.match(routerConfig, /^  maxResponseBytes: 8388608$/m);
    assert.match(routerConfig, /^activation:\n  prepareTimeoutMs: 120000$/m);
    assert.match(
      routerConfig,
      /^runtime:\n  port: \d+\n  path: \/runtime\n  maxConcurrency: 128$/m,
    );
    assert.doesNotMatch(routerConfig, /idleTimeoutMs/);
    assert.doesNotMatch(routerConfig, /bodyLimitBytes/);
    assert.doesNotMatch(routerConfig, /^artifactRoots?:/m);
    const runtimeConfig = await readFile(join(devHome, 'runtime.yml'), 'utf8');
    assert.doesNotMatch(runtimeConfig, /^(?:profile|environment):/m);
    assert.doesNotMatch(runtimeConfig, /^artifactRoots?:/m);
    assert.doesNotMatch(runtimeConfig, /mongoUrl/);
    assert.doesNotMatch(runtimeConfig, /maxConcurrency|idleTimeoutMs/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('isolated test instance writes explicit runtime concurrency to router.yml', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-isolated-instance-concurrency-'));
  const configPath = join(root, 'config.yml');
  const devHome = join(root, 'dev-home');
  try {
    await writeFile(configPath, isolatedTestInstanceConfigText({
      devHome,
      cargoTarget: join(root, 'cargo-target'),
      basePort: 46100,
      mongoPort: 46103,
    }));
    await captureCheckedCommand(process.execPath, [instanceScript, 'init', configPath], {
      cwd: skiffRoot,
    });

    const routerConfig = await readFile(join(devHome, 'router.yml'), 'utf8');
    assert.match(
      routerConfig,
      /^runtime:\n  port: 46101\n  path: \/runtime\n  maxConcurrency: 128$/m,
    );
    assert.doesNotMatch(routerConfig, /idleTimeoutMs/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('instance config rejects invalid profile names', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-invalid-profile-'));
  try {
    for (const [index, profile] of [
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
        profile,
        devHome: join(root, 'dev-home'),
      }));
      await assert.rejects(
        readInstanceConfig({ configPath, repoRoot: skiffRoot }),
        /profile must be an ASCII token/,
        `expected profile ${JSON.stringify(profile)} to be rejected`,
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('instance config requires explicit positive safe HTTP byte ceilings', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-invalid-http-'));
  try {
    const missingPath = join(root, 'missing.yml');
    await writeFile(missingPath, [
      'profile: dev',
      `devHome: ${JSON.stringify(join(root, 'dev-home'))}`,
      '',
    ].join('\n'));
    await assert.rejects(
      readInstanceConfig({ configPath: missingPath, repoRoot: skiffRoot }),
      /http must be a mapping with explicit maxRequestBytes and maxResponseBytes/,
    );

    for (const [index, value] of ['0', '-1', '1.5'].entries()) {
      const configPath = join(root, `invalid-${index}.yml`);
      await writeFile(configPath, instanceConfigText({
        profile: 'dev',
        devHome: join(root, `dev-home-${index}`),
        maxResponseBytes: value,
      }));
      await assert.rejects(
        readInstanceConfig({ configPath, repoRoot: skiffRoot }),
        /http\.maxResponseBytes must be a positive safe integer/,
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('instance config owns an explicit positive activation prepare timeout', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-instance-invalid-activation-'));
  try {
    const configPath = join(root, 'custom.yml');
    await writeFile(configPath, instanceConfigText({
      profile: 'dev',
      devHome: join(root, 'dev-home'),
      activationPrepareTimeoutMs: '130000',
    }));
    const config = await readInstanceConfig({ configPath, repoRoot: skiffRoot });
    assert.equal(config.activation.prepareTimeoutMs, 130000);
    assert.equal(instanceSummary(config).activationPrepareTimeoutMs, 130000);

    const oldConfigPath = join(root, 'without-activation.yml');
    await writeFile(oldConfigPath, [
      'profile: dev',
      `devHome: ${JSON.stringify(join(root, 'old-home'))}`,
      'http:',
      '  maxRequestBytes: 67108864',
      '  maxResponseBytes: 8388608',
      '',
    ].join('\n'));
    const oldConfig = await readInstanceConfig({
      configPath: oldConfigPath,
      repoRoot: skiffRoot,
    });
    assert.equal(oldConfig.activation.prepareTimeoutMs, 120000);

    for (const [index, value] of ['0', '-1', '1.5', '"120000"'].entries()) {
      const invalidPath = join(root, `invalid-${index}.yml`);
      await writeFile(invalidPath, instanceConfigText({
        profile: 'dev',
        devHome: join(root, `invalid-home-${index}`),
        activationPrepareTimeoutMs: value,
      }));
      await assert.rejects(
        readInstanceConfig({ configPath: invalidPath, repoRoot: skiffRoot }),
        /activation\.prepareTimeoutMs must be a positive safe integer/,
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function instanceConfigText({
  profile,
  devHome,
  maxRequestBytes = '67108864',
  maxResponseBytes = '8388608',
  activationPrepareTimeoutMs = '120000',
}) {
  return [
    `profile: ${profile}`,
    `devHome: ${JSON.stringify(devHome)}`,
    `cargoTargetDir: ${JSON.stringify(join(devHome, 'cargo-target'))}`,
    'http:',
    `  maxRequestBytes: ${maxRequestBytes}`,
    `  maxResponseBytes: ${maxResponseBytes}`,
    'activation:',
    `  prepareTimeoutMs: ${activationPrepareTimeoutMs}`,
    'components:',
    '  telemetry: disabled',
    '  mongo: disabled',
    '  watch: disabled',
    '',
  ].join('\n');
}
