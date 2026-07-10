#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const workspaceRoot = resolve(scriptDir, '..', '..');
const skiffCli = join(scriptDir, 'skiff.mjs');
const keepPackageLiveTemp =
  process.env.SKIFF_PACKAGE_LIVE_KEEP_TEMP === '1' ||
  process.env.SKIFF_PACKAGE_LIVE_KEEP_TEMP === 'true';

try {
  await main();
} catch (error) {
  console.error(`package remote live test failed: ${error?.message || String(error)}`);
  process.exitCode = 1;
}

async function main() {
  const authority = process.env.SKIFF_PACKAGE_TEST_AUTHORITY ?? process.env.SKIFF_PACKAGE_AUTHORITY;
  if (!authority) {
    throw new Error('SKIFF_PACKAGE_TEST_AUTHORITY is required, and must be an organization authority the CLI token can publish under.');
  }

  try {
    await runCli(['package', 'remote', 'ping']);
  } catch (error) {
    throw new Error(`package remote is not reachable; start it or set SKIFF_PACKAGE_REMOTE_URL, then retry. ${error.message}`);
  }

  const stamp = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  const id = `${authority}/sample-${stamp}`;
  const version = `0.0.0-${Date.now()}`;
  const ref = `${id}@${version}`;
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-package-live-'));
  let output;
  try {
    const packageRoot = join(tempRoot, 'pkg');
    const outDir = join(tempRoot, 'out');
    const manifestText = [
      `id: ${id}`,
      `version: ${version}`,
      'resources:',
      '  - prompts/system.md',
      '',
    ].join('\n');
    const resourceText = `live prompt ${stamp}\n`;
    const sourceText = [
      'export function packageLiveValue() -> string {',
      `  return "ok-${stamp}"`,
      '}',
      '',
    ].join('\n');

    await mkdir(join(packageRoot, 'prompts'), { recursive: true });
    await writeFile(join(packageRoot, 'package.yml'), manifestText);
    await writeFile(join(packageRoot, 'main.skiff'), sourceText);
    await writeFile(join(packageRoot, 'prompts', 'system.md'), resourceText);

    const publishResponse = await runCliJson(['package', 'publish', packageRoot, '--wait', '--json']);
    assert(publishResponse?.revision?.revisionId, 'publish --wait did not return a built revision');
    assert(publishResponse?.pointer?.revisionId === publishResponse.revision.revisionId, 'published pointer does not match built revision');

    const resolveResponse = await runCliJson(['package', 'resolve', ref, '--json']);
    assert(resolveResponse?.found === true, 'resolve did not find the published package');
    assert(resolveResponse?.pointer?.revisionId === publishResponse.revision.revisionId, 'resolve pointer does not match publish result');

    const pullResponse = await runCliJson(['package', 'pull', ref, '--out', outDir, '--json']);
    assert(pullResponse?.revision?.revisionId === publishResponse.revision.revisionId, 'pull revision does not match publish result');

    const pulledManifest = await readFile(join(outDir, 'package.yml'), 'utf8');
    const pulledSource = await readFile(join(outDir, 'main.skiff'), 'utf8');
    const pulledResource = await readFile(join(outDir, 'prompts', 'system.md'), 'utf8');
    assert(pulledManifest === manifestText, 'pulled package.yml did not match the published manifest');
    assert(pulledSource === sourceText, 'pulled main.skiff did not match the published source');
    assert(pulledResource === resourceText, 'pulled resource did not match the published resource');

    output = {
      ok: true,
      ref,
      revisionId: publishResponse.revision.revisionId,
      tempPreserved: keepPackageLiveTemp,
      ...(keepPackageLiveTemp
        ? { tempDeleted: false, tempRoot, packageRoot, outDir }
        : { tempDeleted: true }),
    };
  } finally {
    if (!keepPackageLiveTemp) {
      await rm(tempRoot, { recursive: true, force: true });
    }
  }
  console.log(JSON.stringify(output, null, 2));
}

function runCliJson(args) {
  return runCli(args).then(({ stdout }) => {
    try {
      return JSON.parse(stdout);
    } catch (error) {
      throw new Error(`failed to parse JSON from skiff ${args.join(' ')}: ${error.message}\nstdout:\n${stdout}`);
    }
  });
}

function runCli(args) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [skiffCli, ...args], {
      cwd: workspaceRoot,
      env: process.env,
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
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise({ stdout: stdout.trim(), stderr: stderr.trim() });
        return;
      }
      reject(new Error(`skiff ${args.join(' ')} exited with ${signal ?? code}${stderr ? `\nstderr:\n${stderr.trim()}` : ''}${stdout ? `\nstdout:\n${stdout.trim()}` : ''}`));
    });
  });
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
