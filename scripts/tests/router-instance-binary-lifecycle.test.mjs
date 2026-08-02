import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..', '..');
const skiffCli = join(scriptDir, '..', 'skiff.mjs');

test('instance build/up installs the Rust router binary and refresh keeps runtime', {
  skip: process.platform === 'win32',
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-instance-'));
  const fakeBin = join(root, 'fake-bin');
  const invocationLog = join(root, 'cargo-invocations.log');
  const realCargo = which('cargo');
  const base = await reservePortPair();
  const configPath = join(root, 'instance', 'config.yml');
  const env = {
    ...process.env,
    PATH: `${fakeBin}:${process.env.PATH}`,
    REAL_CARGO: realCargo,
    FAKE_CARGO_INVOCATION_LOG: invocationLog,
  };

  await mkdir(fakeBin, { recursive: true });
  await writeFakeCargo(join(fakeBin, 'cargo'));

  try {
    await initializeFixture(configPath, base, env);

    await run('node', [skiffCli, 'instance', 'up', configPath], {
      env: { ...env, FAKE_ROUTER_VERSION: 'v1' },
    });
    const status1 = await status(configPath, env);
    const router1 = componentStatus(status1, 'router');
    const runtime1 = componentStatus(status1, 'runtime');
    assert.equal(router1.category, 'running');
    assert.equal(runtime1.category, 'running');
    const routerPid1 = router1.pid;
    const runtimePid1 = runtime1.pid;
    assert.ok(Number.isInteger(routerPid1) && routerPid1 > 0);
    assert.ok(Number.isInteger(runtimePid1) && runtimePid1 > 0);
    assert.deepEqual(
      router1.managedBinary.currentIdentity.file,
      router1.managedBinary.recordedIdentity.file,
      'installed router binary identity must be recorded',
    );

    // The real binary is bound to the fixture ports by the instance.
    const publicResponse = await httpGet(`http://127.0.0.1:${base}/`);
    assert.match(publicResponse, /^HTTP\/1\.1 200/);
    const controlResponse = await httpGet(`http://127.0.0.1:${base + 1}/__router/health`);
    assert.match(controlResponse, /^HTTP\/1\.1 200/);

    const build2 = JSON.parse(await runCapture(
      'node',
      [skiffCli, 'instance', 'build', configPath],
      { env: { ...env, FAKE_ROUTER_VERSION: 'v2' } },
    ));
    assert.deepEqual(build2.staleProcesses, [{ name: 'router', pid: routerPid1 }]);
    assert.match(build2.recovery, /instance refresh-binaries/);
    assert.ok(build2.router?.path.endsWith('/bin/skiff-router'));
    const stale = await status(configPath, env);
    assert.equal(componentStatus(stale, 'router').category, 'stale-binary');
    assert.equal(componentStatus(stale, 'runtime').pid, runtimePid1);

    await run('node', [skiffCli, 'instance', 'up', configPath], {
      env: { ...env, FAKE_ROUTER_VERSION: 'v2' },
    });
    const refreshed = await status(configPath, env);
    const router2 = componentStatus(refreshed, 'router');
    assert.equal(router2.category, 'running');
    assert.notEqual(router2.pid, routerPid1, 'stale router must restart');
    assert.equal(
      componentStatus(refreshed, 'runtime').pid,
      runtimePid1,
      'refresh Router must not restart Runtime',
    );

    const beforeOnly = await readFile(invocationLog, 'utf8');
    const buildOnly = JSON.parse(await runCapture(
      'node',
      [skiffCli, 'instance', 'build', configPath, '--only', 'router'],
      { env: { ...env, FAKE_ROUTER_VERSION: 'v3' } },
    ));
    assert.equal(buildOnly.runtime, undefined, '--only router must not build runtime');
    assert.equal(
      buildOnly.ecosystemStoreCli,
      undefined,
      '--only router must not build the compiler',
    );
    assert.ok(buildOnly.router?.path.endsWith('/bin/skiff-router'));
    const onlyInvocations = (await readFile(invocationLog, 'utf8'))
      .slice(beforeOnly.length)
      .trim()
      .split('\n')
      .filter(Boolean);
    assert.deepEqual(
      onlyInvocations,
      ['skiff-router'],
      '--only router must invoke exactly the router build',
    );
    const staleOnly = await status(configPath, env);
    assert.equal(componentStatus(staleOnly, 'router').category, 'stale-binary');
    assert.equal(componentStatus(staleOnly, 'runtime').pid, runtimePid1);

    await run('node', [skiffCli, 'instance', 'up', configPath], {
      env: { ...env, FAKE_ROUTER_VERSION: 'v3' },
    });
    const finalStatus = await status(configPath, env);
    assert.equal(componentStatus(finalStatus, 'router').category, 'running');
    assert.equal(
      componentStatus(finalStatus, 'runtime').pid,
      runtimePid1,
      'second Router refresh must still keep Runtime',
    );

    await run('node', [skiffCli, 'instance', 'down', configPath], { env });
    const stopped = await status(configPath, env);
    assert.equal(componentStatus(stopped, 'router').category, 'stopped');
    assert.equal(componentStatus(stopped, 'runtime').category, 'stopped');
  } finally {
    await runBestEffort('node', [skiffCli, 'instance', 'down', configPath], { env });
    await rm(root, { recursive: true, force: true });
  }
});

test('instance build --only router rejects the TS implementation', {
  skip: process.platform === 'win32',
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-only-ts-'));
  const fakeBin = join(root, 'fake-bin');
  const env = {
    ...process.env,
    PATH: `${fakeBin}:${process.env.PATH}`,
  };
  await mkdir(fakeBin, { recursive: true });
  await writeFakeCargo(join(fakeBin, 'cargo'));
  const configPath = join(root, 'instance', 'config.yml');
  try {
    await initializeFixture(configPath, await reservePortPair(), env, 'ts');
    const outcome = await spawnCapture(
      'node',
      [skiffCli, 'instance', 'build', configPath, '--only', 'router'],
      { env },
    );
    assert.notEqual(outcome.code, 0);
    assert.match(outcome.stderr, /requires router\.implementation: rust/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

async function initializeFixture(configPath, base, env, implementation = 'rust') {
  await run('node', [skiffCli, 'instance', 'init', configPath], { env });
  const source = await readFile(configPath, 'utf8');
  await writeFile(
    configPath,
    [
      source
        .replace('cargoTargetDir: ../build/cargo-target', `cargoTargetDir: ${join(skiffRoot, 'build', 'cargo-target')}`)
        .replace('  base: 4100', `  base: ${base}`)
        .replace('  telemetry: managed', '  telemetry: disabled')
        .replace('  mongo: disabled', '  mongo: disabled')
        .replace('  watch: disabled', '  watch: disabled'),
      'router:',
      `  implementation: ${implementation}`,
      '',
    ].join('\n'),
  );
}

async function writeFakeCargo(path) {
  await writeExecutable(path, `#!/usr/bin/env node
import { appendFileSync, chmodSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
const binIndex = process.argv.indexOf('--bin');
const bin = process.argv[binIndex + 1];
const invocationLog = process.env.FAKE_CARGO_INVOCATION_LOG;
if (invocationLog) {
  appendFileSync(invocationLog, bin + '\\n');
}
const outputDir = join(process.env.CARGO_TARGET_DIR, 'debug');
mkdirSync(outputDir, { recursive: true });
if (bin === 'runtime') {
  const output = join(outputDir, process.platform === 'win32' ? 'runtime.exe' : 'runtime');
  writeFileSync(output, '#!/usr/bin/env node\\nsetInterval(() => {}, 60_000);\\n');
  chmodSync(output, 0o755);
} else if (bin === 'skiff-compiler') {
  const output = join(outputDir, process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler');
  writeFileSync(output, '#!/usr/bin/env node\\n');
  chmodSync(output, 0o755);
} else if (bin === 'skiff-router') {
  const result = spawnSync(process.env.REAL_CARGO, process.argv.slice(2), { stdio: 'inherit', env: process.env });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
  const output = join(outputDir, process.platform === 'win32' ? 'skiff-router.exe' : 'skiff-router');
  const version = process.env.FAKE_ROUTER_VERSION || 'base';
  const original = readFileSync(output);
  writeFileSync(output, Buffer.concat([original, Buffer.from('\\nskiff-router-fake-version:' + version + '\\n')]));
  chmodSync(output, 0o755);
} else {
  throw new Error('unexpected fake cargo bin ' + bin);
}
`);
}

async function writeExecutable(path, contents) {
  await writeFile(path, contents);
  await chmod(path, 0o755);
}

async function status(configPath, env) {
  const output = await runCapture(
    'node',
    [skiffCli, 'instance', 'status', configPath, '--json'],
    { env },
  );
  return JSON.parse(output);
}

function componentStatus(instanceStatus, component) {
  const found = instanceStatus.processes.find(({ name }) => name === component);
  assert.ok(found, `status must include ${component}`);
  return found;
}

function which(binary) {
  const result = spawnSync('which', [binary], { encoding: 'utf8' });
  if (result.status !== 0) {
    throw new Error(`${binary} must be available on PATH`);
  }
  return result.stdout.trim();
}

function httpGet(url) {
  return new Promise((resolvePromise, reject) => {
    const request = spawn('node', ['-e', `
      fetch(process.argv[1]).then(async (response) => {
        console.log('HTTP/1.1 ' + response.status + ' ' + response.statusText);
        console.log(await response.text());
      }).catch((error) => { console.error(error); process.exitCode = 1; });
    `, url], { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    request.stdout.setEncoding('utf8');
    request.stderr.setEncoding('utf8');
    request.stdout.on('data', (chunk) => { stdout += chunk; });
    request.stderr.on('data', (chunk) => { stderr += chunk; });
    request.once('error', reject);
    request.once('close', (code) => {
      if (code === 0) {
        resolvePromise(stdout);
      } else {
        reject(new Error(`HTTP probe failed (${code}): ${stderr}`));
      }
    });
  });
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
  const server = createServer();
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
  const address = server.address();
  await new Promise((resolvePromise) => server.close(resolvePromise));
  return address.port;
}

async function canListen(port) {
  const server = createServer();
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

function spawnCapture(command, args, options) {
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
    child.once('error', reject);
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, stdout, stderr });
    });
  });
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
    child.once('error', reject);
    child.once('close', (code, signal) => {
      if (options.capture) {
        if (code === 0) {
          resolvePromise(stdout);
          return;
        }
        reject(new Error(
          `${command} ${args.join(' ')} exited with ${signal ?? code}${stderr || stdout ? `:\n${stderr}${stdout}` : ''}`,
        ));
        return;
      }
      if (code === 0) {
        resolvePromise({ code, signal, stdout, stderr });
        return;
      }
      reject(new Error(
        `${command} ${args.join(' ')} exited with ${signal ?? code}${stderr || stdout ? `:\n${stderr}${stdout}` : ''}`,
      ));
    });
  });
}
