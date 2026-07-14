import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { isolatedInstanceOperations } from '../lib/isolated-test-runtime-instance.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const skiffCli = join(root, 'scripts', 'skiff.mjs');
const instanceCli = join(root, 'scripts', 'skiff-instance.mjs');

test('missing tar is reported through the safe outcome failure before remote I/O', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-missing-tar-'));
  const packageRoot = join(fixture, 'package');
  const emptyBin = join(fixture, 'empty-bin');
  try {
    await mkdir(packageRoot, { recursive: true });
    await mkdir(emptyBin);
    await writeFile(join(packageRoot, 'package.yml'), [
      'id: example.com/missing-tar',
      'version: 0.1.0',
      '',
    ].join('\n'));
    await writeFile(join(packageRoot, 'main.skiff'), 'export function value() -> string { return "ok" }\n');
    const result = await runProcess(process.execPath, [
      skiffCli,
      'package',
      'publish',
      packageRoot,
    ], {
      env: { ...process.env, HOME: fixture, PATH: emptyBin },
    });
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /failed to spawn tar: ENOENT/);
    assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, /spawnargs|cause/);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('instance status treats missing lsof as unavailable and missing ps as process fallback', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-instance-command-outcome-'));
  const configPath = join(fixture, 'instance', 'config.yml');
  const emptyBin = join(fixture, 'empty-bin');
  const lsofBin = join(fixture, 'lsof-bin');
  try {
    await mkdir(emptyBin);
    await mkdir(lsofBin);
    const initialized = await runProcess(process.execPath, [instanceCli, 'init', configPath]);
    assert.equal(initialized.code, 0, initialized.stderr);

    const unavailableResult = await runProcess(process.execPath, [
      instanceCli,
      'status',
      configPath,
      '--json',
    ], { env: { ...process.env, PATH: emptyBin } });
    assert.equal(unavailableResult.code, 0, unavailableResult.stderr);
    const unavailable = JSON.parse(unavailableResult.stdout);
    assert.equal(unavailable.listenerDiscovery.available, false);
    assert.ok(unavailable.listenerDiscovery.errors.every((message) =>
      message.includes('failed to spawn lsof: ENOENT')));
    assert.doesNotMatch(unavailableResult.stdout, /spawnargs|cause/);

    const fakePid = process.pid;
    const lsofPath = join(lsofBin, 'lsof');
    await writeFile(lsofPath, `#!${process.execPath}\nprocess.stdout.write(process.env.FAKE_LISTENER_PID + '\\n');\n`);
    await chmod(lsofPath, 0o755);
    const fallbackResult = await runProcess(process.execPath, [
      instanceCli,
      'status',
      configPath,
      '--json',
    ], {
      env: {
        ...process.env,
        PATH: lsofBin,
        FAKE_LISTENER_PID: String(fakePid),
      },
    });
    assert.equal(fallbackResult.code, 0, fallbackResult.stderr);
    const fallback = JSON.parse(fallbackResult.stdout);
    assert.equal(fallback.listenerDiscovery.available, true);
    const listeners = fallback.processes
      .flatMap((processStatus) => processStatus.ports)
      .flatMap((port) => port.listeners)
      .filter((listener) => listener.pid === fakePid);
    assert.ok(listeners.length > 0);
    assert.ok(listeners.every((listener) =>
      listener.ppid === null && listener.pgid === null && listener.command === ''));
    assert.doesNotMatch(fallbackResult.stdout, /spawnargs|cause/);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('isolated status checked adapter rejects nonzero and invalid JSON before cleanup verification', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-isolated-status-command-'));
  const scriptsRoot = join(fixture, 'scripts');
  const instancePath = join(scriptsRoot, 'skiff-instance.mjs');
  const operations = isolatedInstanceOperations({
    skiffRoot: fixture,
    baseEnv: process.env,
  });
  try {
    await mkdir(scriptsRoot, { recursive: true });
    await writeFile(instancePath, [
      "process.stdout.write('status stdout');",
      "process.stderr.write('status stderr');",
      'process.exit(9);',
    ].join('\n'));
    await assert.rejects(
      operations.verifyInstanceStopped('/tmp/fake-config.yml'),
      (error) => {
        assert.match(error.message, /node exited with 9/);
        assert.match(error.message, /stderr:\nstatus stderr/);
        assert.match(error.message, /stdout:\nstatus stdout/);
        assert.equal(Object.hasOwn(error, 'cause'), false);
        return true;
      },
    );

    await writeFile(instancePath, "process.stdout.write('not-json');\n");
    await assert.rejects(
      operations.verifyInstanceStopped('/tmp/fake-config.yml'),
      SyntaxError,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

test('runtime and compiler DAG adapters promote spawn failure before status interpretation', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-dag-missing-cargo-'));
  try {
    for (const script of [
      'check-runtime-crate-dag.mjs',
      'check-compiler-crate-dag.mjs',
    ]) {
      const result = await runProcess(process.execPath, [join(root, 'scripts', script)], {
        env: { ...process.env, PATH: fixture },
      });
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /ENOENT/);
      assert.doesNotMatch(result.stderr, /spawnargs|cause/);
    }
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

function runProcess(command, args, options = {}) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? root,
      env: options.env ?? process.env,
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
