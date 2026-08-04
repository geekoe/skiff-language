import { readdir } from 'node:fs/promises';
import { extname, join, relative, sep } from 'node:path';

const JAVASCRIPT_EXTENSIONS = new Set(['.cjs', '.js', '.mjs']);

export const DEFAULT_EXCLUDED_DIRECTORIES = new Set([
  '.git',
  '.stack',
  '.skiff-package-store',
  '.turbo',
  'build',
  'coverage',
  'dist',
  'node_modules',
  'target',
  'var',
]);

export async function discoverJavaScriptFiles(
  root,
  { excludedDirectories = DEFAULT_EXCLUDED_DIRECTORIES } = {},
) {
  return discoverFiles(root, {
    excludedDirectories,
    matches: (entry) => JAVASCRIPT_EXTENSIONS.has(extname(entry.name)),
  });
}

export async function discoverScriptTests(root) {
  return discoverDirectFiles(join(root, 'scripts', 'tests'), (entry) =>
    entry.name.endsWith('.test.mjs'),
  ).then((files) => files.map((path) => repoRelative(root, path)));
}

export async function discoverCheckerScripts(root) {
  return discoverDirectFiles(join(root, 'scripts'), (entry) =>
    entry.name.startsWith('check-') && entry.name.endsWith('.mjs'),
  ).then((files) => files.map((path) => repoRelative(root, path)));
}

export async function discoverRuntimeLiveTests(root) {
  const liveRoot = join(root, 'runtime', 'live-tests');
  return discoverFiles(liveRoot, {
    excludedDirectories: new Set(),
    matches: (entry) => entry.name.endsWith('.live.test.skiff'),
  });
}

async function discoverFiles(root, { excludedDirectories, matches }) {
  const files = [];
  await collectFiles(root, files, { excludedDirectories, matches });
  return files.sort((left, right) => left.localeCompare(right));
}

async function collectFiles(directory, files, options) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }

  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (!options.excludedDirectories.has(entry.name)) {
        await collectFiles(path, files, options);
      }
      continue;
    }
    if (entry.isFile() && options.matches(entry)) {
      files.push(path);
    }
  }
}

async function discoverDirectFiles(directory, matches) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return [];
    }
    throw error;
  }
  return entries
    .filter((entry) => entry.isFile() && matches(entry))
    .map((entry) => join(directory, entry.name))
    .sort((left, right) => left.localeCompare(right));
}

export function repoRelative(root, path) {
  return relative(root, path).split(sep).join('/');
}
