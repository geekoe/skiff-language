import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const checker = join(repoRoot, 'scripts', 'check-artifact-identity-single-source.mjs');

test('single-source self-test locks exact callable deployment input and one owner', async () => {
  const result = await runChecker(['--self-test']);
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /Artifact identity single-source self-test passed/);
});

function runChecker(args) {
  return new Promise((resolveResult, reject) => {
    const child = spawn(process.execPath, [checker, ...args], {
      cwd: repoRoot,
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
    child.once('error', reject);
    child.once('close', (code) => {
      resolveResult({ code, stdout, stderr });
    });
  });
}
