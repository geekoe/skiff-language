import { readdir, readFile, stat } from 'node:fs/promises';
import { join, relative } from 'node:path';

import {
  collectTestOnlyModuleFiles,
  loadRuntimeRustSources,
  productionRustViews,
} from './runtime-artifact-boundary-rust-source.mjs';

export async function loadRuntimeExecutionBoundarySources(repoRoot, sourceRoots) {
  const sources = new Map();
  const missingRoots = [];

  for (const sourceRoot of sourceRoots) {
    const absoluteRoot = join(repoRoot, sourceRoot.root);
    if (!(await pathExists(absoluteRoot))) {
      missingRoots.push(sourceRoot);
      continue;
    }
    if (sourceRoot.language === 'rust') {
      const rustSources = await loadRuntimeRustSources(repoRoot, absoluteRoot);
      const testOnlyFiles = collectTestOnlyModuleFiles(rustSources);
      for (const [relPath, source] of rustSources) {
        if (testOnlyFiles.has(relPath)) {
          continue;
        }
        sources.set(relPath, {
          language: 'rust',
          relPath,
          source,
          ...productionRustViews(source),
        });
      }
      continue;
    }
    if (sourceRoot.language === 'typescript') {
      const paths = [];
      await visitTypeScript(absoluteRoot, paths, repoRoot);
      for (const relPath of paths.sort()) {
        const source = await readFile(join(repoRoot, relPath), 'utf8');
        sources.set(relPath, {
          language: 'typescript',
          relPath,
          source,
          ...productionTypeScriptViews(source),
        });
      }
      continue;
    }
    throw new Error(
      `unsupported runtime execution boundary source language ${String(sourceRoot.language)}`,
    );
  }

  return { missingRoots, sources };
}

export function productionTypeScriptViews(source) {
  const commentless = maskTypeScript(source, { comments: true, strings: false });
  return {
    commentless,
    identifiers: maskTypeScript(commentless, { comments: false, strings: true }),
  };
}

async function visitTypeScript(directory, files, repoRoot) {
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await visitTypeScript(path, files, repoRoot);
    } else if (entry.isFile() && /\.(?:c|m)?tsx?$/.test(entry.name)) {
      files.push(normalizePath(relative(repoRoot, path)));
    }
  }
}

function maskTypeScript(source, options) {
  const output = [...source];
  let state = 'code';
  let escaped = false;

  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];

    if (state === 'line-comment') {
      if (char === '\n') {
        state = 'code';
      } else {
        mask(output, index);
      }
      continue;
    }
    if (state === 'block-comment') {
      mask(output, index);
      if (char === '*' && next === '/') {
        mask(output, index + 1);
        index += 1;
        state = 'code';
      }
      continue;
    }
    if (state !== 'code') {
      if (options.strings) {
        mask(output, index);
      }
      if (!escaped && matchesQuoteState(char, state)) {
        state = 'code';
      }
      if (!escaped && char === '\\') {
        escaped = true;
      } else {
        escaped = false;
      }
      continue;
    }

    if (options.comments && char === '/' && next === '/') {
      mask(output, index);
      mask(output, index + 1);
      index += 1;
      state = 'line-comment';
      continue;
    }
    if (options.comments && char === '/' && next === '*') {
      mask(output, index);
      mask(output, index + 1);
      index += 1;
      state = 'block-comment';
      continue;
    }
    if (char === "'" || char === '"' || char === '`') {
      state = char === "'" ? 'single-quote' : char === '"' ? 'double-quote' : 'template';
      escaped = false;
      if (options.strings) {
        mask(output, index);
      }
    }
  }

  return output.join('');
}

function matchesQuoteState(char, state) {
  return (
    (state === 'single-quote' && char === "'")
    || (state === 'double-quote' && char === '"')
    || (state === 'template' && char === '`')
  );
}

function mask(output, index) {
  if (output[index] !== '\n') {
    output[index] = ' ';
  }
}

async function pathExists(path) {
  try {
    await stat(path);
    return true;
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function normalizePath(path) {
  return path.split('\\').join('/');
}
