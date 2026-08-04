import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { isolatedInstanceOperations } from '../lib/isolated-test-runtime-instance.mjs';
import {
  captureIsolatedTestConfig,
  claimIsolatedTestWorkspace,
} from '../lib/isolated-test-runtime-workspace.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
test('isolated status checked adapter rejects nonzero and invalid JSON before cleanup verification', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'skiff-isolated-status-command-'));
  const scriptsRoot = join(fixture, 'scripts');
  const instancePath = join(scriptsRoot, 'skiff-instance.mjs');
  const configPath = join(fixture, 'instance', 'instance.yml');
  const operations = isolatedInstanceOperations({
    skiffRoot: fixture,
    baseEnv: process.env,
  });
  try {
    let ownershipReceipt = await claimIsolatedTestWorkspace(fixture);
    await mkdir(scriptsRoot, { recursive: true });
    await mkdir(dirname(configPath), { recursive: true });
    await writeFile(configPath, 'profile: isolated-test\n');
    ownershipReceipt = await captureIsolatedTestConfig(ownershipReceipt, configPath);
    await writeFile(instancePath, [
      "process.stdout.write('status stdout');",
      "process.stderr.write('status stderr');",
      'process.exit(9);',
    ].join('\n'));
    await assert.rejects(
      operations.verifyInstanceStopped(ownershipReceipt),
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
      operations.verifyInstanceStopped(ownershipReceipt),
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
