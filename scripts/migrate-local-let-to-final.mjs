#!/usr/bin/env node

import { createHash, randomBytes } from 'node:crypto';
import { lstat, readFile, realpath, rename, rm, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { captureCheckedCommand } from './lib/command-execution.mjs';
import { writeJsonAtomic } from './lib/source-key.mjs';

export const LOCAL_LET_PATTERN = /^([\t ]*)let(?=[\t ]+[A-Za-z_][A-Za-z0-9_]*[\t ]*(?::|=))/gm;

const DEFAULT_EXPECTED_COUNTS = Object.freeze({
  skiff: 258,
  'skiff-packages': 313,
  internals: 2551,
});

const REPO_DEFINITIONS = Object.freeze([
  { name: 'skiff', option: '--skiff-root' },
  { name: 'skiff-packages', option: '--skiff-packages-root' },
  { name: 'internals', option: '--internals-root' },
]);

const MANIFEST_SCHEMA = 'skiff-let-to-final-migration-v1';
const EMBEDDED_MANIFEST_SCHEMA = 'skiff-let-to-final-embedded-v1';
const EMBEDDED_ALLOWLIST = Object.freeze(['vscode/scripts/test-grammar.mjs']);
const UTF8_BOM = Buffer.from([0xef, 0xbb, 0xbf]);

export function parseMigrationArgs(argv) {
  const options = {
    roots: {},
    expects: {},
    mode: null,
    manifestOut: null,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];

    if (isModeArgument(argument)) {
      setMode(options, argument);
      continue;
    }

    if (argument === '--expect' || argument.startsWith('--expect=')) {
      let expectValue;
      if (argument === '--expect') {
        index += 1;
        if (index >= argv.length) {
          throw new Error('--expect requires a value');
        }
        expectValue = argv[index];
      } else {
        expectValue = argumentValue(argument, '--expect');
      }
      const parsed = parseExpect(expectValue);
      if (options.expects[parsed.repo] !== undefined) {
        throw new Error(`duplicate --expect for ${parsed.repo}`);
      }
      options.expects[parsed.repo] = parsed.count;
      continue;
    }

    if (argument === '--manifest-out' || argument.startsWith('--manifest-out=')) {
      let manifestValue;
      if (argument === '--manifest-out') {
        index += 1;
        if (index >= argv.length) {
          throw new Error('--manifest-out requires a path');
        }
        manifestValue = argv[index];
      } else {
        manifestValue = argumentValue(argument, '--manifest-out');
      }
      if (options.manifestOut !== null) {
        throw new Error('duplicate --manifest-out');
      }
      options.manifestOut = manifestValue;
      continue;
    }

    const repoDefinition = REPO_DEFINITIONS.find((definition) => (
      argument === definition.option || argument.startsWith(`${definition.option}=`)
    ));
    if (repoDefinition) {
      const value = argumentValue(argument, repoDefinition.option);
      if (value === null) {
        index += 1;
        if (index >= argv.length) {
          throw new Error(`${repoDefinition.option} requires a path`);
        }
        setRoot(options, repoDefinition.name, argv[index]);
      } else {
        setRoot(options, repoDefinition.name, value);
      }
      continue;
    }

    throw new Error(`unexpected argument: ${argument}`);
  }

  if (options.mode === null) {
    throw new Error('expected one of --check, --write, --embedded-fixtures-check, or --embedded-fixtures-write');
  }
  if (options.manifestOut === null) {
    throw new Error('--manifest-out is required');
  }
  for (const definition of REPO_DEFINITIONS) {
    if (options.roots[definition.name] === undefined) {
      throw new Error(`${definition.option} is required`);
    }
  }
  return options;
}

function isModeArgument(argument) {
  return argument === '--check'
    || argument === '--write'
    || argument === '--embedded-fixtures-check'
    || argument === '--embedded-fixtures-write';
}

function setMode(options, mode) {
  if (options.mode !== null) {
    throw new Error(`cannot combine modes ${options.mode} and ${mode}`);
  }
  options.mode = mode.slice(2);
}

function argumentValue(argument, prefix) {
  if (argument === prefix) {
    return null;
  }
  if (argument.startsWith(`${prefix}=`)) {
    return argument.slice(prefix.length + 1);
  }
  return null;
}

function setRoot(options, name, root) {
  if (options.roots[name] !== undefined) {
    throw new Error(`duplicate root for ${name}`);
  }
  if (!path.isAbsolute(root)) {
    throw new Error(`${name} root must be an absolute path: ${root}`);
  }
  options.roots[name] = root;
}

function parseExpect(value) {
  const match = /^([A-Za-z][A-Za-z0-9-]*)=(\d+)$/.exec(value);
  if (!match) {
    throw new Error(`invalid --expect value ${JSON.stringify(value)}; expected <repo>=<count>`);
  }
  const repo = match[1];
  if (!REPO_DEFINITIONS.some((definition) => definition.name === repo)) {
    throw new Error(`unknown --expect repo ${repo}; expected ${REPO_DEFINITIONS.map((definition) => definition.name).join(', ')}`);
  }
  return { repo, count: Number(match[2]) };
}

export function collectRegexLocalLetRanges(source) {
  const regex = new RegExp(LOCAL_LET_PATTERN.source, 'gm');
  const ranges = [];
  let match;
  while ((match = regex.exec(source)) !== null) {
    const start = match.index + match[1].length;
    ranges.push([start, start + 3]);
  }
  return ranges;
}

export function scanSkiffLocalLetRanges(source) {
  const ranges = [];
  const length = source.length;
  let position = 0;
  let lineStartIndentOnly = true;

  const newlineLength = (index) => (
    source[index] === '\r' && source[index + 1] === '\n' ? 2 : 1
  );

  const skipLineComment = (index) => {
    while (index < length && source[index] !== '\n' && source[index] !== '\r') {
      index += 1;
    }
    return index;
  };

  const skipBlockComment = (index) => {
    let scan = index + 2;
    while (scan + 1 < length) {
      if (source[scan] === '*' && source[scan + 1] === '/') {
        return scan + 2;
      }
      if (source[scan] === '\r' || source[scan] === '\n') {
        lineStartIndentOnly = true;
        scan += newlineLength(scan);
      } else {
        scan += 1;
      }
    }
    throw new Error('unterminated block comment while scanning Skiff source');
  };

  const skipQuoted = (quote, index) => {
    let scan = index + 1;
    while (scan < length) {
      const character = source[scan];
      if (character === '\\') {
        if (scan + 1 >= length) {
          throw new Error('unterminated escape while scanning Skiff string');
        }
        const escaped = source[scan + 1];
        if (escaped === '\r' || escaped === '\n') {
          lineStartIndentOnly = true;
          scan += newlineLength(scan + 1);
        } else {
          scan += 2;
        }
        continue;
      }
      if (character === quote) {
        return scan + 1;
      }
      if (character === '\r' || character === '\n') {
        lineStartIndentOnly = true;
        scan += newlineLength(scan);
      } else {
        scan += 1;
      }
    }
    throw new Error(`unterminated ${quote} string while scanning Skiff source`);
  };

  while (position < length) {
    const character = source[position];

    if (character === '\r' || character === '\n') {
      lineStartIndentOnly = true;
      position += newlineLength(position);
      continue;
    }

    if (lineStartIndentOnly && (character === ' ' || character === '\t')) {
      position += 1;
      continue;
    }

    if (source.startsWith('//', position)) {
      position = skipLineComment(position);
      continue;
    }

    if (source.startsWith('/*', position)) {
      position = skipBlockComment(position);
      lineStartIndentOnly = false;
      continue;
    }

    if (character === '"' || character === "'" || character === '`') {
      position = skipQuoted(character, position);
      lineStartIndentOnly = false;
      continue;
    }

    if (
      lineStartIndentOnly
      && source.startsWith('let', position)
      && isLocalLetAt(source, position)
    ) {
      ranges.push([position, position + 3]);
      lineStartIndentOnly = false;
      position += 3;
      continue;
    }

    if (lineStartIndentOnly) {
      lineStartIndentOnly = false;
    }
    position += 1;
  }

  return ranges;
}

export function maskSkiffCommentsAndStrings(source) {
  const characters = source.split('');
  const length = source.length;
  let position = 0;
  let lineStartIndentOnly = true;

  const newlineLength = (index) => (
    source[index] === '\r' && source[index + 1] === '\n' ? 2 : 1
  );

  const maskRange = (start, end) => {
    for (let index = start; index < end; index += 1) {
      if (characters[index] !== '\n' && characters[index] !== '\r') {
        characters[index] = '\0';
      }
    }
  };

  const skipLineComment = (index) => {
    let scan = index;
    while (scan < length && source[scan] !== '\n' && source[scan] !== '\r') {
      scan += 1;
    }
    maskRange(index, scan);
    return scan;
  };

  const skipBlockComment = (index) => {
    let scan = index + 2;
    while (scan + 1 < length) {
      if (source[scan] === '*' && source[scan + 1] === '/') {
        maskRange(index, scan + 2);
        return scan + 2;
      }
      if (source[scan] === '\r' || source[scan] === '\n') {
        lineStartIndentOnly = true;
        scan += newlineLength(scan);
      } else {
        scan += 1;
      }
    }
    throw new Error('unterminated block comment while masking Skiff source');
  };

  const skipQuoted = (quote, index) => {
    let scan = index + 1;
    while (scan < length) {
      const character = source[scan];
      if (character === '\\') {
        if (scan + 1 >= length) {
          throw new Error('unterminated escape while masking Skiff string');
        }
        const escaped = source[scan + 1];
        if (escaped === '\r' || escaped === '\n') {
          lineStartIndentOnly = true;
          scan += newlineLength(scan + 1);
        } else {
          scan += 2;
        }
        continue;
      }
      if (character === quote) {
        maskRange(index, scan + 1);
        return scan + 1;
      }
      if (character === '\r' || character === '\n') {
        lineStartIndentOnly = true;
        scan += newlineLength(scan);
      } else {
        scan += 1;
      }
    }
    throw new Error(`unterminated ${quote} string while masking Skiff source`);
  };

  while (position < length) {
    const character = source[position];
    if (character === '\r' || character === '\n') {
      lineStartIndentOnly = true;
      position += newlineLength(position);
      continue;
    }
    if (lineStartIndentOnly && (character === ' ' || character === '\t')) {
      position += 1;
      continue;
    }
    if (source.startsWith('//', position)) {
      position = skipLineComment(position);
      continue;
    }
    if (source.startsWith('/*', position)) {
      position = skipBlockComment(position);
      lineStartIndentOnly = false;
      continue;
    }
    if (character === '"' || character === "'" || character === '`') {
      position = skipQuoted(character, position);
      lineStartIndentOnly = false;
      continue;
    }
    if (lineStartIndentOnly) {
      lineStartIndentOnly = false;
    }
    position += 1;
  }
  return characters.join('');
}

function isLocalLetAt(source, position) {
  if (!source.startsWith('let', position)) {
    return false;
  }
  let index = position + 3;
  const length = source.length;
  if (index >= length || (source[index] !== ' ' && source[index] !== '\t')) {
    return false;
  }
  while (index < length && (source[index] === ' ' || source[index] === '\t')) {
    index += 1;
  }
  if (index >= length || !/[A-Za-z_]/.test(source[index])) {
    return false;
  }
  index += 1;
  while (index < length && /[A-Za-z0-9_]/.test(source[index])) {
    index += 1;
  }
  while (index < length && (source[index] === ' ' || source[index] === '\t')) {
    index += 1;
  }
  return source[index] === ':' || source[index] === '=';
}

export function assertScannerMatchesRegex(source) {
  const masked = maskSkiffCommentsAndStrings(source);
  const regexRanges = collectRegexLocalLetRanges(masked);
  const scannerRanges = scanSkiffLocalLetRanges(source);
  const same = rangesEqual(regexRanges, scannerRanges);
  if (!same) {
    throw new Error([
      'Skiff scanner/regex mismatch; fail closed without editing',
      `regex-only: ${formatRanges(difference(regexRanges, scannerRanges))}`,
      `scanner-only: ${formatRanges(difference(scannerRanges, regexRanges))}`,
    ].join('\n'));
  }
  return { regexRanges, scannerRanges };
}

export function replaceLocalLet(source) {
  const { scannerRanges } = assertScannerMatchesRegex(source);
  return replaceAtRanges(source, scannerRanges, 'final');
}

function replaceAtRanges(source, ranges, replacement) {
  const sorted = [...ranges].sort((left, right) => left[0] - right[0] || left[1] - right[1]);
  const chunks = [];
  let cursor = 0;
  for (const range of sorted) {
    if (range[0] < cursor || range[0] >= range[1]) {
      throw new Error(`overlapping or invalid replacement range ${range.join(':')}`);
    }
    chunks.push(source.slice(cursor, range[0]), replacement);
    cursor = range[1];
  }
  chunks.push(source.slice(cursor));
  return chunks.join('');
}

function rangesEqual(left, right) {
  return left.length === right.length
    && left.every((range, index) => range[0] === right[index][0] && range[1] === right[index][1]);
}

function difference(left, right) {
  return left.filter((range) => !right.some((other) => (
    other[0] === range[0] && other[1] === range[1]
  )));
}

function formatRanges(ranges) {
  return ranges.map((range) => range.join(':')).join(',') || '<none>';
}

async function gitChecked(root, args) {
  const result = await captureCheckedCommand('git', ['-C', root, ...args], { cwd: root });
  return result.stdout;
}

async function gitTopLevel(root) {
  return (await gitChecked(root, ['rev-parse', '--show-toplevel'])).trim();
}

async function listTrackedFiles(root, patterns) {
  const output = await gitChecked(root, ['ls-files', '-z', '--', ...patterns]);
  return output.split('\0').filter(Boolean);
}

async function assertCleanOverlap(root, patterns, label) {
  const output = await gitChecked(root, [
    'status',
    '--porcelain=v1',
    '-z',
    '--',
    ...patterns,
  ]);
  const entries = output.split('\0').filter(Boolean);
  if (entries.length > 0) {
    throw new Error(`${label}: dirty overlap detected (${entries.length} path(s)); commit or revert before migration`);
  }
}

async function validateRepoRoot(definition) {
  const requestedRoot = path.resolve(definition.root);
  const gitRoot = await gitTopLevel(requestedRoot);
  const realRequested = await realpath(requestedRoot);
  const realGitRoot = await realpath(gitRoot);
  if (realRequested !== realGitRoot) {
    throw new Error(`${definition.name} root is not the git top-level: ${requestedRoot}`);
  }
  return {
    name: definition.name,
    root: requestedRoot,
    gitRoot: realGitRoot,
  };
}

async function validateTrackedFile(root, relativePath, label) {
  const safePath = assertSafeRelativePath(relativePath, label);
  const absolutePath = path.join(root, safePath);
  const fileStat = await lstat(absolutePath);
  if (fileStat.isSymbolicLink()) {
    throw new Error(`${label}: tracked file must not be a symlink: ${safePath}`);
  }
  if (!fileStat.isFile()) {
    throw new Error(`${label}: tracked file is not a regular file: ${safePath}`);
  }
  const realFile = await realpath(absolutePath);
  const realRoot = await realpath(root);
  const relativeReal = path.relative(realRoot, realFile);
  if (
    relativeReal === '..'
    || relativeReal.startsWith(`..${path.sep}`)
    || path.isAbsolute(relativeReal)
  ) {
    throw new Error(`${label}: tracked file resolves outside ${realRoot}: ${safePath}`);
  }
  return { path: safePath, absolutePath };
}

function assertSafeRelativePath(relativePath, label) {
  if (
    typeof relativePath !== 'string'
    || relativePath.length === 0
    || relativePath.includes('\0')
  ) {
    throw new Error(`${label}: invalid repository-relative path ${JSON.stringify(relativePath)}`);
  }
  const parts = relativePath.split('/');
  if (
    parts.some((part) => part === '..' || part === '.')
    || relativePath.startsWith('/')
  ) {
    throw new Error(`${label}: invalid repository-relative path ${relativePath}`);
  }
  return relativePath;
}

async function readSourceFile(absolutePath, label) {
  const buffer = await readFile(absolutePath);
  const hasBom = buffer.length >= 3
    && buffer[0] === 0xef
    && buffer[1] === 0xbb
    && buffer[2] === 0xbf;
  const logicalBuffer = hasBom ? buffer.subarray(3) : buffer;
  let logical;
  try {
    logical = new TextDecoder('utf-8', { fatal: true }).decode(logicalBuffer);
  } catch (error) {
    throw new Error(`${label}: invalid UTF-8 source: ${error.message}`);
  }
  return { buffer, logical, hasBom };
}

function charRangeToByteRange(source, range, bomOffset) {
  const prefixBytes = Buffer.byteLength(source.slice(0, range[0]));
  return [
    bomOffset + prefixBytes,
    bomOffset + prefixBytes + Buffer.byteLength(source.slice(range[0], range[1])),
  ];
}

async function collectRepoMigration(definition) {
  const repo = await validateRepoRoot(definition);
  const head = (await gitChecked(repo.gitRoot, ['rev-parse', 'HEAD'])).trim();
  await assertCleanOverlap(repo.gitRoot, ['*.skiff'], `${repo.name} .skiff`);

  const tracked = await listTrackedFiles(repo.gitRoot, ['*.skiff']);
  const files = [];
  const writes = [];
  let count = 0;

  for (const relativePath of tracked) {
    const label = `${repo.name} ${relativePath}`;
    const file = await validateTrackedFile(repo.gitRoot, relativePath, label);
    const source = await readSourceFile(file.absolutePath, label);
    const scanner = assertScannerMatchesRegex(source.logical);
    const replacementCount = scanner.regexRanges.length;
    count += replacementCount;

    if (replacementCount === 0) {
      continue;
    }

    const bomOffset = source.hasBom ? UTF8_BOM.length : 0;
    const byteRanges = scanner.regexRanges.map((range) => (
      charRangeToByteRange(source.logical, range, bomOffset)
    ));
    const replacements = scanner.regexRanges.map((range, index) => ({
      byteRange: byteRanges[index],
      oldText: source.logical.slice(range[0], range[1]),
      contextSha256: sha256Buffer(source.buffer),
    }));
    const replacedLogical = replaceLocalLet(source.logical);
    const replacedBuffer = Buffer.concat([
      source.hasBom ? UTF8_BOM : Buffer.alloc(0),
      Buffer.from(replacedLogical, 'utf8'),
    ]);
    const beforeSha256 = sha256Buffer(source.buffer);
    const afterSha256 = sha256Buffer(replacedBuffer);

    files.push({
      path: file.path,
      beforeSha256,
      afterSha256,
      replacementCount,
      byteRanges,
      replacements,
    });
    if (beforeSha256 !== afterSha256) {
      writes.push({ path: file.path, absolutePath: file.absolutePath, buffer: replacedBuffer });
    }
  }

  return {
    ...repo,
    head,
    files,
    writes,
    count,
  };
}

function assertExpectedCounts(repos, expects) {
  for (const repo of repos) {
    const expected = expects[repo.name] ?? DEFAULT_EXPECTED_COUNTS[repo.name];
    if (repo.count !== expected) {
      throw new Error([
        `${repo.name} count drift: expected ${expected} statement-head let declaration(s), found ${repo.count}`,
        `pass --expect ${repo.name}=${repo.count} after reviewing the new inventory`,
      ].join('\n'));
    }
  }
}

function buildNormalManifest(repos, mode) {
  const repoEntries = Object.fromEntries(repos.map((repo) => [repo.name, {
    root: repo.gitRoot,
    head: repo.head,
    count: repo.count,
    files: repo.files,
  }]));
  return {
    schemaVersion: MANIFEST_SCHEMA,
    mode,
    repos: repoEntries,
    counts: Object.fromEntries(repos.map((repo) => [repo.name, repo.count])),
    total: repos.reduce((total, repo) => total + repo.count, 0),
  };
}

async function writeAtomicFile(absolutePath, buffer) {
  const existing = await stat(absolutePath);
  const temporaryPath = path.join(
    path.dirname(absolutePath),
    `.${path.basename(absolutePath)}.${process.pid}.${Date.now()}.${randomBytes(4).toString('hex')}.tmp`,
  );
  try {
    await writeFile(temporaryPath, buffer, {
      flag: 'wx',
      mode: existing.mode & 0o7777,
    });
    await rename(temporaryPath, absolutePath);
  } catch (error) {
    await rm(temporaryPath, { force: true }).catch(() => {});
    throw error;
  }
}

async function applyNormalWrites(repos) {
  let changedFiles = 0;
  for (const repo of repos) {
    for (const write of repo.writes) {
      await writeAtomicFile(write.absolutePath, write.buffer);
      changedFiles += 1;
    }
  }
  return changedFiles;
}

export function scanRustStringLiterals(buffer) {
  const literals = [];
  const length = buffer.length;
  let position = 0;

  while (position < length) {
    const byte = buffer[position];
    if (byte === 0x2f && buffer[position + 1] === 0x2f) {
      position = skipRustLineComment(buffer, position);
      continue;
    }
    if (byte === 0x2f && buffer[position + 1] === 0x2a) {
      position = skipRustBlockComment(buffer, position);
      continue;
    }

    const rawPrefix = rawStringPrefixAt(buffer, position);
    if (rawPrefix !== null) {
      const literal = parseRawRustString(buffer, rawPrefix);
      literals.push(literal);
      position = literal.end;
      continue;
    }

    if (byte === 0x62 && buffer[position + 1] === 0x22) {
      const literal = parseOrdinaryRustString(buffer, position, true);
      literals.push(literal);
      position = literal.end;
      continue;
    }
    if (byte === 0x22) {
      const literal = parseOrdinaryRustString(buffer, position, false);
      literals.push(literal);
      position = literal.end;
      continue;
    }

    const charQuoteIndex = byte === 0x62 && buffer[position + 1] === 0x27
      ? position + 1
      : byte === 0x27
        ? position
        : -1;
    if (charQuoteIndex !== -1) {
      const character = tryParseRustCharacter(buffer, charQuoteIndex);
      if (character !== null) {
        position = character.end;
        continue;
      }
    }

    position += 1;
  }

  return literals;
}

function skipRustLineComment(buffer, start) {
  let index = start + 2;
  while (index < buffer.length && buffer[index] !== 0x0a && buffer[index] !== 0x0d) {
    index += 1;
  }
  if (index < buffer.length) {
    index += buffer[index] === 0x0d && buffer[index + 1] === 0x0a ? 2 : 1;
  }
  return index;
}

function skipRustBlockComment(buffer, start) {
  let index = start + 2;
  while (index + 1 < buffer.length) {
    if (buffer[index] === 0x2a && buffer[index + 1] === 0x2f) {
      return index + 2;
    }
    index += 1;
  }
  throw new Error('unterminated Rust block comment');
}

function rawStringPrefixAt(buffer, index) {
  const bytePrefix = buffer[index] === 0x62;
  const rawIndex = bytePrefix ? index + 1 : index;
  if (buffer[rawIndex] !== 0x72) {
    return null;
  }
  let hashes = 0;
  let hashIndex = rawIndex + 1;
  while (buffer[hashIndex] === 0x23) {
    hashes += 1;
    hashIndex += 1;
  }
  if (buffer[hashIndex] !== 0x22) {
    return null;
  }
  return {
    start: index,
    bytePrefix,
    hashes,
    quoteIndex: hashIndex,
  };
}

function parseRawRustString(buffer, prefix) {
  const contentStart = prefix.quoteIndex + 1;
  let scan = contentStart;
  while (scan < buffer.length) {
    if (buffer[scan] === 0x22 && hashesFollow(buffer, scan + 1, prefix.hashes)) {
      const contentEnd = scan;
      const end = scan + 1 + prefix.hashes;
      return {
        kind: 'raw',
        rawDelimiter: `${prefix.bytePrefix ? 'b' : ''}r${'#'.repeat(prefix.hashes)}"`,
        contentStart,
        contentEnd,
        decodedText: buffer.subarray(contentStart, contentEnd).toString('utf8'),
        mapping: null,
        end,
      };
    }
    scan += 1;
  }
  throw new Error('unterminated Rust raw string');
}

function hashesFollow(buffer, start, count) {
  for (let index = 0; index < count; index += 1) {
    if (buffer[start + index] !== 0x23) {
      return false;
    }
  }
  return true;
}

function parseOrdinaryRustString(buffer, start, bytePrefix) {
  const quoteIndex = bytePrefix ? start + 1 : start;
  const contentStart = quoteIndex + 1;
  const decoded = [];
  const mapping = [];
  let scan = contentStart;

  while (scan < buffer.length) {
    const byte = buffer[scan];
    if (byte === 0x22) {
      return {
        kind: 'ordinary',
        rawDelimiter: bytePrefix ? 'b"' : '"',
        contentStart,
        contentEnd: scan,
        decodedText: Buffer.from(decoded).toString('utf8'),
        mapping,
        end: scan + 1,
      };
    }
    if (byte === 0x5c) {
      const escapeStart = scan;
      scan += 1;
      if (scan >= buffer.length) {
        throw new Error('unterminated escape in Rust string');
      }
      const escaped = buffer[scan];
      if (escaped === 0x0a || escaped === 0x0d) {
        scan += escaped === 0x0d && buffer[scan + 1] === 0x0a ? 2 : 1;
        continue;
      }
      const decodedBytes = decodeRustEscape(buffer, scan, (index) => {
        scan = index;
      });
      appendDecoded(decoded, mapping, decodedBytes, escapeStart, scan + 1);
      scan += 1;
      continue;
    }

    appendDecoded(decoded, mapping, [byte], scan, scan + 1);
    scan += 1;
  }

  throw new Error('unterminated Rust string');
}

function decodeRustEscape(buffer, index, updateIndex) {
  const escaped = buffer[index];
  if (escaped === 0x6e) return [0x0a];
  if (escaped === 0x72) return [0x0d];
  if (escaped === 0x74) return [0x09];
  if (escaped === 0x30) return [0x00];
  if (escaped === 0x5c) return [0x5c];
  if (escaped === 0x22) return [0x22];
  if (escaped === 0x27) return [0x27];
  if (escaped === 0x78) {
    if (index + 2 < buffer.length && isHexByte(buffer[index + 1]) && isHexByte(buffer[index + 2])) {
      updateIndex(index + 2);
      return [hexValue(buffer[index + 1]) * 16 + hexValue(buffer[index + 2])];
    }
    throw new Error('invalid Rust \\x escape');
  }
  if (escaped === 0x75) {
    if (buffer[index + 1] === 0x7b) {
      const close = buffer.indexOf(0x7d, index + 2);
      if (close === -1 || close === index + 2) {
        throw new Error('invalid Rust \\u escape');
      }
      const text = buffer.subarray(index + 2, close).toString('utf8');
      if (!/^[0-9A-Fa-f]+$/.test(text)) {
        throw new Error('invalid Rust \\u escape');
      }
      const codePoint = Number.parseInt(text, 16);
      if (codePoint > 0x10ffff) {
        throw new Error('Rust unicode escape is out of range');
      }
      updateIndex(close);
      return [...Buffer.from(String.fromCodePoint(codePoint), 'utf8')];
    }
    throw new Error('invalid Rust \\u escape');
  }
  return [escaped];
}

function isHexByte(byte) {
  return (byte >= 0x30 && byte <= 0x39)
    || (byte >= 0x41 && byte <= 0x46)
    || (byte >= 0x61 && byte <= 0x66);
}

function hexValue(byte) {
  if (byte >= 0x30 && byte <= 0x39) return byte - 0x30;
  if (byte >= 0x41 && byte <= 0x46) return byte - 0x41 + 10;
  return byte - 0x61 + 10;
}

function appendDecoded(decoded, mapping, bytes, start, end) {
  for (const byte of bytes) {
    decoded.push(byte);
    mapping.push({ start, end });
  }
}

function tryParseRustCharacter(buffer, quoteIndex) {
  let scan = quoteIndex + 1;
  let escaped = false;
  while (scan < buffer.length) {
    const byte = buffer[scan];
    if (escaped) {
      escaped = false;
      scan += 1;
      continue;
    }
    if (byte === 0x5c) {
      escaped = true;
      scan += 1;
      continue;
    }
    if (byte === 0x27) {
      return { start: quoteIndex, end: scan + 1 };
    }
    if (
      byte === 0x0a
      || byte === 0x0d
      || byte === 0x20
      || byte === 0x09
      || isRustCharBoundary(byte)
    ) {
      return null;
    }
    scan += 1;
  }
  return null;
}

function isRustCharBoundary(byte) {
  return byte === 0x2c
    || byte === 0x3b
    || byte === 0x28
    || byte === 0x29
    || byte === 0x3a
    || byte === 0x7b
    || byte === 0x7d
    || byte === 0x5b
    || byte === 0x5d
    || byte === 0x3c
    || byte === 0x3e
    || byte === 0x3d
    || byte === 0x2b
    || byte === 0x2a
    || byte === 0x26
    || byte === 0x7c
    || byte === 0x21
    || byte === 0x3f;
}

function mapDecodedRange(literal, decodedStart, decodedEnd) {
  if (decodedStart < 0 || decodedEnd <= decodedStart) {
    throw new Error('invalid decoded range');
  }
  if (literal.mapping === null) {
    return [literal.contentStart + decodedStart, literal.contentStart + decodedEnd];
  }
  const startMapping = literal.mapping[decodedStart];
  const endMapping = literal.mapping[decodedEnd - 1];
  if (!startMapping || !endMapping) {
    throw new Error('decoded range does not map to original Rust string bytes');
  }
  return [startMapping.start, endMapping.end];
}

export async function collectEmbeddedCandidates(repoDefinitions) {
  const repos = [];
  const candidates = [];
  const counts = {};

  for (const definition of repoDefinitions) {
    const repo = await validateRepoRoot(definition);
    const head = (await gitChecked(repo.gitRoot, ['rev-parse', 'HEAD'])).trim();
    const rustFiles = await listTrackedFiles(repo.gitRoot, ['*.rs']);
    const allowlistFiles = repo.name === 'skiff'
      ? await listTrackedFiles(repo.gitRoot, EMBEDDED_ALLOWLIST)
      : [];
    const patterns = [...new Set(['*.rs', ...allowlistFiles])];
    await assertCleanOverlap(repo.gitRoot, patterns, `${repo.name} embedded fixtures`);
    repos.push({ ...repo, head });

    let repoCount = 0;
    for (const relativePath of [...rustFiles, ...allowlistFiles]) {
      const label = `${repo.name} ${relativePath}`;
      const file = await validateTrackedFile(repo.gitRoot, relativePath, label);
      const buffer = await readFile(file.absolutePath);
      const literals = scanRustStringLiterals(buffer);
      const fileCandidates = [];

      for (const literal of literals) {
        const decodedRanges = scanSkiffLocalLetRanges(literal.decodedText);
        for (const decodedRange of decodedRanges) {
          const [decodedStart, decodedEnd] = charRangeToByteRange(
            literal.decodedText,
            decodedRange,
            0,
          );
          const range = mapDecodedRange(literal, decodedStart, decodedEnd);
          const oldBytes = buffer.subarray(range[0], range[1]).toString('utf8');
          if (oldBytes !== 'let') {
            throw new Error(`${label}: scanner range did not resolve to literal "let" bytes`);
          }
          fileCandidates.push({
            repo: repo.name,
            head,
            path: file.path,
            rawDelimiter: literal.rawDelimiter,
            range,
            oldText: oldBytes,
            beforeSha256: sha256Buffer(buffer),
            contextSha256: sha256Buffer(buffer),
            afterSha256: null,
          });
        }
      }

      if (fileCandidates.length > 0) {
        const applied = applyByteReplacements(
          buffer,
          fileCandidates.map((candidate) => candidate.range),
          'final',
        );
        const afterSha256 = sha256Buffer(applied);
        for (const candidate of fileCandidates) {
          candidate.afterSha256 = afterSha256;
        }
      }
      repoCount += fileCandidates.length;
      candidates.push(...fileCandidates);
    }
    counts[repo.name] = repoCount;
  }

  candidates.sort((left, right) => (
    left.repo.localeCompare(right.repo)
    || left.path.localeCompare(right.path)
    || left.range[0] - right.range[0]
  ));
  return {
    repos,
    candidates,
    counts,
    total: Object.values(counts).reduce((total, count) => total + count, 0),
  };
}

export function applyByteReplacements(buffer, inputRanges, replacement = 'final') {
  const ranges = [...inputRanges].sort((left, right) => (
    left[0] - right[0] || left[1] - right[1]
  ));
  const chunks = [];
  let cursor = 0;
  for (const range of ranges) {
    const [start, end] = range;
    if (
      start < cursor
      || start >= end
      || end > buffer.length
    ) {
      throw new Error(`overlapping or invalid replacement range ${range.join(':')}`);
    }
    const oldBytes = buffer.subarray(start, end).toString('utf8');
    if (oldBytes !== 'let') {
      throw new Error(`range ${range.join(':')} does not contain the literal "let"`);
    }
    chunks.push(buffer.subarray(cursor, start), Buffer.from(replacement, 'utf8'));
    cursor = end;
  }
  chunks.push(buffer.subarray(cursor));
  return Buffer.concat(chunks);
}

async function runEmbeddedCheck(repoDefinitions, manifestOut) {
  const result = await collectEmbeddedCandidates(repoDefinitions);
  const manifest = {
    schemaVersion: EMBEDDED_MANIFEST_SCHEMA,
    mode: 'embedded-check',
    repos: Object.fromEntries(result.repos.map((repo) => [repo.name, {
      root: repo.gitRoot,
      head: repo.head,
    }])),
    counts: result.counts,
    total: result.total,
    candidates: result.candidates,
  };
  await writeJsonAtomic(manifestOut, manifest);
  return {
    mode: 'embedded-check',
    manifest,
    counts: result.counts,
    total: result.total,
  };
}

async function runEmbeddedWrite(manifestPath, repoDefinitions) {
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  if (
    manifest?.schemaVersion !== EMBEDDED_MANIFEST_SCHEMA
    || manifest?.mode !== 'embedded-check'
  ) {
    throw new Error('embedded write requires a reviewed embedded-fixtures-check manifest');
  }
  if (!Array.isArray(manifest.candidates)) {
    throw new Error('embedded manifest candidates must be an array');
  }
  if (!manifest.repos || typeof manifest.repos !== 'object') {
    throw new Error('embedded manifest repos metadata is missing');
  }

  const requestedRoots = Object.fromEntries(repoDefinitions.map((definition) => [
    definition.name,
    path.resolve(definition.root),
  ]));
  for (const [repoName, metadata] of Object.entries(manifest.repos)) {
    if (await realpath(metadata.root) !== await realpath(requestedRoots[repoName])) {
      throw new Error(`embedded manifest ${repoName} root does not match CLI root`);
    }
  }

  const grouped = groupEmbeddedCandidates(manifest.candidates);
  const writes = [];

  for (const group of grouped) {
    const repoName = group.repo;
    const repo = await validateRepoRoot({
      name: repoName,
      root: requestedRoots[repoName],
    });
    if (repo.gitRoot !== await realpath(manifest.repos[repoName].root)) {
      throw new Error(`embedded manifest ${repoName} top-level changed`);
    }
    const currentHead = (await gitChecked(repo.gitRoot, ['rev-parse', 'HEAD'])).trim();
    if (currentHead !== group.head || currentHead !== manifest.repos[repoName].head) {
      throw new Error(`embedded manifest ${repoName} HEAD drifted; review before write`);
    }

    const file = await validateTrackedFile(repo.gitRoot, group.path, `${repoName} ${group.path}`);
    const buffer = await readFile(file.absolutePath);
    const first = group.entries[0];
    if (
      sha256Buffer(buffer) !== first.beforeSha256
      || sha256Buffer(buffer) !== first.contextSha256
    ) {
      throw new Error(`${repoName} ${group.path}: embedded manifest context hash mismatch`);
    }
    for (const entry of group.entries) {
      const oldBytes = buffer.subarray(entry.range[0], entry.range[1]).toString('utf8');
      if (oldBytes !== entry.oldText || oldBytes !== 'let') {
        throw new Error(`${repoName} ${group.path}: embedded manifest range mismatch`);
      }
    }

    const applied = applyByteReplacements(
      buffer,
      group.entries.map((entry) => entry.range),
      'final',
    );
    const afterSha256 = sha256Buffer(applied);
    for (const entry of group.entries) {
      if (entry.afterSha256 !== null && entry.afterSha256 !== afterSha256) {
        throw new Error(`${repoName} ${group.path}: embedded manifest after-hash mismatch`);
      }
    }
    writes.push({ repoName, absolutePath: file.absolutePath, buffer: applied });
  }

  for (const write of writes) {
    await writeAtomicFile(write.absolutePath, write.buffer);
  }

  const appliedCandidates = manifest.candidates.map((candidate) => {
    const matchingWrite = writes.find((write) => write.repoName === candidate.repo);
    return {
      ...candidate,
      afterSha256: matchingWrite ? sha256Buffer(matchingWrite.buffer) : candidate.afterSha256,
      status: 'applied',
    };
  });
  const appliedManifest = {
    ...manifest,
    mode: 'embedded-applied',
    candidates: appliedCandidates,
  };
  await writeJsonAtomic(manifestPath, appliedManifest);
  return {
    mode: 'embedded-write',
    manifest: appliedManifest,
    changedFiles: writes.length,
  };
}

function groupEmbeddedCandidates(candidates) {
  const groups = new Map();
  for (const candidate of candidates) {
    if (!candidate || typeof candidate !== 'object') {
      throw new Error('embedded manifest contains an invalid candidate');
    }
    const key = `${candidate.repo}\0${candidate.path}`;
    const group = groups.get(key) ?? {
      repo: candidate.repo,
      path: candidate.path,
      head: candidate.head,
      entries: [],
    };
    group.entries.push(candidate);
    groups.set(key, group);
  }
  return [...groups.values()];
}

function sha256Buffer(buffer) {
  return createHash('sha256').update(buffer).digest('hex');
}

export async function migrateLocalLetToFinal(argv, { cwd = process.cwd() } = {}) {
  const parsed = parseMigrationArgs(argv);
  const manifestOut = path.resolve(cwd, parsed.manifestOut);
  const repoDefinitions = REPO_DEFINITIONS.map((definition) => ({
    name: definition.name,
    root: parsed.roots[definition.name],
  }));

  if (parsed.mode === 'embedded-fixtures-check') {
    return runEmbeddedCheck(repoDefinitions, manifestOut);
  }
  if (parsed.mode === 'embedded-fixtures-write') {
    return runEmbeddedWrite(manifestOut, repoDefinitions);
  }

  const repos = [];
  for (const definition of repoDefinitions) {
    repos.push(await collectRepoMigration(definition));
  }
  assertExpectedCounts(repos, parsed.expects);

  const manifest = buildNormalManifest(repos, parsed.mode);
  if (parsed.mode === 'write') {
    for (const repo of repos) {
      await assertCleanOverlap(repo.gitRoot, ['*.skiff'], `${repo.name} .skiff`);
    }
    const changedFiles = await applyNormalWrites(repos);
    await writeJsonAtomic(manifestOut, manifest);
    return {
      mode: parsed.mode,
      manifest,
      counts: manifest.counts,
      total: manifest.total,
      changedFiles,
    };
  }

  await writeJsonAtomic(manifestOut, manifest);
  return {
    mode: parsed.mode,
    manifest,
    counts: manifest.counts,
    total: manifest.total,
    changedFiles: 0,
  };
}

async function main() {
  try {
    const result = await migrateLocalLetToFinal(process.argv.slice(2));
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
