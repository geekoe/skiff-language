import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import {
  buildBinaryArgs,
  componentInfo,
  loadProcessYml,
  mergeEnv,
  parseCliArgs,
  pickBinaryPath,
  pidAlive,
  readPidFile,
  resolveComponentBinary,
  sha256OfFile,
  tailLines,
} from '../skiff-process.mjs';

const scriptsDir = dirname(dirname(fileURLToPath(import.meta.url)));
const cliPath = join(scriptsDir, 'skiff-process.mjs');

const FAKE_BINARY_SOURCE = `#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const configPath = process.argv[2];
const runDir = path.dirname(configPath);
const component = path.basename(configPath, '.yml');
fs.writeFileSync(path.join(runDir, component + '.pid'), String(process.pid));
fs.writeFileSync(path.join(runDir, 'marker.txt'), String(process.env.FAKE_MARKER ?? 'no-marker'));
process.stdout.write('fake ' + component + ' started pid=' + process.pid + '\\n');
process.stderr.write('fake ' + component + ' stderr ready\\n');
const keepAlive = setInterval(() => {}, 1000);
process.on('SIGTERM', () => {
  clearInterval(keepAlive);
  process.exit(0);
});
`;

function runCli(args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [cliPath, ...args], {
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('close', (code) => resolvePromise({ code, stdout, stderr }));
    child.on('error', reject);
  });
}

async function makeFakeBinary(root) {
  const binaryPath = join(root, 'fake-binary');
  await writeFile(binaryPath, FAKE_BINARY_SOURCE, { mode: 0o755 });
  return binaryPath;
}

async function writeRunDir(root, { component = 'router', binaryPath = null, env = null } = {}) {
  const runDir = join(root, `${component}-run`);
  await mkdir(runDir, { recursive: true });
  await writeFile(join(runDir, `${component}.yml`), `runDir: ${runDir}\nprofile: debug\n`);
  if (binaryPath !== null) {
    let processYml = `binary: ${binaryPath}\n`;
    if (env !== null) {
      processYml += `env:\n${Object.entries(env)
        .map(([key, value]) => `  ${key}: ${value}`)
        .join('\n')}\n`;
    }
    await writeFile(join(runDir, 'process.yml'), processYml);
  }
  return runDir;
}

async function cleanupRunDir(root, runDir, component) {
  const pid = await readPidFile(runDir, component);
  if (pid !== null && pidAlive(pid)) {
    try {
      process.kill(pid, 'SIGKILL');
    } catch {
      // already gone
    }
  }
  await rm(root, { recursive: true, force: true });
}

function sha256Of(text) {
  return createHash('sha256').update(text).digest('hex');
}

test('pure binary resolution: process.yml binary wins, then cargo target, then build/bin', async (t) => {
  assert.equal(pickBinaryPath(null, null, null), null);
  assert.equal(pickBinaryPath('/a/bin', null, null), '/a/bin');
  assert.equal(pickBinaryPath(null, '/cargo/target/debug/router', null), '/cargo/target/debug/router');
  assert.equal(pickBinaryPath(null, null, '/skiff/build/bin/router'), '/skiff/build/bin/router');
  assert.equal(pickBinaryPath('/a/bin', '/cargo/bin', '/build/bin'), '/a/bin');

  const root = await mkdtemp(join(tmpdir(), 'skiff-process-resolve-'));
  t.after(() => rm(root, { recursive: true, force: true }));

  const skiffRoot = join(root, 'checkout');
  const runDir = join(root, 'run');
  await mkdir(skiffRoot, { recursive: true });
  await mkdir(runDir, { recursive: true });
  await writeFile(join(runDir, 'runtime.yml'), 'profile: debug\n');

  const targetDir = join(root, 'cargo-target');
  const buildBin = join(skiffRoot, 'build', 'bin');
  await mkdir(join(targetDir, 'debug'), { recursive: true });
  await mkdir(buildBin, { recursive: true });
  await writeFile(join(targetDir, 'debug', 'runtime'), 'cargo-binary');
  await writeFile(join(buildBin, 'runtime'), 'build-binary');

  const fakeLoader = async ({ env }) => ({
    target_directory: env.CARGO_TARGET_DIR ?? targetDir,
  });

  let resolved = await resolveComponentBinary({
    component: 'runtime',
    runDir,
    skiffRoot,
    loadMetadata: fakeLoader,
  });
  assert.equal(resolved.source, 'cargo');
  assert.equal(resolved.binary, join(targetDir, 'debug', 'runtime'));

  await rm(join(targetDir, 'debug', 'runtime'));
  resolved = await resolveComponentBinary({
    component: 'runtime',
    runDir,
    skiffRoot,
    loadMetadata: fakeLoader,
  });
  assert.equal(resolved.source, 'build');
  assert.equal(resolved.binary, join(buildBin, 'runtime'));

  await rm(buildBin, { recursive: true, force: true });
  resolved = await resolveComponentBinary({
    component: 'runtime',
    runDir,
    skiffRoot,
    loadMetadata: fakeLoader,
  });
  assert.equal(resolved, null);
});

test('pure binary resolution: process.yml binary wins and CARGO_TARGET_DIR env reaches metadata loader', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-process-resolve-env-'));
  t.after(() => rm(root, { recursive: true, force: true }));

  const skiffRoot = join(root, 'checkout');
  const runDir = join(root, 'run');
  await mkdir(skiffRoot, { recursive: true });
  await mkdir(runDir, { recursive: true });
  await writeFile(join(runDir, 'runtime.yml'), 'profile: debug\n');

  const seenEnv = [];
  const fakeLoader = async ({ env }) => {
    seenEnv.push(env);
    return { target_directory: env.CARGO_TARGET_DIR ?? join(root, 'fallback-target') };
  };

  const explicitBinary = join(root, 'explicit-binary');
  await writeFile(explicitBinary, '#!/usr/bin/env node\nsetInterval(() => {}, 1000);\n', { mode: 0o755 });
  await writeFile(join(runDir, 'process.yml'), `binary: ${explicitBinary}\n`);
  let resolved = await resolveComponentBinary({
    component: 'runtime',
    runDir,
    skiffRoot,
    env: { CARGO_TARGET_DIR: join(root, 'env-target') },
    loadMetadata: fakeLoader,
  });
  assert.equal(resolved.source, 'process.yml');
  assert.equal(resolved.binary, explicitBinary);
  assert.equal(seenEnv.length, 0);

  await mkdir(join(root, 'env-target', 'debug'), { recursive: true });
  await writeFile(join(root, 'env-target', 'debug', 'runtime'), 'env-binary');
  await rm(join(runDir, 'process.yml'));
  resolved = await resolveComponentBinary({
    component: 'runtime',
    runDir,
    skiffRoot,
    env: { CARGO_TARGET_DIR: join(root, 'env-target') },
    loadMetadata: fakeLoader,
  });
  assert.equal(resolved.source, 'cargo');
  assert.equal(resolved.binary, join(root, 'env-target', 'debug', 'runtime'));
  assert.equal(seenEnv.length, 1);
  assert.equal(seenEnv[0].CARGO_TARGET_DIR, join(root, 'env-target'));
});

test('pure helpers: process.yml parsing, env merge, binary args, cli args', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-process-helpers-'));
  t.after(() => rm(root, { recursive: true, force: true }));

  const runDir = join(root, 'run');
  await mkdir(runDir, { recursive: true });
  await writeFile(
    join(runDir, 'process.yml'),
    `binary: /abs/path/to/bin\nenv:\n  SKIFF_OVERRIDE: from-yml\n  PORT: 4100\n`,
  );
  const processYml = await loadProcessYml(runDir);
  assert.deepEqual(processYml, { binary: '/abs/path/to/bin', env: { SKIFF_OVERRIDE: 'from-yml', PORT: '4100' } });

  const merged = mergeEnv({ SKIFF_OVERRIDE: 'base', KEEP: 'yes' }, processYml.env);
  assert.equal(merged.KEEP, 'yes');
  assert.equal(merged.SKIFF_OVERRIDE, 'from-yml');

  assert.deepEqual(componentInfo('router'), { crate: 'skiff-router', bin: 'skiff-router', manifest: 'router/Cargo.toml' });
  assert.deepEqual(componentInfo('runtime'), { crate: 'runtime', bin: 'runtime', manifest: 'runtime/Cargo.toml' });
  assert.throws(() => componentInfo('watch'), /unknown component/);

  assert.deepEqual(buildBinaryArgs(componentInfo('router'), '/run/router.yml'), ['/run/router.yml']);
  assert.deepEqual(buildBinaryArgs(componentInfo('runtime'), '/run/runtime.yml'), ['/run/runtime.yml']);

  assert.deepEqual(parseCliArgs(['router', 'status', '--dir', '/run']), {
    component: 'router',
    action: 'status',
    dir: '/run',
  });
  assert.throws(() => parseCliArgs(['router', 'status']), /--dir/);
  assert.throws(() => parseCliArgs(['router', 'status', '--dir', '/run', '--extra', 'x']), /unexpected argument/);
  assert.throws(() => parseCliArgs(['watch', 'status', '--dir', '/run']), /unknown component/);
  assert.throws(() => parseCliArgs(['router', 'frobnicate', '--dir', '/run']), /unknown action/);

  assert.deepEqual(tailLines('a\nb\nc\n', 2), ['c', '']);
  assert.equal(pidAlive(null), false);
  assert.equal(pidAlive(0), false);
  assert.equal(pidAlive(999_999_999), false);
  assert.equal(pidAlive(process.pid), true);
});

test('start/status/duplicate-start/stop/logs lifecycle through the CLI', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-process-lifecycle-'));
  const binaryPath = await makeFakeBinary(root);
  const runDir = await writeRunDir(root, {
    binaryPath,
    env: { FAKE_MARKER: 'marker-value' },
  });
  const component = 'router';
  t.after(() => cleanupRunDir(root, runDir, component));

  let started = await runCli([component, 'start', '--dir', runDir]);
  assert.equal(started.code, 0, started.stderr);
  const startMatch = started.stdout.match(/^started router \(pid (\d+)\)$/m);
  assert.ok(startMatch, started.stdout);
  const pid = Number(startMatch[1]);
  assert.equal(pidAlive(pid), true);
  assert.equal(await readPidFile(runDir, component), pid);
  assert.equal((await readFile(join(runDir, 'marker.txt'), 'utf8')).trim(), 'marker-value');

  const outLog = await readFile(join(runDir, 'router.out.log'), 'utf8');
  assert.match(outLog, new RegExp(`fake router started pid=${pid}`));
  const errLog = await readFile(join(runDir, 'router.err.log'), 'utf8');
  assert.match(errLog, /fake router stderr ready/);

  const status = await runCli([component, 'status', '--dir', runDir]);
  assert.equal(status.code, 0, status.stderr);
  const parsedStatus = JSON.parse(status.stdout);
  assert.equal(parsedStatus.component, component);
  assert.equal(parsedStatus.runDir, resolve(runDir));
  assert.equal(parsedStatus.pid, pid);
  assert.equal(parsedStatus.alive, true);
  assert.equal(parsedStatus.binary, binaryPath);
  assert.equal(parsedStatus.binarySha256, sha256Of(FAKE_BINARY_SOURCE));

  const duplicate = await runCli([component, 'start', '--dir', runDir]);
  assert.equal(duplicate.code, 1);
  assert.match(duplicate.stderr, /already running \(pid \d+\)/);
  assert.equal(await readPidFile(runDir, component), pid);

  const filler = Array.from({ length: 250 }, (_, index) => `filler-line-${index + 1}`).join('\n');
  await writeFile(join(runDir, 'router.out.log'), `\n${filler}\n`, { flag: 'a' });
  const logs = await runCli([component, 'logs', '--dir', runDir]);
  assert.equal(logs.code, 0, logs.stderr);
  assert.match(logs.stdout, /filler-line-100/);
  assert.doesNotMatch(logs.stdout, /filler-line-1\n/);
  assert.match(logs.stdout, /\[err\] fake router stderr ready/);

  const stopped = await runCli([component, 'stop', '--dir', runDir]);
  assert.equal(stopped.code, 0, stopped.stderr);
  assert.match(stopped.stdout, new RegExp(`^stopped router \\(pid ${pid}\\)$`, 'm'));
  await delay(200);
  assert.equal(pidAlive(pid), false);
  assert.equal(await readPidFile(runDir, component), pid);

  const staleStatus = await runCli([component, 'status', '--dir', runDir]);
  const staleParsed = JSON.parse(staleStatus.stdout);
  assert.equal(staleParsed.pid, pid);
  assert.equal(staleParsed.alive, false);
  assert.equal(staleParsed.binary, binaryPath);

  const restarted = await runCli([component, 'start', '--dir', runDir]);
  assert.equal(restarted.code, 0, restarted.stderr);
  const restartedMatch = restarted.stdout.match(/^started router \(pid (\d+)\)$/m);
  assert.ok(restartedMatch, restarted.stdout);
  const newPid = Number(restartedMatch[1]);
  assert.notEqual(newPid, pid);
  assert.equal(pidAlive(newPid), true);
  assert.equal(await readPidFile(runDir, component), newPid);

  const finalStop = await runCli([component, 'stop', '--dir', runDir]);
  assert.equal(finalStop.code, 0, finalStop.stderr);
  await delay(200);
  assert.equal(pidAlive(newPid), false);
});

test('restart stops the live process and starts a new one', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-process-restart-'));
  const binaryPath = await makeFakeBinary(root);
  const runDir = await writeRunDir(root, { component: 'runtime', binaryPath });
  const component = 'runtime';
  t.after(() => cleanupRunDir(root, runDir, component));

  const started = await runCli([component, 'start', '--dir', runDir]);
  assert.equal(started.code, 0, started.stderr);
  const firstPid = Number(started.stdout.match(/^started runtime \(pid (\d+)\)$/m)[1]);

  const restarted = await runCli([component, 'restart', '--dir', runDir]);
  assert.equal(restarted.code, 0, restarted.stderr);
  assert.match(restarted.stdout, new RegExp(`^stopped runtime \\(pid ${firstPid}\\)$`, 'm'));
  const secondPid = Number(restarted.stdout.match(/^started runtime \(pid (\d+)\)$/m)[1]);
  assert.notEqual(secondPid, firstPid);
  await delay(200);
  assert.equal(pidAlive(firstPid), false);
  assert.equal(pidAlive(secondPid), true);
  assert.equal(await readPidFile(runDir, component), secondPid);

  const stopped = await runCli([component, 'stop', '--dir', runDir]);
  assert.equal(stopped.code, 0, stopped.stderr);
  await delay(200);
  assert.equal(pidAlive(secondPid), false);
});

test('stop on a dir with no pid prints not running; status/logs tolerate missing files', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-process-empty-'));
  const runDir = join(root, 'never-created-run');
  t.after(() => rm(root, { recursive: true, force: true }));

  const stopped = await runCli(['router', 'stop', '--dir', runDir]);
  assert.equal(stopped.code, 0, stopped.stderr);
  assert.match(stopped.stdout, /not running/);

  const status = await runCli(['router', 'status', '--dir', runDir]);
  assert.equal(status.code, 0, status.stderr);
  const parsed = JSON.parse(status.stdout);
  assert.equal(parsed.component, 'router');
  assert.equal(parsed.runDir, resolve(runDir));
  assert.equal(parsed.pid, null);
  assert.equal(parsed.alive, false);
  assert.equal(Object.hasOwn(parsed, 'binary'), false);

  const logs = await runCli(['router', 'logs', '--dir', runDir]);
  assert.equal(logs.code, 0, logs.stderr);
  assert.match(logs.stdout, /no log files/);

  const missingDir = await runCli(['router', 'status', '--dir', '/nonexistent/run-dir']);
  assert.equal(missingDir.code, 0, missingDir.stderr);
  const missingParsed = JSON.parse(missingDir.stdout);
  assert.equal(missingParsed.pid, null);
  assert.equal(missingParsed.alive, false);
});

test('start fails without a config file and on missing binary', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-process-config-'));
  const runDir = join(root, 'run');
  await mkdir(runDir, { recursive: true });
  t.after(() => rm(root, { recursive: true, force: true }));

  await writeFile(join(runDir, 'process.yml'), 'binary: /definitely/not/a/binary\n');
  const noConfig = await runCli(['router', 'start', '--dir', runDir]);
  assert.equal(noConfig.code, 1);
  assert.match(noConfig.stderr, /config file not found/);

  await writeFile(join(runDir, 'router.yml'), 'profile: debug\n');
  const noBinary = await runCli(['router', 'start', '--dir', runDir]);
  assert.equal(noBinary.code, 1);
  assert.match(noBinary.stderr, /binary not found/);
});
