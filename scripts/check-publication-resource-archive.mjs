#!/usr/bin/env node

import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { collectPackageSourceArchivePaths } from './lib/package-source-archive.mjs';
import { discoverDeclaredResourceFiles } from './lib/publication-resources.mjs';

const root = await mkdtemp(join(tmpdir(), 'skiff-publication-resource-archive-'));

try {
  await checkPackageSourceArchiveIncludesManifestResources();
  await checkExternalDocumentsAreNotResourceManifests();
  console.log('Publication resource archive check passed.');
} finally {
  await rm(root, { recursive: true, force: true });
}

async function checkPackageSourceArchiveIncludesManifestResources() {
  const packageRoot = join(root, 'pkg');
  const outsideRoot = join(root, 'outside');
  await mkdir(outsideRoot, { recursive: true });
  await mkdir(join(packageRoot, 'nested'), { recursive: true });
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
  await writeFile(join(packageRoot, 'nested', 'package.yml'), 'id: nested\n');
  await writeFile(join(packageRoot, 'src', 'main.skiff'), 'function main() -> string { return "ok" }\n');
  await writeFile(join(packageRoot, 'node_modules', 'ignored', 'ignored.skiff'), 'ignored\n');
  await writeFile(join(outsideRoot, 'leak.txt'), 'leak\n');
  await symlink(outsideRoot, join(packageRoot, 'link'), 'dir');

  await expectFailure(
    collectPackageSourceArchivePaths(packageRoot),
    'duplicate path prompts/system.md',
  );

  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: example.com/pkg',
      'version: 1.0.0',
      'resources: ["nested/package.yml"]',
      '',
    ].join('\n'),
  );

  await expectFailure(
    collectPackageSourceArchivePaths(packageRoot),
    'control file',
  );

  for (const externalFile of ['http.yml', 'websocket.yml']) {
    await writeFile(
      join(packageRoot, 'package.yml'),
      [
        'id: example.com/pkg',
        'version: 1.0.0',
        `resources: ["${externalFile}"]`,
        '',
      ].join('\n'),
    );
    await writeFile(
      join(packageRoot, externalFile),
      externalFile === 'http.yml' ? '{}\n' : 'path: /socket\n',
    );
    await expectFailure(
      collectPackageSourceArchivePaths(packageRoot),
      'control file',
    );
  }

  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: example.com/pkg',
      'version: 1.0.0',
      'resources: ["link/leak.txt"]',
      '',
    ].join('\n'),
  );

  await expectFailure(
    collectPackageSourceArchivePaths(packageRoot),
    'symlink',
  );

  await writeFile(
    join(packageRoot, 'package.yml'),
    [
      'id: example.com/pkg',
      'version: 1.0.0',
      'resources: [',
      '  "prompts/system.md",',
      ']',
      '',
    ].join('\n'),
  );
  await writeFile(join(packageRoot, 'http.yml'), '{}\n');
  await writeFile(join(packageRoot, 'websocket.yml'), 'path: /socket\n');

  const files = await collectPackageSourceArchivePaths(packageRoot);
  const expected = ['package.yml', 'prompts/system.md', 'src/main.skiff'];
  if (JSON.stringify(files) !== JSON.stringify(expected)) {
    throw new Error(`unexpected package source archive files: ${JSON.stringify(files)}`);
  }
}

async function checkExternalDocumentsAreNotResourceManifests() {
  const packageRoot = join(root, 'manifest-discovery');
  await mkdir(join(packageRoot, 'resources'), { recursive: true });
  await writeFile(
    join(packageRoot, 'package.yml'),
    'id: example.com/discovery\nversion: 1.0.0\nresources: ["resources/package.txt"]\n',
  );
  await writeFile(
    join(packageRoot, 'service.yml'),
    'id: example.com/discovery\nresources: ["resources/service.txt"]\n',
  );
  await writeFile(
    join(packageRoot, 'http.yml'),
    'resources: ["resources/http.txt"]\n',
  );
  await writeFile(
    join(packageRoot, 'websocket.yml'),
    'path: /socket\nresources: ["resources/websocket.txt"]\n',
  );
  for (const name of ['package', 'service', 'http', 'websocket']) {
    await writeFile(join(packageRoot, 'resources', `${name}.txt`), `${name}\n`);
  }

  const discovered = (await discoverDeclaredResourceFiles(
    packageRoot,
    new Set(['package.yml', 'service.yml', 'http.yml', 'websocket.yml']),
  )).sort((left, right) => left.localeCompare(right));
  const expected = [
    join(packageRoot, 'resources', 'package.txt'),
    join(packageRoot, 'resources', 'service.txt'),
  ].sort((left, right) => left.localeCompare(right));
  if (JSON.stringify(discovered) !== JSON.stringify(expected)) {
    throw new Error(
      `external documents were treated as resource manifests: ${JSON.stringify(discovered)}`,
    );
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
