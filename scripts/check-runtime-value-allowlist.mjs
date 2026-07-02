#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const defaultAllowlistPath = join(
  repoRoot,
  'doc',
  'implementation',
  'runtime-value-convergence',
  'serde-json-value-allowlist.json',
);

main().catch((error) => {
  console.error(error.stack ?? error.message);
  process.exitCode = 1;
});

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printUsage();
    return;
  }

  const allowlist = await loadAllowlist(options.allowlistPath);
  const compiled = compileAllowlist(allowlist);
  const result = {
    scannedFiles: 0,
    rawHits: 0,
    allowedHits: 0,
    deniedHits: 0,
    unlistedHits: 0,
    violations: [],
  };

  for (const file of await collectRuntimeRustFiles()) {
    const source = await readFile(file.absPath, 'utf8');
    if (isCfgTestSupportOnlyFile(source)) {
      continue;
    }
    result.scannedFiles += 1;

    const productionSource = stripCfgTestModules(stripCfgTestSupportItems(source));
    const maskedSource = maskRustCommentsAndStrings(productionSource);
    const aliases = serdeJsonValueAliases(maskedSource);
    const hits = serdeJsonValueHits(file.relPath, productionSource, maskedSource, aliases);

    for (const hit of hits) {
      result.rawHits += 1;
      const deny = matchingEntry(compiled.deny, hit);
      if (deny) {
        result.deniedHits += 1;
        result.violations.push({ ...hit, kind: 'DENY', rule: deny.id, reason: deny.reason });
        continue;
      }

      const allow = matchingEntry(compiled.entries, hit);
      if (allow) {
        result.allowedHits += 1;
        continue;
      }

      result.unlistedHits += 1;
      result.violations.push({
        ...hit,
        kind: 'UNLISTED',
        rule: null,
        reason: 'No legitimate A/B/C allowlist entry matched this serde_json::Value usage.',
      });
    }
  }

  result.violations.sort((left, right) => {
    const pathOrder = left.relPath.localeCompare(right.relPath);
    return pathOrder === 0 ? left.line - right.line : pathOrder;
  });

  if (options.json) {
    console.log(JSON.stringify(result, null, 2));
  } else {
    printTextResult(result, options.limit);
  }

  if (result.violations.length > 0 && !options.allowNonempty) {
    process.exitCode = 1;
  }
}

function parseArgs(args) {
  const options = {
    allowNonempty: false,
    allowlistPath: defaultAllowlistPath,
    help: false,
    json: false,
    limit: 120,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
    } else if (arg === '--allow-nonempty') {
      options.allowNonempty = true;
    } else if (arg === '--json') {
      options.json = true;
    } else if (arg === '--allowlist') {
      index += 1;
      if (!args[index]) {
        throw new Error('--allowlist requires a path');
      }
      options.allowlistPath = resolve(repoRoot, args[index]);
    } else if (arg.startsWith('--allowlist=')) {
      options.allowlistPath = resolve(repoRoot, arg.slice('--allowlist='.length));
    } else if (arg === '--limit') {
      index += 1;
      if (!args[index]) {
        throw new Error('--limit requires a number');
      }
      options.limit = parseLimit(args[index]);
    } else if (arg.startsWith('--limit=')) {
      options.limit = parseLimit(arg.slice('--limit='.length));
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  return options;
}

function parseLimit(value) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`invalid --limit value: ${value}`);
  }
  return parsed;
}

function printUsage() {
  console.log(`Usage: node scripts/check-runtime-value-allowlist.mjs [options]

Scans production runtime Rust code for serde_json::Value usages not covered by
the T0 legitimate Value allowlist.

Options:
  --allowlist <path>  Allowlist JSON path.
  --allow-nonempty    Print differences but exit 0.
  --json              Emit machine-readable JSON.
  --limit <n>         Limit printed violations. 0 means all. Default: 120.
  -h, --help          Show this help.
`);
}

async function loadAllowlist(path) {
  const parsed = JSON.parse(await readFile(path, 'utf8'));
  validateAllowlist(parsed, path);
  return parsed;
}

function validateAllowlist(allowlist, path) {
  if (!allowlist || typeof allowlist !== 'object') {
    throw new Error(`${path}: allowlist must be an object`);
  }
  if (!Array.isArray(allowlist.entries)) {
    throw new Error(`${path}: entries must be an array`);
  }
  if (allowlist.deny !== undefined && !Array.isArray(allowlist.deny)) {
    throw new Error(`${path}: deny must be an array when present`);
  }
  const ids = new Set();
  for (const entry of allowlist.entries) {
    validateEntry(entry, path, 'entries');
    if (!['A', 'B', 'C'].includes(entry.category)) {
      throw new Error(`${path}: entry ${entry.id} category must be A, B, or C`);
    }
    if (ids.has(entry.id)) {
      throw new Error(`${path}: duplicate entry id ${entry.id}`);
    }
    ids.add(entry.id);
  }
  for (const entry of allowlist.deny ?? []) {
    validateEntry(entry, path, 'deny');
    if (ids.has(entry.id)) {
      throw new Error(`${path}: duplicate entry id ${entry.id}`);
    }
    ids.add(entry.id);
  }
}

function validateEntry(entry, path, section) {
  if (!entry || typeof entry !== 'object') {
    throw new Error(`${path}: ${section} entries must be objects`);
  }
  if (!entry.id || typeof entry.id !== 'string') {
    throw new Error(`${path}: ${section} entry is missing id`);
  }
  const hasPath =
    typeof entry.path === 'string' ||
    Array.isArray(entry.paths) ||
    typeof entry.pathPrefix === 'string' ||
    Array.isArray(entry.pathPrefixes);
  if (!hasPath) {
    throw new Error(`${path}: ${section} entry ${entry.id} must declare path(s) or pathPrefix(es)`);
  }
  for (const field of ['linePatterns', 'denyLinePatterns']) {
    if (entry[field] !== undefined && !Array.isArray(entry[field])) {
      throw new Error(`${path}: ${section} entry ${entry.id} ${field} must be an array`);
    }
  }
  if (!entry.reason || typeof entry.reason !== 'string') {
    throw new Error(`${path}: ${section} entry ${entry.id} must have a reason`);
  }
}

function compileAllowlist(allowlist) {
  return {
    entries: allowlist.entries.map(compileEntry),
    deny: (allowlist.deny ?? []).map(compileEntry),
  };
}

function compileEntry(entry) {
  return {
    ...entry,
    pathPatterns: [...stringArray(entry.path), ...stringArray(entry.paths)].map(globToRegExp),
    pathPrefixes: [...stringArray(entry.pathPrefix), ...stringArray(entry.pathPrefixes)].map(
      normalizePath,
    ),
    linePatterns: (entry.linePatterns ?? []).map((pattern) => new RegExp(pattern)),
    denyLinePatterns: (entry.denyLinePatterns ?? []).map((pattern) => new RegExp(pattern)),
  };
}

function stringArray(value) {
  if (value === undefined) {
    return [];
  }
  if (typeof value === 'string') {
    return [value];
  }
  if (Array.isArray(value)) {
    return value.map((item) => {
      if (typeof item !== 'string') {
        throw new Error('path entries must be strings');
      }
      return item;
    });
  }
  throw new Error('path entries must be strings or arrays of strings');
}

function globToRegExp(pattern) {
  const normalized = normalizePath(pattern);
  const escaped = normalized
    .split('**')
    .map((part) =>
      part
        .replace(/[.+^${}()|[\]\\]/g, '\\$&')
        .replace(/\*/g, '[^/]*'),
    )
    .join('.*');
  return new RegExp(`^${escaped}$`);
}

async function collectRuntimeRustFiles() {
  const files = [];
  await collectRustFiles(join(repoRoot, 'runtime'), files);
  return files.filter((file) => isProductionRuntimeRustFile(file.relPath)).sort((left, right) => {
    return left.relPath.localeCompare(right.relPath);
  });
}

async function collectRustFiles(directory, files) {
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    if (entry.name === 'target') {
      continue;
    }
    const absPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      await collectRustFiles(absPath, files);
      continue;
    }
    if (!entry.isFile() || !entry.name.endsWith('.rs')) {
      continue;
    }
    files.push({
      absPath,
      relPath: normalizePath(relative(repoRoot, absPath)),
    });
  }
}

function isProductionRuntimeRustFile(relPath) {
  if (!relPath.startsWith('runtime/') || !relPath.endsWith('.rs')) {
    return false;
  }
  if (relPath.startsWith('runtime/benches/') || relPath.startsWith('runtime/live-tests/')) {
    return false;
  }
  if (relPath.split('/').includes('tests')) {
    return false;
  }
  const name = basename(relPath);
  return name !== 'tests.rs' && name !== 'test_support.rs';
}

function serdeJsonValueAliases(maskedSource) {
  const aliases = new Set();
  for (const match of maskedSource.matchAll(/\buse\s+serde_json\s*::\s*Value(?:\s+as\s+([A-Za-z_]\w*))?\s*;/g)) {
    aliases.add(match[1] ?? 'Value');
  }
  for (const match of maskedSource.matchAll(/\buse\s+serde_json\s*::\s*\{([^;]+)\}\s*;/gs)) {
    for (const part of match[1].split(',')) {
      const valueMatch = /^Value(?:\s+as\s+([A-Za-z_]\w*))?$/.exec(part.trim());
      if (valueMatch) {
        aliases.add(valueMatch[1] ?? 'Value');
      }
    }
  }
  return aliases;
}

function serdeJsonValueHits(relPath, source, maskedSource, aliases) {
  const hits = [];
  const sourceLines = source.split('\n');
  const maskedLines = maskedSource.split('\n');
  for (let index = 0; index < maskedLines.length; index += 1) {
    const maskedLine = maskedLines[index];
    if (isSerdeJsonValueImportLine(maskedLine)) {
      continue;
    }
    if (
      /^\s*type\s+Value\s*=/.test(maskedLine) &&
      !/\bserde_json\s*::\s*Value\b/.test(maskedLine)
    ) {
      continue;
    }
    const tokens = new Set();
    if (/\bserde_json\s*::\s*Value\b/.test(maskedLine)) {
      tokens.add('serde_json::Value');
    }
    for (const alias of aliases) {
      if (lineContainsAliasValueUsage(maskedLine, alias)) {
        tokens.add(alias);
      }
    }
    if (tokens.size === 0) {
      continue;
    }
    hits.push({
      relPath,
      line: index + 1,
      tokens: [...tokens].sort(),
      sourceLine: (sourceLines[index] ?? '').trim(),
    });
  }
  return hits;
}

function isSerdeJsonValueImportLine(line) {
  return /\buse\s+serde_json\s*::/.test(line) && /\bValue\b/.test(line);
}

function lineContainsAliasValueUsage(line, alias) {
  const regexp = new RegExp(`\\b${escapeRegExp(alias)}\\b`, 'g');
  let match;
  while ((match = regexp.exec(line)) !== null) {
    if (line.slice(Math.max(0, match.index - 2), match.index) === '::') {
      continue;
    }
    return true;
  }
  return false;
}

function matchingEntry(entries, hit) {
  return entries.find((entry) => entryMatches(entry, hit));
}

function entryMatches(entry, hit) {
  const pathMatches =
    entry.pathPatterns.some((pattern) => pattern.test(hit.relPath)) ||
    entry.pathPrefixes.some((prefix) => hit.relPath.startsWith(prefix));
  if (!pathMatches) {
    return false;
  }
  if (entry.denyLinePatterns.some((pattern) => pattern.test(hit.sourceLine))) {
    return false;
  }
  if (entry.linePatterns.length === 0) {
    return true;
  }
  return entry.linePatterns.some((pattern) => pattern.test(hit.sourceLine));
}

function printTextResult(result, limit) {
  console.log('Runtime serde_json::Value allowlist check');
  console.log(`  scanned production Rust files: ${result.scannedFiles}`);
  console.log(`  raw serde_json::Value hit lines: ${result.rawHits}`);
  console.log(`  allowed hit lines: ${result.allowedHits}`);
  console.log(`  denied known-debt hit lines: ${result.deniedHits}`);
  console.log(`  unlisted hit lines: ${result.unlistedHits}`);
  console.log(`  non-allowlisted hit lines: ${result.violations.length}`);

  const counts = countViolations(result.violations);
  if (counts.length > 0) {
    console.log('');
    console.log('By rule:');
    for (const count of counts) {
      console.log(`  ${String(count.count).padStart(4)} ${count.label}`);
    }
  }

  if (result.violations.length === 0) {
    return;
  }

  const visible = limit === 0 ? result.violations : result.violations.slice(0, limit);
  console.log('');
  for (const violation of visible) {
    const rule = violation.rule ? ` ${violation.rule}` : '';
    console.log(`${violation.kind}${rule} ${violation.relPath}:${violation.line}`);
    console.log(`  tokens: ${violation.tokens.join(', ')}`);
    console.log(`  line: ${violation.sourceLine}`);
    console.log(`  reason: ${violation.reason}`);
  }
  if (visible.length < result.violations.length) {
    console.log('');
    console.log(`... ${result.violations.length - visible.length} more hit line(s); rerun with --limit=0 to show all.`);
  }
}

function countViolations(violations) {
  const counts = new Map();
  for (const violation of violations) {
    const label = violation.rule ? `${violation.kind} ${violation.rule}` : violation.kind;
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([label, count]) => ({ label, count }))
    .sort((left, right) => right.count - left.count || left.label.localeCompare(right.label));
}

function stripCfgTestSupportItems(text) {
  let output = text;
  let searchIndex = 0;
  while (searchIndex < output.length) {
    const masked = maskRustCommentsAndStrings(output);
    const attrMatch = /#\[\s*cfg\s*\((?<condition>(?:[^\]\(\)]|\([^)]*\))*)\)\s*\]/g;
    attrMatch.lastIndex = searchIndex;
    const match = attrMatch.exec(masked);
    if (!match) {
      break;
    }
    if (!isTestSupportCfgCondition(match.groups?.condition ?? '')) {
      searchIndex = match.index + 1;
      continue;
    }

    const removal = cfgTestItemRange(masked, match.index, match[0].length);
    if (removal === undefined) {
      searchIndex = match.index + 1;
      continue;
    }
    const replacement = output.slice(removal.start, removal.end).replace(/[^\n]/g, ' ');
    output = output.slice(0, removal.start) + replacement + output.slice(removal.end);
    searchIndex = removal.start + replacement.length;
  }
  return output;
}

function stripCfgTestModules(text) {
  const lines = text.split('\n');
  const maskedLines = maskRustCommentsAndStrings(text).split('\n');
  for (let index = 0; index < maskedLines.length; index += 1) {
    if (!/^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*$/.test(maskedLines[index])) {
      continue;
    }
    let modLine = index + 1;
    while (modLine < maskedLines.length && maskedLines[modLine].trim() === '') {
      modLine += 1;
    }
    if (!/^\s*mod\s+tests\b/.test(maskedLines[modLine] ?? '')) {
      continue;
    }
    let endLine = modLine;
    if (!(maskedLines[modLine] ?? '').includes(';')) {
      let depth = 0;
      let seenOpen = false;
      for (; endLine < maskedLines.length; endLine += 1) {
        for (const char of maskedLines[endLine]) {
          if (char === '{') {
            depth += 1;
            seenOpen = true;
          } else if (char === '}') {
            depth -= 1;
          }
        }
        if (seenOpen && depth === 0) {
          break;
        }
      }
      if (endLine >= maskedLines.length) {
        endLine = maskedLines.length - 1;
      }
    }
    for (let line = index; line <= endLine; line += 1) {
      lines[line] = lines[line].replace(/[^\r]/g, ' ');
    }
    index = endLine;
  }
  return lines.join('\n');
}

function isCfgTestSupportOnlyFile(text) {
  const masked = maskRustCommentsAndStrings(text);
  const innerCfg = /#!\[\s*cfg\s*\((?<condition>(?:[^\]\(\)]|\([^)]*\))*)\)\s*\]/g;
  let match;
  while ((match = innerCfg.exec(masked)) !== null) {
    if (isTestSupportCfgCondition(match.groups?.condition ?? '')) {
      return true;
    }
  }
  return false;
}

function isTestSupportCfgCondition(condition) {
  if (/\bfeature\s*=\s*"test-support"/.test(condition)) {
    return true;
  }
  if (/\bnot\s*\(\s*test\s*\)/.test(condition)) {
    return false;
  }
  return /\btest\b/.test(condition);
}

function cfgTestItemRange(maskedText, attrIndex, attrLength) {
  let index = attrIndex + attrLength;
  while (index < maskedText.length && /\s/.test(maskedText[index])) {
    index += 1;
  }

  const nextSemicolon = maskedText.indexOf(';', index);
  const nextBrace = maskedText.indexOf('{', index);
  if (nextSemicolon !== -1 && (nextBrace === -1 || nextSemicolon < nextBrace)) {
    return { start: attrIndex, end: nextSemicolon + 1 };
  }

  if (nextBrace !== -1) {
    const closeBrace = matchingBraceIndex(maskedText, nextBrace);
    if (closeBrace !== -1) {
      return { start: attrIndex, end: closeBrace + 1 };
    }
  }

  const nextLine = maskedText.indexOf('\n', index);
  if (nextLine !== -1) {
    return { start: attrIndex, end: nextLine + 1 };
  }
  return { start: attrIndex, end: maskedText.length };
}

function matchingBraceIndex(text, openIndex) {
  let depth = 0;
  for (let index = openIndex; index < text.length; index += 1) {
    if (text[index] === '{') {
      depth += 1;
    } else if (text[index] === '}') {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function maskRustCommentsAndStrings(text) {
  let output = '';
  let index = 0;
  while (index < text.length) {
    const char = text[index];
    const next = text[index + 1];

    if (char === '/' && next === '/') {
      const end = text.indexOf('\n', index);
      const stop = end === -1 ? text.length : end;
      output += text.slice(index, stop).replace(/[^\n]/g, ' ');
      index = stop;
      continue;
    }
    if (char === '/' && next === '*') {
      const end = rustBlockCommentEnd(text, index);
      output += text.slice(index, end).replace(/[^\n]/g, ' ');
      index = end;
      continue;
    }
    const raw = rawStringEnd(text, index);
    if (raw !== null) {
      output += text.slice(index, raw).replace(/[^\n]/g, ' ');
      index = raw;
      continue;
    }
    if (char === '"' || (char === 'b' && next === '"')) {
      const start = char === 'b' ? index + 1 : index;
      const end = quotedStringEnd(text, start);
      output += text.slice(index, end).replace(/[^\n]/g, ' ');
      index = end;
      continue;
    }
    if (char === '\'' && isLikelyCharLiteral(text, index)) {
      const end = charLiteralEnd(text, index);
      output += text.slice(index, end).replace(/[^\n]/g, ' ');
      index = end;
      continue;
    }

    output += char;
    index += 1;
  }
  return output;
}

function rustBlockCommentEnd(text, start) {
  let depth = 0;
  let index = start;
  while (index < text.length) {
    if (text[index] === '/' && text[index + 1] === '*') {
      depth += 1;
      index += 2;
      continue;
    }
    if (text[index] === '*' && text[index + 1] === '/') {
      depth -= 1;
      index += 2;
      if (depth === 0) {
        return index;
      }
      continue;
    }
    index += 1;
  }
  return text.length;
}

function rawStringEnd(text, start) {
  let index = start;
  if (text[index] === 'b') {
    index += 1;
  }
  if (text[index] !== 'r') {
    return null;
  }
  index += 1;
  let hashes = 0;
  while (text[index] === '#') {
    hashes += 1;
    index += 1;
  }
  if (text[index] !== '"') {
    return null;
  }
  index += 1;
  const terminator = `"${'#'.repeat(hashes)}`;
  const end = text.indexOf(terminator, index);
  return end === -1 ? text.length : end + terminator.length;
}

function quotedStringEnd(text, quoteIndex) {
  let index = quoteIndex + 1;
  while (index < text.length) {
    if (text[index] === '\\') {
      index += 2;
      continue;
    }
    if (text[index] === '"') {
      return index + 1;
    }
    index += 1;
  }
  return text.length;
}

function isLikelyCharLiteral(text, start) {
  const next = text[start + 1];
  if (next === undefined || /[A-Za-z_]/.test(next)) {
    return false;
  }
  return text.indexOf('\'', start + 1) !== -1;
}

function charLiteralEnd(text, start) {
  let index = start + 1;
  while (index < text.length) {
    if (text[index] === '\\') {
      index += 2;
      continue;
    }
    if (text[index] === '\'') {
      return index + 1;
    }
    index += 1;
  }
  return text.length;
}

function normalizePath(path) {
  return path.split('\\').join('/');
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
