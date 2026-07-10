import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { renderRuntimeConfig } from '../lib/runtime-stack-config.mjs';

const scriptsDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const repoRoot = path.dirname(scriptsDir);
const deployScript = path.join(scriptsDir, 'deploy-runtime-stack.mjs');

test('runtime config renders an optional host keyring mount path', () => {
  const common = {
    routerUrl: 'ws://127.0.0.1:4001/runtime',
    runtimeHome: '/srv/skiff/runtime-home',
    httpResponseMaxBytes: 8388608,
  };

  assert.doesNotMatch(renderRuntimeConfig(common), /serviceDb|keyringFile/);
  assert.match(
    renderRuntimeConfig({
      ...common,
      serviceDbEncryptionKeyringFile: '/run/secrets/skiff-service-db-keyring.json',
    }),
    /serviceDb:\n  encryption:\n    keyringFile: "\/run\/secrets\/skiff-service-db-keyring\.json"/,
  );
});

test('deploy CLI writes only the remote keyring path to runtime.yml', async () => {
  const mountPath = '/run/secrets/skiff-service-db-keyring.json';
  const result = await runDeploy({
    args: ['--service-db-encryption-keyring-file', mountPath],
  });
  try {
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.runtimeConfig, new RegExp(`keyringFile: ${JSON.stringify(mountPath)}`));

    const summary = JSON.parse(result.stdout);
    assert.equal(summary.serviceDb.encryptionKeyringConfigured, true);
    assert.equal(result.stdout.includes(mountPath), false);
    assert.equal(result.commandLog.includes(mountPath), false);
    assert.equal(result.commandLog.includes('skiff-service-db-keyring.json'), false);
  } finally {
    await result.cleanup();
  }
});

test('deploy CLI accepts the keyring mount path from the environment', async () => {
  const mountPath = '/var/run/skiff/keyring.json';
  const result = await runDeploy({
    env: { SKIFF_SERVICE_DB_ENCRYPTION_KEYRING_FILE: mountPath },
  });
  try {
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.runtimeConfig, new RegExp(`keyringFile: ${JSON.stringify(mountPath)}`));
    assert.equal(JSON.parse(result.stdout).serviceDb.encryptionKeyringConfigured, true);
    assert.equal(result.stdout.includes(mountPath), false);
    assert.equal(result.commandLog.includes(mountPath), false);
  } finally {
    await result.cleanup();
  }
});

test('deploy CLI omits keyring config unless explicitly configured', async () => {
  const result = await runDeploy();
  try {
    assert.equal(result.code, 0, result.stderr);
    assert.doesNotMatch(result.runtimeConfig, /serviceDb|keyringFile/);
    assert.equal(JSON.parse(result.stdout).serviceDb.encryptionKeyringConfigured, false);
  } finally {
    await result.cleanup();
  }
});

test('deploy CLI rejects a relative remote keyring path before running commands', async () => {
  const result = await runDeploy({
    args: ['--service-db-encryption-keyring-file', 'secrets/keyring.json'],
  });
  try {
    assert.notEqual(result.code, 0);
    assert.match(
      result.stderr,
      /--service-db-encryption-keyring-file must be an absolute path on the remote runtime host/,
    );
    assert.equal(result.commandLog, '');
  } finally {
    await result.cleanup();
  }
});

async function runDeploy({ args = [], env = {} } = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'skiff-deploy-test-'));
  const fakeBin = path.join(root, 'bin');
  const captureRoot = path.join(root, 'capture');
  const commandLogPath = path.join(captureRoot, 'commands.jsonl');
  const runtimeConfigPath = path.join(captureRoot, 'runtime.yml');
  const manifestPath = path.join(root, 'manifest.json');
  await mkdir(fakeBin, { recursive: true });
  await mkdir(captureRoot, { recursive: true });

  const binaryPath = path.relative(repoRoot, process.execPath);
  await writeFile(manifestPath, JSON.stringify({
    schemaVersion: 'skiff-runtime-stack-build-v1',
    commit: 'test-commit',
    units: {
      runtime: rustBuildUnit(binaryPath),
      'artifact-identity': rustBuildUnit(binaryPath),
    },
  }));
  await writeFakeCommand(path.join(fakeBin, 'ssh'), false);
  await writeFakeCommand(path.join(fakeBin, 'rsync'), true);

  const childEnv = {
    ...process.env,
    PATH: `${fakeBin}${path.delimiter}${process.env.PATH ?? ''}`,
    SKIFF_DEPLOY_TEST_COMMAND_LOG: commandLogPath,
    SKIFF_DEPLOY_TEST_RUNTIME_CONFIG: runtimeConfigPath,
  };
  delete childEnv.SKIFF_SERVICE_DB_ENCRYPTION_KEYRING_FILE;
  Object.assign(childEnv, env);

  const child = await spawnCapture(process.execPath, [
    deployScript,
    '--remote',
    'deploy.test',
    '--only',
    'runtime',
    '--remote-skiff',
    '/srv/skiff',
    '--build-manifest',
    manifestPath,
    ...args,
  ], childEnv);

  return {
    ...child,
    commandLog: await readOptionalFile(commandLogPath),
    runtimeConfig: await readOptionalFile(runtimeConfigPath),
    cleanup: () => rm(root, { recursive: true, force: true }),
  };
}

function rustBuildUnit(binaryPath) {
  return {
    kind: 'rs',
    commit: 'test-commit',
    sourceKey: 'test-source',
    artifacts: [{ kind: 'binary', path: binaryPath }],
  };
}

async function writeFakeCommand(file, captureRuntimeConfig) {
  await writeFile(file, `#!/usr/bin/env node
import { appendFileSync, copyFileSync } from 'node:fs';
const args = process.argv.slice(2);
appendFileSync(process.env.SKIFF_DEPLOY_TEST_COMMAND_LOG, JSON.stringify({
  command: ${JSON.stringify(path.basename(file))},
  args,
}) + '\\n');
if (${captureRuntimeConfig} && args.at(-2)?.endsWith('/runtime.yml')) {
  copyFileSync(args.at(-2), process.env.SKIFF_DEPLOY_TEST_RUNTIME_CONFIG);
}
`);
  await chmod(file, 0o755);
}

function spawnCapture(command, args, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: repoRoot,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
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
      resolve({ code, signal, stdout, stderr });
    });
  });
}

async function readOptionalFile(file) {
  try {
    return await readFile(file, 'utf8');
  } catch (error) {
    if (error.code === 'ENOENT') {
      return '';
    }
    throw error;
  }
}
