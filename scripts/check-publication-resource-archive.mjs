#!/usr/bin/env node

import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { collectPackageSourceArchivePaths } from './lib/package-source-archive.mjs';

const root = await mkdtemp(join(tmpdir(), 'skiff-publication-resource-archive-'));

try {
  await checkPackageSourceArchiveIncludesManifestResources();
  console.log('Publication resource archive check passed.');
} finally {
  await rm(root, { recursive: true, force: true });
}

async function checkPackageSourceArchiveIncludesManifestResources() {
  const packageRoot = join(root, 'pkg');
  await mkdir(join(packageRoot, 'prompts'), { recursive: true });
  await mkdir(join(packageRoot, 'src'), { recursive: true });
  await mkdir(join(packageRoot, 'node_modules', 'ignored'), { recursive: true });
  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: example.com/pkg',
      'version: 1.0.0',
      'resources:',
      '  - prompts/system.md',
      '  - prompts/system.md',
      '',
    ].join('\n'),
  );
  await writeFile(join(packageRoot, 'prompts', 'system.md'), 'resource bytes\n');
  await writeFile(join(packageRoot, 'src', 'main.skiff'), 'function main() -> string { return "ok" }\n');
  await writeFile(join(packageRoot, 'node_modules', 'ignored', 'ignored.skiff'), 'ignored\n');

  await expectFailure(
    collectPackageSourceArchivePaths(packageRoot),
    'duplicate path prompts/system.md',
  );

  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: example.com/pkg',
      'version: 1.0.0',
      'resources:',
      '  - prompts/system.md',
      '',
    ].join('\n'),
  );

  const files = await collectPackageSourceArchivePaths(packageRoot);
  const expected = ['package.yml', 'prompts/system.md', 'src/main.skiff'];
  if (JSON.stringify(files) !== JSON.stringify(expected)) {
    throw new Error(`unexpected package source archive files: ${JSON.stringify(files)}`);
  }
}

async function expectFailure(promise, expectedMessagePart) {
  try {
    await promise;
  } catch (error) {
    if (`${error?.message ?? error}`.includes(expectedMessagePart)) {
      return;
    }
    throw error;
  }
  throw new Error(`expected failure containing ${JSON.stringify(expectedMessagePart)}`);
}
