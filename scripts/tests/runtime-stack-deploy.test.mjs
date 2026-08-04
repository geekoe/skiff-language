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
    environment: 'prod',
  };

  assert.doesNotMatch(
    renderRuntimeConfig(common),
    /serviceDb|keyringFile|maxRequestBytes|maxResponseBytes|maxConcurrency|idleTimeoutMs/,
  );
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
    assert.match(result.runtimeConfig, /^environment: "prod"$/m);
    assert.doesNotMatch(result.runtimeConfig, /^artifactRoots?:/m);
    assert.doesNotMatch(result.runtimeConfig, /mongoUrl/);
    assert.doesNotMatch(
      result.runtimeConfig,
      /maxRequestBytes|maxResponseBytes|maxConcurrency|idleTimeoutMs|bodyLimitBytes/,
    );
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

test('deploy CLI fails closed without the Router-owned Mongo URL', async () => {
  const result = await runDeploy({ includeServiceDbMongoUrl: false });
  try {
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /service DB Mongo URL is required/);
    assert.equal(result.commandLog, '');
  } finally {
    await result.cleanup();
  }
});

test('deploy CLI requires explicit positive HTTP byte ceilings', async () => {
  const missing = await runDeploy({ includeHttpByteCeilings: false });
  try {
    assert.notEqual(missing.code, 0);
    assert.match(missing.stderr, /SKIFF_HTTP_MAX_REQUEST_BYTES must be a positive safe integer/);
    assert.equal(missing.commandLog, '');
  } finally {
    await missing.cleanup();
  }

  for (const [option, value] of [
    ['--http-max-request-bytes', '0'],
    ['--http-max-response-bytes', '1.5'],
  ]) {
    const invalid = await runDeploy({ args: [option, value] });
    try {
      assert.notEqual(invalid.code, 0);
      assert.match(invalid.stderr, new RegExp(`${option} must be a positive safe integer`));
      assert.equal(invalid.commandLog, '');
    } finally {
      await invalid.cleanup();
    }
  }
});

test('deploy CLI renders an independent activation prepare timeout', async () => {
  const result = await runDeploy({
    only: 'router',
    args: ['--activation-prepare-timeout-ms', '130000'],
  });
  try {
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.routerConfig, /^activation:\n  prepareTimeoutMs: 130000$/m);
    assert.match(result.routerConfig, /^requestTimeoutMs: 20000$/m);
  } finally {
    await result.cleanup();
  }

  const invalid = await runDeploy({
    only: 'router',
    args: ['--activation-prepare-timeout-ms', '0'],
  });
  try {
    assert.notEqual(invalid.code, 0);
    assert.match(
      invalid.stderr,
      /--activation-prepare-timeout-ms must be a positive safe integer/,
    );
    assert.equal(invalid.commandLog, '');
  } finally {
    await invalid.cleanup();
  }
});

test('router deploy uploads the Rust binary and writes only supported PM2 args', async () => {
  const result = await runDeploy({ only: 'router' });
  try {
    assert.equal(result.code, 0, result.stderr);
    assert.doesNotMatch(result.routerConfig, /^ecosystemStoreCliPath:/m);
    assert.match(result.routerConfig, /^artifactsPath: "\/srv\/skiff\/artifacts"$/m);
    assert.match(result.routerConfig, /^  mongoUrl: "mongodb:\/\/127\.0\.0\.1:27017\/skiff"$/m);
    assert.match(result.routerConfig, /^  maxRequestBytes: 67108864$/m);
    assert.match(result.routerConfig, /^  maxResponseBytes: 8388608$/m);
    assert.match(result.routerConfig, /^activation:\n  prepareTimeoutMs: 120000$/m);
    assert.match(
      result.routerConfig,
      /^runtime:\n  port: 4001\n  path: \/runtime\n  maxConcurrency: 128$/m,
    );
    assert.doesNotMatch(result.routerConfig, /idleTimeoutMs/);
    assert.doesNotMatch(result.routerConfig, /bodyLimitBytes/);
    assert.doesNotMatch(result.routerConfig, /^artifactRoots?:/m);
    assert.match(
      result.ecosystemConfig,
      /script: '\/srv\/skiff\/bin\/skiff-router'/,
    );
    assert.match(
      result.ecosystemConfig,
      /args: '\/srv\/skiff\/config\/router\.yml'/,
    );
    assert.match(result.ecosystemConfig, /interpreter: 'none'/);
    assert.doesNotMatch(result.ecosystemConfig, /src\/router\/server\.ts/);
    const routerApp = result.ecosystemConfig.split("name: 'skiff-telemetry'")[0];
    assert.doesNotMatch(routerApp, /--import tsx/);
    assert.doesNotMatch(result.ecosystemConfig, /--release-mode/);

    const commands = result.commandLog.trim().split('\n').map((line) => JSON.parse(line));
    assert.equal(commands.some(({ command, args }) =>
      command === 'rsync'
      && args.at(-1) === 'deploy.test:/srv/skiff/bin/skiff-router'
      && path.resolve(repoRoot, args.at(-2)) === process.execPath
    ), true);
    assert.equal(commands.some(({ command, args }) =>
      command === 'ssh'
      && args.at(-1) === 'chmod +x /srv/skiff/bin/skiff-router'
    ), true);
    assert.equal(commands.some(({ command, args }) =>
      command === 'rsync'
      && args.at(-1) === 'deploy.test:/srv/skiff/bin/skiff-compiler'
    ), false, '--only router must not implicitly deploy the compiler');
    assert.equal(JSON.parse(result.stdout).deployed.router.artifacts.length, 1);
  } finally {
    await result.cleanup();
  }
});

test('router deploy fails before remote commands when the router build artifact is missing', async () => {
  const result = await runDeploy({ only: 'router', omitRouter: true });
  try {
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /router is missing from .*manifest\.json/);
    assert.equal(result.commandLog, '');
  } finally {
    await result.cleanup();
  }
});

async function runDeploy({
  args = [],
  env = {},
  only = 'runtime',
  omitCompiler = false,
  omitRouter = false,
  includeServiceDbMongoUrl = true,
  includeHttpByteCeilings = true,
} = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'skiff-deploy-test-'));
  const fakeBin = path.join(root, 'bin');
  const captureRoot = path.join(root, 'capture');
  const commandLogPath = path.join(captureRoot, 'commands.jsonl');
  const runtimeConfigPath = path.join(captureRoot, 'runtime.yml');
  const routerConfigPath = path.join(captureRoot, 'router.yml');
  const ecosystemConfigPath = path.join(captureRoot, 'ecosystem.config.cjs');
  const manifestPath = path.join(root, 'manifest.json');
  await mkdir(fakeBin, { recursive: true });
  await mkdir(captureRoot, { recursive: true });

  const binaryPath = path.relative(repoRoot, process.execPath);
  await writeFile(manifestPath, JSON.stringify({
    schemaVersion: 'skiff-runtime-stack-build-v1',
    commit: 'test-commit',
    units: {
      runtime: rustBuildUnit(binaryPath),
      ...(omitCompiler ? {} : { compiler: rustBuildUnit(binaryPath) }),
      ...(omitRouter ? {} : { router: rustBuildUnit(binaryPath) }),
    },
  }));
  await writeFakeCommand(path.join(fakeBin, 'ssh'));
  await writeFakeCommand(path.join(fakeBin, 'rsync'));

  const childEnv = {
    ...process.env,
    PATH: `${fakeBin}${path.delimiter}${process.env.PATH ?? ''}`,
    SKIFF_DEPLOY_TEST_COMMAND_LOG: commandLogPath,
    SKIFF_DEPLOY_TEST_RUNTIME_CONFIG: runtimeConfigPath,
    SKIFF_DEPLOY_TEST_ROUTER_CONFIG: routerConfigPath,
    SKIFF_DEPLOY_TEST_ECOSYSTEM_CONFIG: ecosystemConfigPath,
  };
  delete childEnv.SKIFF_SERVICE_DB_ENCRYPTION_KEYRING_FILE;
  delete childEnv.SKIFF_SERVICE_DB_MONGO_URL;
  delete childEnv.SERVICE_DB_MONGO_URL;
  delete childEnv.SKIFF_HTTP_MAX_REQUEST_BYTES;
  delete childEnv.SKIFF_HTTP_MAX_RESPONSE_BYTES;
  delete childEnv.SKIFF_ACTIVATION_PREPARE_TIMEOUT_MS;
  Object.assign(childEnv, env);

  const child = await spawnCapture(process.execPath, [
    deployScript,
    '--remote',
    'deploy.test',
    '--only',
    only,
    '--remote-skiff',
    '/srv/skiff',
    '--build-manifest',
    manifestPath,
    ...(includeServiceDbMongoUrl
      ? ['--service-db-mongo-url', 'mongodb://127.0.0.1:27017/skiff']
      : []),
    ...(includeHttpByteCeilings
      ? [
          '--http-max-request-bytes',
          '67108864',
          '--http-max-response-bytes',
          '8388608',
        ]
      : []),
    ...args,
  ], childEnv);

  return {
    ...child,
    commandLog: await readOptionalFile(commandLogPath),
    ecosystemConfig: await readOptionalFile(ecosystemConfigPath),
    routerConfig: await readOptionalFile(routerConfigPath),
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

async function writeFakeCommand(file) {
  await writeFile(file, `#!/usr/bin/env node
import { appendFileSync, copyFileSync } from 'node:fs';
const args = process.argv.slice(2);
appendFileSync(process.env.SKIFF_DEPLOY_TEST_COMMAND_LOG, JSON.stringify({
  command: ${JSON.stringify(path.basename(file))},
  args,
}) + '\\n');
if (${JSON.stringify(path.basename(file))} === 'rsync' && args.at(-2)?.endsWith('/runtime.yml')) {
  copyFileSync(args.at(-2), process.env.SKIFF_DEPLOY_TEST_RUNTIME_CONFIG);
}
if (${JSON.stringify(path.basename(file))} === 'rsync' && args.at(-2)?.endsWith('/router.yml')) {
  copyFileSync(args.at(-2), process.env.SKIFF_DEPLOY_TEST_ROUTER_CONFIG);
}
if (${JSON.stringify(path.basename(file))} === 'rsync' && args.at(-2)?.endsWith('/ecosystem.config.cjs')) {
  copyFileSync(args.at(-2), process.env.SKIFF_DEPLOY_TEST_ECOSYSTEM_CONFIG);
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
