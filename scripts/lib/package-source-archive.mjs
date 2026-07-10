import { readdir } from 'node:fs/promises';
import { join, relative, sep } from 'node:path';
import { validatedManifestResourceArchivePaths } from './publication-resources.mjs';

export async function collectPackageSourceArchivePaths(root) {
  const files = ['package.yml'];
  files.push(...await validatedManifestResourceArchivePaths(root, join(root, 'package.yml')));
  await collectSkiffFilePaths(root, root, files);
  return [...new Set(files)].sort((left, right) => left.localeCompare(right));
}

async function collectSkiffFilePaths(root, directory, files) {
  const entries = (await readdir(directory, { withFileTypes: true }))
    .sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if (entry.name === 'package.yml') {
      continue;
    }
    const entryPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (shouldSkipSourceArchiveDirectory(entry.name)) {
        continue;
      }
      await collectSkiffFilePaths(root, entryPath, files);
      continue;
    }
    if (!entry.isFile() || !entry.name.endsWith('.skiff')) {
      continue;
    }
    const relPath = relative(root, entryPath).split(sep).join('/');
    safeArchivePathParts(relPath);
    files.push(relPath);
  }
}

function shouldSkipSourceArchiveDirectory(name) {
  const lower = name.toLowerCase();
  return name.startsWith('.')
    || lower === 'node_modules'
    || lower === 'build'
    || lower === 'dist'
    || lower === 'out'
    || lower === 'target'
    || lower === 'coverage'
    || lower === 'tmp'
    || lower === 'temp'
    || lower === 'cache'
    || lower.includes('cache');
}

function safeArchivePathParts(relPath) {
  if (typeof relPath !== 'string' || relPath.length === 0 || relPath.includes('\0') || relPath.includes('\n') || relPath.includes('\r')) {
    throw new Error(`unsafe archive path ${relPath}`);
  }
  if (relPath.startsWith('/') || relPath.includes('\\')) {
    throw new Error(`unsafe archive path ${relPath}`);
  }
  const parts = relPath.split('/');
  if (parts.some((part) => part.length === 0 || part === '.' || part === '..')) {
    throw new Error(`unsafe archive path ${relPath}`);
  }
  return parts;
}
