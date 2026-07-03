#!/usr/bin/env node

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  defaultInstanceConfig,
  defaultInstancePorts,
  instanceSummary,
  readInstanceConfig,
} from './lib/local-instance-config.mjs';
import {
  defaultDevHome,
  devRuntimePaths,
} from './lib/dev-runtime-paths.mjs';

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
  assert.equal(expected.paths.configPath, configPath);
  assert.equal(expected.paths.instanceRoot, instanceRoot);
  assert.equal(expected.paths.devHome, join(instanceRoot, 'dev-home'));
  assert.equal(expected.paths.artifactRoot, join(instanceRoot, 'dev-home', 'artifacts'));
  assert.equal(expected.urls.routerReload, 'http://127.0.0.1:4101/__skiff/reload-artifacts');

  await run('node', [skiffCli, 'instance', 'init', configPath]);
  const configText = await readFile(configPath, 'utf8');
  assert.match(configText, /^devHome: /m);
  assert.match(configText, /^  base: 4100$/m);
  assert.match(configText, /^  mongo: 27017$/m);

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
  assert.equal(paths.basePort, 4100);
  assert.equal(paths.routerHttpPort, 4100);
  assert.equal(paths.routerControlPort, 4101);
  assert.equal(paths.telemetryPort, 4102);
  assert.equal(paths.mongoPort, 27017);
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
    defaultCustomConfigText.replace(/^  base: 4100$/m, '  base: 4300'),
  );
  const customConfigText = await readFile(customConfigPath, 'utf8');
  assert.match(customConfigText, /^  base: 4300$/m);
  assert.match(customConfigText, /^  mongo: 27017$/m);
  const custom = await readInstanceConfig({ configPath: customConfigPath, repoRoot: skiffRoot });
  assert.equal(custom.ports.routerHttp, 4300);
  assert.equal(custom.ports.routerControl, 4301);
  assert.equal(custom.ports.telemetry, 4302);
  assert.equal(custom.ports.mongo, 27017);

  await assertMissing(join(instanceRoot, 'skiff.yml'));
  await assertMissing(join(instanceRoot, 'skiff.local.yml'));
  await assertMissing(join(skiffRoot, 'skiff.yml'));
  console.log('[check-local-instance] ok');
} finally {
  await rm(tempRoot, { recursive: true, force: true });
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
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: skiffRoot,
      stdio: 'inherit',
      env: process.env,
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise();
        return;
      }
      reject(new Error(`${command} exited with ${signal ?? code}`));
    });
  });
}

function runCapture(command, args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: skiffRoot,
      stdio: ['ignore', 'pipe', 'pipe'],
      env: process.env,
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise(stdout);
        return;
      }
      reject(new Error(`${command} exited with ${signal ?? code}: ${stderr}`));
    });
  });
}
