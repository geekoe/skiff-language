import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { installManagedBinary } from '../lib/managed-binary.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..', '..');
const skiffCli = join(scriptDir, '..', 'skiff.mjs');

test('same-content managed install repairs executable mode atomically', {
  skip: process.platform === 'win32',
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-managed-mode-'));
  const source = join(root, 'source');
  const destination = join(root, 'bin', 'skiff-compiler');
  try {
    await mkdir(dirname(destination), { recursive: true });
    await writeFile(source, '#!/usr/bin/env node\n');
    await writeFile(destination, '#!/usr/bin/env node\n');
    await chmod(source, 0o755);
    await chmod(destination, 0o644);

    await installManagedBinary(source, destination);

    assert.equal((await stat(destination)).mode & 0o7777, 0o755);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('managed runtime binary identity restarts only the matching stale instance', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-managed-binary-'));
  const fakeBin = join(root, 'fake-bin');
  const cargoTargetDir = join(root, 'cargo-target');
  const launchLog = join(root, 'runtime-launches.log');
  const baseA = await reservePortPair();
  const baseB = await reservePortPair(new Set([baseA, baseA + 1]));
  const configA = join(root, 'instance-a', 'config.yml');
  const configB = join(root, 'instance-b', 'config.yml');
  const env = {
    ...process.env,
    PATH: `${fakeBin}:${process.env.PATH}`,
    CARGO_TARGET_DIR: cargoTargetDir,
    FAKE_RUNTIME_LAUNCH_LOG: launchLog,
  };

  await mkdir(fakeBin, { recursive: true });
  await writeFakeCargo(join(fakeBin, 'cargo'));
  await writeFakePnpm(join(fakeBin, 'pnpm'));

  try {
    await initializeFixture(configA, baseA, cargoTargetDir, env);
    await initializeFixture(configB, baseB, cargoTargetDir, env);

    await run('node', [skiffCli, 'instance', 'up', configA], { env: { ...env, FAKE_RUNTIME_VERSION: 'a1' } });
    await run('node', [skiffCli, 'instance', 'up', configB], { env: { ...env, FAKE_RUNTIME_VERSION: 'b1' } });
    const pathsA = JSON.parse(await runCapture(
      'node',
      [skiffCli, 'instance', 'paths', configA, '--json'],
      { env },
    ));
    assert.equal(
      pathsA.ecosystemStoreCli,
      join(
        pathsA.devHome,
        'bin',
        process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler',
      ),
    );
    const compilerInfo = await stat(pathsA.ecosystemStoreCli);
    assert.equal(compilerInfo.isFile(), true);
    if (process.platform !== 'win32') {
      assert.notEqual(compilerInfo.mode & 0o111, 0, 'managed compiler install must be executable');
    }
    const initialA = await runtimeStatus(configA, env);
    const initialB = await runtimeStatus(configB, env);
    const initialRouterA = await componentStatus(configA, 'router', env);
    assert.equal(initialA.category, 'running');
    assert.equal(initialA.managedBinary.matches, true);

    await run('node', [skiffCli, 'instance', 'up', configA], { env: { ...env, FAKE_RUNTIME_VERSION: 'a1' } });
    const unchangedA = await runtimeStatus(configA, env);
    assert.equal(unchangedA.pid, initialA.pid, 'unchanged binary must keep the runtime PID');
    assert.deepEqual(
      unchangedA.managedBinary.currentIdentity.file,
      unchangedA.managedBinary.recordedIdentity.file,
      'same-content install must preserve the executable file identity fast path',
    );

    const buildOutput = await runCapture(
      'node',
      [skiffCli, 'instance', 'build', configA],
      { env: { ...env, FAKE_RUNTIME_VERSION: 'a2' } },
    );
    const buildResult = JSON.parse(buildOutput);
    assert.deepEqual(buildResult.staleProcesses, [{ name: 'runtime', pid: initialA.pid }]);
    assert.match(buildResult.recovery, /instance refresh-binaries/);
    const staleA = await runtimeStatus(configA, env);
    assert.equal(staleA.category, 'stale-binary');
    assert.equal(staleA.pid, initialA.pid, 'build-only must report rather than silently mutate a running process');
    assert.equal(staleA.managedBinary.matches, false);

    await run('node', [skiffCli, 'instance', 'up', configA], { env: { ...env, FAKE_RUNTIME_VERSION: 'a2' } });
    const restartedA = await runtimeStatus(configA, env);
    assert.equal(restartedA.category, 'running');
    assert.notEqual(restartedA.pid, initialA.pid, 'instance up must replace a stale runtime');
    assert.equal((await componentStatus(configA, 'router', env)).pid, initialRouterA.pid, 'runtime refresh must not restart another component');
    assert.equal((await runtimeStatus(configB, env)).pid, initialB.pid, 'another instance must not be restarted');

    await rm(pathsA.ecosystemStoreCli);
    const compilerRebuildOutput = JSON.parse(await runCapture(
      'node',
      [skiffCli, 'instance', 'build', configA],
      { env: { ...env, FAKE_RUNTIME_VERSION: 'a3' } },
    ));
    assert.deepEqual(compilerRebuildOutput.staleProcesses, [{ name: 'runtime', pid: restartedA.pid }]);
    const compilerAfterBuildDev = await stat(pathsA.ecosystemStoreCli);
    assert.equal(compilerAfterBuildDev.isFile(), true);
    if (process.platform !== 'win32') {
      assert.notEqual(
        compilerAfterBuildDev.mode & 0o111,
        0,
        'instance build must reinstall the managed compiler as executable',
      );
    }
    await run('node', [skiffCli, 'instance', 'refresh-binaries', configA], { env });
    const refreshedA = await runtimeStatus(configA, env);
    assert.equal(refreshedA.category, 'running');
    assert.notEqual(refreshedA.pid, restartedA.pid, 'standard build/install must reconcile the active runtime');
    assert.equal((await runtimeStatus(configB, env)).pid, initialB.pid, 'build/install refresh must stay config-scoped');

    const buildOnlyOutput = JSON.parse(await runCapture(
      'node',
      [skiffCli, 'instance', 'build', configA],
      { env: { ...env, FAKE_RUNTIME_VERSION: 'a4' } },
    ));
    assert.deepEqual(buildOnlyOutput.staleProcesses, [{ name: 'runtime', pid: refreshedA.pid }]);
    assert.equal(
      buildOnlyOutput.recovery,
      `node scripts/skiff.mjs instance refresh-binaries ${configA}`,
    );
    const explicitlyStaleA = await runtimeStatus(configA, env);
    assert.equal(explicitlyStaleA.category, 'stale-binary');
    assert.equal(explicitlyStaleA.pid, refreshedA.pid, 'instance build must preserve build-only behavior');
    await run('node', [skiffCli, 'instance', 'refresh-binaries', configA], { env });
    const explicitlyRefreshedA = await runtimeStatus(configA, env);
    assert.equal(explicitlyRefreshedA.category, 'running');
    assert.notEqual(explicitlyRefreshedA.pid, refreshedA.pid);
    assert.equal((await runtimeStatus(configB, env)).pid, initialB.pid);

    await run('node', [skiffCli, 'instance', 'build', configA], {
      env: { ...env, FAKE_RUNTIME_VERSION: 'a5' },
    });
    const beforeRepairA = await runtimeStatus(configA, env);
    assert.equal(beforeRepairA.category, 'stale-binary');
    await run('node', [skiffCli, 'instance', 'repair', configA], {
      env: { ...env, FAKE_RUNTIME_VERSION: 'a5' },
    });
    const repairedA = await runtimeStatus(configA, env);
    assert.equal(repairedA.category, 'running');
    assert.notEqual(repairedA.pid, beforeRepairA.pid, 'instance repair must replace a stale runtime');
    assert.equal((await componentStatus(configA, 'router', env)).pid, initialRouterA.pid);
    assert.equal((await runtimeStatus(configB, env)).pid, initialB.pid);

    const launches = await readFile(launchLog, 'utf8');
    assert.match(launches, /^a1 \d+$/m);
    assert.match(launches, /^a2 \d+$/m);
    assert.match(launches, /^a3 \d+$/m);
    assert.match(launches, /^a4 \d+$/m);
    assert.match(launches, /^a5 \d+$/m);
    assert.match(launches, /^b1 \d+$/m);
  } finally {
    await runBestEffort('node', [skiffCli, 'instance', 'down', configA], { env });
    await runBestEffort('node', [skiffCli, 'instance', 'down', configB], { env });
    await rm(root, { recursive: true, force: true });
  }
});

async function initializeFixture(configPath, basePort, cargoTargetDir, env) {
  await run('node', [skiffCli, 'instance', 'init', configPath], { env });
  const source = await readFile(configPath, 'utf8');
  await writeFile(
    configPath,
    source
      .replace('cargoTargetDir: ../build/cargo-target', `cargoTargetDir: ${cargoTargetDir}`)
      .replace('  base: 4100', `  base: ${basePort}`)
      .replace('  telemetry: managed', '  telemetry: disabled'),
  );
}

async function runtimeStatus(configPath, env) {
  return componentStatus(configPath, 'runtime', env);
}

async function componentStatus(configPath, component, env) {
  const result = JSON.parse(await runCapture(
    'node',
    [skiffCli, 'instance', 'status', configPath, '--json'],
    { env },
  ));
  return result.processes.find(({ name }) => name === component);
}

async function writeFakeCargo(path) {
  await writeExecutable(path, `#!/usr/bin/env node
import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
const binIndex = process.argv.indexOf('--bin');
const bin = process.argv[binIndex + 1];
const manifestIndex = process.argv.indexOf('--manifest-path');
const manifest = process.argv[manifestIndex + 1];
const outputDir = join(process.env.CARGO_TARGET_DIR, 'debug');
mkdirSync(outputDir, { recursive: true });
if (bin === 'runtime') {
  const version = process.env.FAKE_RUNTIME_VERSION || 'unknown';
  const program = \`#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
appendFileSync(process.env.FAKE_RUNTIME_LAUNCH_LOG, \${JSON.stringify(version)} + ' ' + process.pid + '\\\\n');
setInterval(() => {}, 60_000);
\`;
  const output = join(outputDir, process.platform === 'win32' ? 'runtime.exe' : 'runtime');
  writeFileSync(output, program);
  chmodSync(output, 0o755);
} else if (bin === 'skiff-compiler') {
  if (manifest !== join(process.cwd(), 'compiler', 'Cargo.toml')) {
    throw new Error('compiler must be built from the current checkout');
  }
  const output = join(outputDir, process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler');
  writeFileSync(output, '#!/usr/bin/env node\\n');
  chmodSync(output, 0o755);
} else if (bin === 'skiff-router') {
  const output = join(outputDir, process.platform === 'win32' ? 'skiff-router.exe' : 'skiff-router');
  writeFileSync(output, '#!/usr/bin/env node\\nimport { readFileSync } from \\'node:fs\\';\\nimport net from \\'node:net\\';\\nconst source = readFileSync(process.argv[2], \\'utf8\\');\\nconst ports = [...source.matchAll(/^\\\\s+port:\\\\s*(\\\\d+)$/gm)].map((match) => Number(match[1]));\\nfor (const port of new Set(ports)) { net.createServer(() => {}).listen(port, \\'127.0.0.1\\'); }\\nsetInterval(() => {}, 60_000);\\n');
  chmodSync(output, 0o755);
} else {
  throw new Error('unexpected fake cargo bin ' + bin);
}
`);
}

async function writeFakePnpm(path) {
  await writeExecutable(path, `#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import net from 'node:net';
const configIndex = process.argv.indexOf('--config');
const source = readFileSync(process.argv[configIndex + 1], 'utf8');
const ports = [...source.matchAll(/^\\s+port:\\s*(\\d+)$/gm)].map((match) => Number(match[1]));
for (const port of new Set(ports)) {
  net.createServer(() => {}).listen(port, '127.0.0.1');
}
setInterval(() => {}, 60_000);
`);
}

async function writeExecutable(path, contents) {
  await writeFile(path, contents);
  await chmod(path, 0o755);
}

async function reservePortPair(excluded = new Set()) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const candidate = await reserveOnePort();
    if (candidate >= 65535 || excluded.has(candidate) || excluded.has(candidate + 1)) {
      continue;
    }
    if (await canListen(candidate + 1)) {
      return candidate;
    }
  }
  throw new Error('could not reserve an adjacent local port pair');
}

async function reserveOnePort() {
  const server = net.createServer();
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
  const address = server.address();
  await new Promise((resolvePromise) => server.close(resolvePromise));
  return address.port;
}

async function canListen(port) {
  const server = net.createServer();
  try {
    await new Promise((resolvePromise, reject) => {
      server.once('error', reject);
      server.listen(port, '127.0.0.1', resolvePromise);
    });
    return true;
  } catch {
    return false;
  } finally {
    if (server.listening) {
      await new Promise((resolvePromise) => server.close(resolvePromise));
    }
  }
}

function run(command, args, options) {
  return runChild(command, args, { ...options, capture: false });
}

function runCapture(command, args, options) {
  return runChild(command, args, { ...options, capture: true });
}

async function runBestEffort(command, args, options) {
  try {
    await run(command, args, options);
  } catch {
    // The fixture may have failed before its config or processes existed.
  }
}

function runChild(command, args, options) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: skiffRoot,
      env: options.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise(options.capture ? stdout : undefined);
        return;
      }
      reject(new Error(
        `${command} ${args.join(' ')} exited with ${signal ?? code}${stderr || stdout ? `:\n${stderr}${stdout}` : ''}`,
      ));
    });
  });
}
