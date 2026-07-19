import { readdir, readFile } from 'node:fs/promises';
import { basename, dirname, extname, join, relative } from 'node:path';

export async function loadRuntimeRustSources(repoRoot, runtimeRoot) {
  const relPaths = [];
  await visit(runtimeRoot, relPaths, repoRoot);
  return new Map(
    await Promise.all(
      relPaths
        .sort()
        .map(async (relPath) => [relPath, await readFile(join(repoRoot, relPath), 'utf8')]),
    ),
  );
}

export function productionRustViews(source) {
  const productionSource = stripCfgTestItems(source);
  const commentless = stripRustComments(productionSource);
  return {
    commentless,
    identifiers: maskRustStringLiterals(commentless),
  };
}

export function collectTestOnlyModuleFiles(sources) {
  const testOnly = new Set();
  for (const [relPath, source] of sources) {
    const code = maskRustStringLiterals(stripRustComments(source));
    for (const moduleName of cfgTestExternalModuleNames(code)) {
      const child = resolveModuleFile(relPath, moduleName, sources);
      if (child) {
        testOnly.add(child);
      }
    }
  }

  const pending = [...testOnly];
  while (pending.length > 0) {
    const relPath = pending.pop();
    const source = sources.get(relPath);
    if (!source) {
      continue;
    }
    const code = maskRustStringLiterals(stripRustComments(source));
    for (const moduleName of externalModuleNames(code)) {
      const child = resolveModuleFile(relPath, moduleName, sources);
      if (child && !testOnly.has(child)) {
        testOnly.add(child);
        pending.push(child);
      }
    }
  }
  return testOnly;
}

export function externalModuleNames(code) {
  const names = [];
  const regexp = /(?:^|[;}])\s*(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/gm;
  for (const match of code.matchAll(regexp)) {
    names.push(match[1]);
  }
  return names;
}

export function resolveModuleFile(parentRelPath, moduleName, files) {
  const parentName = basename(parentRelPath);
  const base = ['lib.rs', 'main.rs', 'mod.rs'].includes(parentName)
    ? dirname(parentRelPath)
    : join(dirname(parentRelPath), parentName.slice(0, -extname(parentName).length));
  for (const candidate of [join(base, `${moduleName}.rs`), join(base, moduleName, 'mod.rs')]) {
    const normalized = normalizePath(candidate);
    if (files.has(normalized)) {
      return normalized;
    }
  }
  return undefined;
}

export function lineNumberAt(text, index = 0) {
  return text.slice(0, index).split('\n').length;
}

function cfgTestExternalModuleNames(code) {
  const names = [];
  const regexp = /#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/g;
  for (const match of code.matchAll(regexp)) {
    names.push(match[1]);
  }
  return names;
}

function stripCfgTestItems(source) {
  const lexical = maskRustStringLiterals(stripRustComments(source));
  const ranges = [];
  const regexp = /#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/g;
  for (const match of lexical.matchAll(regexp)) {
    const range = rustItemRange(lexical, match.index, match.index + match[0].length);
    if (range) {
      ranges.push(range);
    }
  }
  let result = source;
  for (const range of ranges.sort((left, right) => right.start - left.start)) {
    const replacement = result.slice(range.start, range.end).replace(/[^\n]/g, ' ');
    result = result.slice(0, range.start) + replacement + result.slice(range.end);
  }
  return result;
}

function rustItemRange(lexical, start, afterCfg) {
  let index = skipWhitespace(lexical, afterCfg);
  while (lexical.startsWith('#[', index)) {
    const close = matchingDelimiterIndex(lexical, index + 1, '[', ']');
    if (close === -1) {
      return undefined;
    }
    index = skipWhitespace(lexical, close + 1);
  }
  const semicolon = lexical.indexOf(';', index);
  const brace = lexical.indexOf('{', index);
  if (semicolon !== -1 && (brace === -1 || semicolon < brace)) {
    return { start, end: semicolon + 1 };
  }
  if (brace !== -1) {
    const close = matchingDelimiterIndex(lexical, brace, '{', '}');
    if (close !== -1) {
      return { start, end: close + 1 };
    }
  }
  const newline = lexical.indexOf('\n', index);
  return { start, end: newline === -1 ? lexical.length : newline + 1 };
}

function matchingDelimiterIndex(text, openIndex, open, close) {
  let depth = 0;
  for (let index = openIndex; index < text.length; index += 1) {
    if (text[index] === open) {
      depth += 1;
    } else if (text[index] === close) {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function stripRustComments(source) {
  const output = [...source];
  let index = 0;
  let blockDepth = 0;
  while (index < source.length) {
    if (blockDepth > 0) {
      if (source.startsWith('/*', index)) {
        mask(output, index, 2);
        blockDepth += 1;
        index += 2;
      } else if (source.startsWith('*/', index)) {
        mask(output, index, 2);
        blockDepth -= 1;
        index += 2;
      } else {
        mask(output, index, 1);
        index += 1;
      }
      continue;
    }
    const stringEnd = rustStringEnd(source, index);
    if (stringEnd !== undefined) {
      index = stringEnd;
      continue;
    }
    if (source.startsWith('//', index)) {
      while (index < source.length && source[index] !== '\n') {
        mask(output, index, 1);
        index += 1;
      }
      continue;
    }
    if (source.startsWith('/*', index)) {
      mask(output, index, 2);
      blockDepth = 1;
      index += 2;
      continue;
    }
    index += 1;
  }
  return output.join('');
}

function maskRustStringLiterals(source) {
  const output = [...source];
  let index = 0;
  while (index < source.length) {
    const end = rustStringEnd(source, index);
    if (end === undefined) {
      index += 1;
      continue;
    }
    for (let cursor = index; cursor < end; cursor += 1) {
      mask(output, cursor, 1);
    }
    index = end;
  }
  return output.join('');
}

function rustStringEnd(source, index) {
  const raw = /^(?:b?r|br)(#+)?"/.exec(source.slice(index));
  if (raw) {
    const hashes = raw[1] ?? '';
    const terminator = `"${hashes}`;
    const end = source.indexOf(terminator, index + raw[0].length);
    return end === -1 ? source.length : end + terminator.length;
  }
  const prefixLength = source.startsWith('b"', index) ? 2 : source[index] === '"' ? 1 : 0;
  if (prefixLength === 0) {
    return undefined;
  }
  let escaped = false;
  for (let cursor = index + prefixLength; cursor < source.length; cursor += 1) {
    if (!escaped && source[cursor] === '"') {
      return cursor + 1;
    }
    if (!escaped && source[cursor] === '\\') {
      escaped = true;
    } else {
      escaped = false;
    }
  }
  return source.length;
}

async function visit(directory, files, repoRoot) {
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await visit(path, files, repoRoot);
    } else if (entry.isFile() && entry.name.endsWith('.rs')) {
      files.push(normalizePath(relative(repoRoot, path)));
    }
  }
}

function mask(output, index, length) {
  for (let offset = 0; offset < length; offset += 1) {
    if (output[index + offset] !== '\n') {
      output[index + offset] = ' ';
    }
  }
}

function skipWhitespace(text, start) {
  let index = start;
  while (index < text.length && /\s/.test(text[index])) {
    index += 1;
  }
  return index;
}

function normalizePath(path) {
  return path.split('\\').join('/');
}
