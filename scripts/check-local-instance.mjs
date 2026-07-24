#!/usr/bin/env node

import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  defaultInstanceConfig,
  defaultInstancePorts,
  instanceSummary,
  readInstanceConfig,
} from './lib/local-instance-config.mjs';
import {
  captureCheckedCommand,
  runAttachedCommand,
} from './lib/command-execution.mjs';
import {
  defaultDevHome,
  devRuntimePaths,
} from './lib/dev-runtime-paths.mjs';
import {
  localServiceDbKeyId,
  serviceDbKeyringFormat,
} from './lib/service-db-keyring.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');
const skiffCli = join(scriptDir, 'skiff.mjs');
const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-local-instance-check-'));
const configPath = join(tempRoot, '.skiff-instance', 'config.yml');
const instanceRoot = dirname(configPath);

try {
  assert.equal(defaultDevHome({ HOME: join(tempRoot, 'home') }), join(skiffRoot, '.skiff-instance', 'dev-home'));
  assert.equal(devRuntimePaths({ env: { HOME: join(tempRoot, 'home') } }).devHome, join(skiffRoot, '.skiff-instance', 'dev-home'));

  const expected = defaultInstanceConfig({ configPath, repoRoot: skiffRoot });
  assert.equal(expected.ports.base, defaultInstancePorts.base);
  assert.equal(expected.ports.routerHttp, defaultInstancePorts.routerHttp);
  assert.equal(expected.ports.routerControl, defaultInstancePorts.routerControl);
  assert.equal(expected.ports.telemetry, defaultInstancePorts.telemetry);
  assert.equal(expected.ports.mongo, defaultInstancePorts.mongo);
  assert.equal(expected.http.maxRequestBytes, 67108864);
  assert.equal(expected.http.maxResponseBytes, 8388608);
  assert.equal(expected.paths.configPath, configPath);
  assert.equal(expected.paths.instanceRoot, instanceRoot);
  assert.equal(expected.paths.devHome, join(instanceRoot, 'dev-home'));
  assert.equal(expected.paths.artifactRoot, join(instanceRoot, 'dev-home', 'artifacts'));
  assert.equal(expected.paths.secretsDir, join(instanceRoot, 'dev-home', 'secrets'));
  assert.equal(
    expected.paths.serviceDbEncryptionKeyringFile,
    join(instanceRoot, 'dev-home', 'secrets', 'service-db-keyring.json'),
  );
  assert.equal(expected.urls.routerReload, 'http://127.0.0.1:4101/__skiff/reload-artifacts');

  await run('node', [skiffCli, 'instance', 'init', configPath]);
  const configText = await readFile(configPath, 'utf8');
  assert.match(configText, /^devHome: /m);
  assert.match(configText, /^  base: 4100$/m);
  assert.match(configText, /^  mongo: 27017$/m);
  assert.match(configText, /^  maxRequestBytes: 67108864$/m);
  assert.match(configText, /^  maxResponseBytes: 8388608$/m);

  const runtimeConfigText = await readFile(expected.paths.runtimeConfig, 'utf8');
  const routerConfigText = await readFile(expected.paths.routerConfig, 'utf8');
  assert.match(
    routerConfigText,
    new RegExp(`^artifactsPath: ${escapeRegExp(JSON.stringify(expected.paths.artifactRoot))}$`, 'm'),
  );
  assert.match(routerConfigText, /^  mongoUrl: /m);
  assert.match(routerConfigText, /^  maxRequestBytes: 67108864$/m);
  assert.match(routerConfigText, /^  maxResponseBytes: 8388608$/m);
  assert.doesNotMatch(routerConfigText, /bodyLimitBytes/);
  assert.doesNotMatch(runtimeConfigText, /maxRequestBytes|maxResponseBytes|bodyLimitBytes/);
  assert.doesNotMatch(routerConfigText, /^artifactRoots?:/m);
  assert.doesNotMatch(runtimeConfigText, /^artifactRoots?:/m);
  assert.doesNotMatch(runtimeConfigText, /mongoUrl/);
  assert.match(runtimeConfigText, /^serviceDb:$/m);
  assert.match(runtimeConfigText, /^  encryption:$/m);
  assert.match(
    runtimeConfigText,
    new RegExp(`^    keyringFile: ${escapeRegExp(JSON.stringify(expected.paths.serviceDbEncryptionKeyringFile))}$`, 'm'),
  );
  const firstKeyringText = await readFile(expected.paths.serviceDbEncryptionKeyringFile, 'utf8');
  assertValidProvisionedKeyring(firstKeyringText);
  if (process.platform !== 'win32') {
    assert.equal(
      (await stat(expected.paths.serviceDbEncryptionKeyringFile)).mode & 0o777,
      0o600,
    );
  }

  await run('node', [skiffCli, 'instance', 'init', configPath, '--force']);
  assert.equal(
    await readFile(expected.paths.serviceDbEncryptionKeyringFile, 'utf8'),
    firstKeyringText,
    're-running an instance lifecycle ensure must preserve the existing key',
  );

  const loaded = await readInstanceConfig({ configPath, repoRoot: skiffRoot });
  assert.deepEqual(instanceSummary(loaded).components, {
    telemetry: 'managed',
    mongo: 'disabled',
    watch: 'disabled',
  });

  const paths = JSON.parse(await runCapture('node', [skiffCli, 'instance', 'paths', configPath, '--json']));
  assert.equal(paths.configPath, configPath);
  assert.equal(paths.instanceRoot, instanceRoot);
  assert.equal(paths.devHome, join(instanceRoot, 'dev-home'));
  assert.equal(paths.artifactRoot, join(instanceRoot, 'dev-home', 'artifacts'));
  assert.equal(paths.secretsDir, expected.paths.secretsDir);
  assert.equal(
    paths.serviceDbEncryptionKeyringFile,
    expected.paths.serviceDbEncryptionKeyringFile,
  );
  assert.equal(paths.basePort, 4100);
  assert.equal(paths.routerHttpPort, 4100);
  assert.equal(paths.routerControlPort, 4101);
  assert.equal(paths.telemetryPort, 4102);
  assert.equal(paths.mongoPort, 27017);
  assert.equal(paths.httpMaxRequestBytes, 67108864);
  assert.equal(paths.httpMaxResponseBytes, 8388608);
  assert.equal(paths.routerReloadUrl, 'http://127.0.0.1:4101/__skiff/reload-artifacts');

  const status = JSON.parse(await runCapture('node', [skiffCli, 'instance', 'status', configPath, '--json']));
  assert.equal(status.configPath, configPath);
  assert.equal(status.instanceRoot, instanceRoot);
  assert.equal(status.urls.routerHttp, 'http://127.0.0.1:4100');
  assert.deepEqual(status.processes.map((processStatus) => processStatus.name), [
    'telemetry',
    'router',
    'runtime',
  ]);
  assert.ok(status.processes.every((processStatus) => processStatus.running === false));

  const customConfigPath = join(tempRoot, 'custom-instance', 'config.yml');
  await run('node', [skiffCli, 'instance', 'init', customConfigPath]);
  const defaultCustomConfigText = await readFile(customConfigPath, 'utf8');
  await writeFile(
    customConfigPath,
    defaultCustomConfigText
      .replace(/^devHome: dev-home$/m, 'devHome: custom-dev-home')
      .replace(/^  base: 4100$/m, '  base: 4300'),
  );
  await run('node', [skiffCli, 'instance', 'init', customConfigPath]);
  const customConfigText = await readFile(customConfigPath, 'utf8');
  assert.match(customConfigText, /^devHome: custom-dev-home$/m);
  assert.match(customConfigText, /^  base: 4300$/m);
  assert.match(customConfigText, /^  mongo: 27017$/m);
  const custom = await readInstanceConfig({ configPath: customConfigPath, repoRoot: skiffRoot });
  assert.equal(custom.paths.devHome, join(dirname(customConfigPath), 'custom-dev-home'));
  assertValidProvisionedKeyring(
    await readFile(custom.paths.serviceDbEncryptionKeyringFile, 'utf8'),
  );
  assert.match(
    await readFile(custom.paths.runtimeConfig, 'utf8'),
    new RegExp(`^    keyringFile: ${escapeRegExp(JSON.stringify(custom.paths.serviceDbEncryptionKeyringFile))}$`, 'm'),
  );
  assert.equal(custom.ports.routerHttp, 4300);
  assert.equal(custom.ports.routerControl, 4301);
  assert.equal(custom.ports.telemetry, 4302);
  assert.equal(custom.ports.mongo, 27017);

  const concurrentKeyringPath = join(
    tempRoot,
    'concurrent-instance',
    'dev-home',
    'secrets',
    'service-db-keyring.json',
  );
  const keyringHelperUrl = pathToFileURL(
    join(scriptDir, 'lib', 'service-db-keyring.mjs'),
  ).href;
  const concurrentEnsureProgram = [
    `import { ensureLocalServiceDbKeyring } from ${JSON.stringify(keyringHelperUrl)};`,
    'console.log(JSON.stringify(await ensureLocalServiceDbKeyring(process.argv[1])));',
  ].join('\n');
  const concurrentEnsures = await Promise.all(
    Array.from({ length: 16 }, async () => JSON.parse(await runCapture('node', [
      '--input-type=module',
      '--eval',
      concurrentEnsureProgram,
      concurrentKeyringPath,
    ]))),
  );
  assert.equal(
    concurrentEnsures.filter(({ action }) => action === 'created').length,
    1,
    'exactly one concurrent ensure must install the keyring',
  );
  assert.ok(concurrentEnsures.every(({ path }) => path === concurrentKeyringPath));
  const concurrentKeyringText = await readFile(concurrentKeyringPath, 'utf8');
  assertValidProvisionedKeyring(concurrentKeyringText);
  assert.equal(
    await readFile(concurrentKeyringPath, 'utf8'),
    concurrentKeyringText,
    'all concurrent losers must observe the complete installed file',
  );
  assert.deepEqual(
    (await readdir(dirname(concurrentKeyringPath))).sort(),
    ['service-db-keyring.json'],
    'provisioning must remove its lock and same-directory temporary file',
  );

  await assertMissing(join(instanceRoot, 'skiff.yml'));
  await assertMissing(join(instanceRoot, 'skiff.local.yml'));
  await assertMissing(join(skiffRoot, 'skiff.yml'));
  console.log('[check-local-instance] ok');
} finally {
  await rm(tempRoot, { recursive: true, force: true });
}

function assertValidProvisionedKeyring(text) {
  const keyring = JSON.parse(text);
  assert.deepEqual(Object.keys(keyring).sort(), ['activeKeyId', 'format', 'keys']);
  assert.equal(keyring.format, serviceDbKeyringFormat);
  assert.equal(keyring.activeKeyId, localServiceDbKeyId);
  assert.deepEqual(Object.keys(keyring.keys), [localServiceDbKeyId]);
  const material = keyring.keys[localServiceDbKeyId];
  assert.match(material, /^[A-Za-z0-9+/]{43}=$/);
  assert.equal(Buffer.from(material, 'base64').length, 32);
  assert.equal(Buffer.from(material, 'base64').toString('base64'), material);
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

async function assertMissing(path) {
  try {
    await stat(path);
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  throw new Error(`${path} should not exist`);
}

function run(command, args) {
  return runAttachedCommand(command, args, { cwd: skiffRoot, env: process.env });
}

async function runCapture(command, args) {
  try {
    const result = await captureCheckedCommand(command, args, {
      cwd: skiffRoot,
      env: process.env,
    });
    return result.stdout;
  } catch (error) {
    throw new Error([
      `${command} exited with ${error?.signal ?? error?.code ?? 'UNKNOWN'}`,
      streamDiagnostic('stderr', error?.stderr),
      streamDiagnostic('stdout', error?.stdout),
    ].filter(Boolean).join('\n'));
  }
}

function streamDiagnostic(label, value) {
  return typeof value === 'string' && value.trim().length > 0
    ? `${label}:\n${value.trim()}`
    : '';
}
