#!/usr/bin/env node

import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const defaultRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const runtimeRootEvalPrefix = 'runtime/driver/eval/';
const promotedEvalPrefix = 'runtime/eval/src/';
const evalErrorBoundaries = new Set(['runtime/driver/eval/error.rs', 'runtime/eval/src/error.rs']);

const rules = [
  {
    id: 'eval_error_root_dependency_boundary',
    appliesTo: (relPath) => relPath.startsWith(runtimeRootEvalPrefix),
    allowedRelPaths: new Set(['runtime/driver/eval/error.rs']),
    regexp: regexpUnion([
      /\bcrate\s*::\s*error\s*(?:::|\b)/,
      /\bcrate\s*::\s*\{\s*error\s*(?:::|,|\})/,
      /\bcrate\s*::\s*\{[^;]*?,\s*error\s*(?:::|,|\})/,
    ]),
    allowedBoundary:
      'Runtime-root eval facade code must not directly import or reference runtime crate::error / crate::error::*; promoted runtime-eval crate::error is eval-local and is not part of this rule.',
  },
  {
    id: 'eval_error_diagnostic_wrapper_boundary',
    allowedRelPaths: evalErrorBoundaries,
    regexp: regexpUnion([
      /\.(?:with_source|with_diagnostic_frame)\s*\(/,
      /\bRuntimeError\s*::\s*(?:WithSource|WithDiagnosticFrame)\b/,
    ]),
    allowedBoundary:
      'Only eval error boundary files may use RuntimeError diagnostic wrapper internals; other production eval files must call eval::error helpers.',
  },
  {
    id: 'eval_error_no_root_only_runtime_variants',
    allowedRelPaths: new Set(),
    regexp: regexpUnion([
      /\bRuntimeError\s*::\s*(?:Mongo|BsonSer|BsonDe)\b/,
      /^\s*(?:Mongo|BsonSer|BsonDe)\s*\(/m,
    ]),
    allowedBoundary:
      'Production eval must not directly use root-only platform RuntimeError variants (Mongo, BsonSer, BsonDe); route eval errors through eval-owned variants or lower contracts.',
  },
  {
    id: 'eval_error_json_variant_boundary',
    allowedRelPaths: evalErrorBoundaries,
    regexp: regexpUnion([
      /\bRuntimeError\s*::\s*Json\b/,
      /^\s*Json\s*\(/m,
    ]),
    allowedBoundary:
      'Only eval error boundary files may define or directly use the eval-owned Json RuntimeError variant.',
  },
];

const options = parseArgs(process.argv.slice(2));
if (options.selfTest) {
  await runSelfTest();
} else {
  const { violations, scannedFiles } = await collectViolations(options.root);
  if (violations.length > 0) {
    console.error('\nRuntime eval error boundary check failed.\n');
    for (const violation of violations) {
      console.error(formatViolation(violation));
    }
    process.exitCode = 1;
  } else {
    console.log(
      `Runtime eval error boundary check passed for ${scannedFiles} production eval file(s).`,
    );
  }
}

function parseArgs(argv) {
  const options = { root: defaultRoot, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--self-test') {
      options.selfTest = true;
      continue;
    }
    if (arg === '--root') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--root requires a directory path');
      }
      options.root = resolve(value);
      index += 1;
      continue;
    }
    if (arg.startsWith('--root=')) {
      options.root = resolve(arg.slice('--root='.length));
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  if (options.selfTest && options.root !== defaultRoot) {
    throw new Error('--self-test uses hermetic fixtures and cannot be combined with --root');
  }
  return options;
}

async function collectViolations(root) {
  const files = await collectProductionEvalRustFiles(root);
  const violations = [];
  for (const file of files) {
    const source = await readFile(file.absPath, 'utf8');
    const scanText = maskRustCommentsAndStrings(stripCfgTestSupportItems(source));
    for (const rule of rules) {
      if (rule.appliesTo && !rule.appliesTo(file.relPath)) {
        continue;
      }
      for (const match of scanText.matchAll(rule.regexp)) {
        if (rule.allowedRelPaths.has(file.relPath)) {
          continue;
        }
        violations.push({
          rule,
          relPath: file.relPath,
          line: lineNumberAt(scanText, match.index ?? 0),
          matched: match[0].trim().replace(/\s+/g, ' '),
          sourceLine: sourceLineAt(source, match.index ?? 0),
        });
      }
    }
  }
  return { violations, scannedFiles: files.length };
}

async function collectProductionEvalRustFiles(root) {
  const roots = [
    join(root, 'runtime', 'driver', 'eval', 'mod.rs'),
    join(root, 'runtime', 'eval', 'src', 'lib.rs'),
  ];
  const files = new Map();
  for (const moduleRoot of roots) {
    await collectReachableModules(root, moduleRoot, files);
  }
  return [...files.values()].sort((left, right) => left.relPath.localeCompare(right.relPath));
}

async function collectReachableModules(root, absPath, files) {
  const relPath = normalizePath(relative(root, absPath));
  if (files.has(relPath)) {
    return;
  }
  const source = await readFile(absPath, 'utf8');
  if (isCfgTestSupportOnlyFile(source)) {
    return;
  }
  files.set(relPath, { absPath, relPath });

  const production = stripCfgTestSupportItems(source);
  const masked = maskRustCommentsAndStrings(production);
  const modulePattern =
    /(?:^|\n)\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(?<name>[A-Za-z_][A-Za-z0-9_]*)\s*;/g;
  for (const match of masked.matchAll(modulePattern)) {
    const child = await resolveModuleFile(absPath, match.groups.name);
    if (child !== undefined) {
      await collectReachableModules(root, child, files);
    }
  }
}

async function resolveModuleFile(parentPath, moduleName) {
  const parentDirectory = dirname(parentPath);
  const stem = parentPath.endsWith('/mod.rs') || parentPath.endsWith('/lib.rs')
    ? parentDirectory
    : join(parentDirectory, parentPath.slice(parentPath.lastIndexOf('/') + 1, -3));
  for (const candidate of [join(stem, `${moduleName}.rs`), join(stem, moduleName, 'mod.rs')]) {
    try {
      await readFile(candidate, 'utf8');
      return candidate;
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        throw error;
      }
    }
  }
  return undefined;
}

function formatViolation(violation) {
  return [
    `DENY ${violation.relPath}:${violation.line} ${violation.rule.id}`,
    `  matched: ${violation.matched}`,
    `  line: ${violation.sourceLine}`,
    `  allowed boundary: ${violation.rule.allowedBoundary}`,
  ].join('\n');
}

async function runSelfTest() {
  const cases = [
    {
      name: 'diagnostic wrapper mutation',
      file: 'runtime/eval/src/production.rs',
      source: 'fn mutation(error: RuntimeError) { let _ = RuntimeError::WithSource { source_id: 1, frame: todo!(), error: Box::new(error) }; }\n',
      expectedId: 'eval_error_diagnostic_wrapper_boundary',
    },
    {
      name: 'eval Json variant mutation',
      file: 'runtime/eval/src/production.rs',
      source: 'fn mutation(error: serde_json::Error) { let _ = RuntimeError::Json(error); }\n',
      expectedId: 'eval_error_json_variant_boundary',
    },
    {
      name: 'root-only variant mutation',
      file: 'runtime/eval/src/production.rs',
      source: 'fn mutation(error: mongodb::error::Error) { let _ = RuntimeError::Mongo(error); }\n',
      expectedId: 'eval_error_no_root_only_runtime_variants',
    },
    {
      name: 'runtime-root dependency mutation',
      file: 'runtime/driver/eval/bridge.rs',
      source: 'use crate::error::RuntimeError;\n',
      expectedId: 'eval_error_root_dependency_boundary',
    },
  ];

  for (const testCase of cases) {
    const root = await createSelfTestFixture();
    try {
      const baseline = await collectViolations(root);
      if (baseline.violations.length !== 0) {
        throw new Error(
          `${testCase.name}: baseline fixture was not clean:\n${baseline.violations.map(formatViolation).join('\n')}`,
        );
      }
      await writeFile(join(root, testCase.file), testCase.source);
      const mutated = await collectViolations(root);
      if (
        mutated.violations.length !== 1 ||
        mutated.violations[0].rule.id !== testCase.expectedId
      ) {
        throw new Error(
          `${testCase.name}: expected one ${testCase.expectedId} violation, got:\n${mutated.violations.map(formatViolation).join('\n')}`,
        );
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  }

  console.log(
    `Runtime eval error boundary self-test passed (${cases.length} mutation RED cases; cfg(test) external module excluded by reachability).`,
  );
}

async function createSelfTestFixture() {
  const root = await mkdtemp(join(tmpdir(), 'skiff-runtime-eval-error-boundary-'));
  await mkdir(join(root, 'runtime', 'driver', 'eval'), { recursive: true });
  await mkdir(join(root, 'runtime', 'eval', 'src'), { recursive: true });
  await writeFile(
    join(root, 'runtime', 'driver', 'eval', 'mod.rs'),
    'mod bridge;\npub(crate) mod error;\n',
  );
  await writeFile(join(root, 'runtime', 'driver', 'eval', 'bridge.rs'), 'fn clean() {}\n');
  await writeFile(
    join(root, 'runtime', 'driver', 'eval', 'error.rs'),
    'pub(crate) use skiff_runtime_eval::error::*;\n',
  );
  await writeFile(
    join(root, 'runtime', 'eval', 'src', 'lib.rs'),
    'pub mod error;\nmod production;\n#[cfg(test)]\nmod external_tests;\n',
  );
  await writeFile(
    join(root, 'runtime', 'eval', 'src', 'error.rs'),
    'enum RuntimeError { Json(serde_json::Error), WithSource { error: Box<RuntimeError> } }\n',
  );
  await writeFile(join(root, 'runtime', 'eval', 'src', 'production.rs'), 'fn clean() {}\n');
  await writeFile(
    join(root, 'runtime', 'eval', 'src', 'external_tests.rs'),
    'fn test_only(error: RuntimeError) { let _ = RuntimeError::WithDiagnosticFrame { error: Box::new(error) }; }\n',
  );
  return root;
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

function maskRustCommentsAndStrings(text) {
  let output = '';
  for (let index = 0; index < text.length; ) {
    const rawString = rawStringAt(text, index);
    if (rawString) {
      output += maskSegment(text.slice(index, rawString.end));
      index = rawString.end;
      continue;
    }

    if (text.startsWith('//', index)) {
      const end = text.indexOf('\n', index);
      const commentEnd = end === -1 ? text.length : end;
      output += maskSegment(text.slice(index, commentEnd));
      index = commentEnd;
      continue;
    }

    if (text.startsWith('/*', index)) {
      const end = blockCommentEnd(text, index);
      output += maskSegment(text.slice(index, end));
      index = end;
      continue;
    }

    if (text[index] === '"' || (text[index] === 'b' && text[index + 1] === '"')) {
      const start = index;
      index += text[index] === 'b' ? 2 : 1;
      while (index < text.length) {
        if (text[index] === '\\') {
          index += 2;
          continue;
        }
        if (text[index] === '"') {
          index += 1;
          break;
        }
        index += 1;
      }
      output += maskSegment(text.slice(start, index));
      continue;
    }

    output += text[index];
    index += 1;
  }
  return output;
}

function rawStringAt(text, index) {
  let cursor = index;
  if (text[cursor] === 'b') {
    cursor += 1;
  }
  if (text[cursor] !== 'r') {
    return undefined;
  }
  cursor += 1;

  let hashes = 0;
  while (text[cursor] === '#') {
    hashes += 1;
    cursor += 1;
  }
  if (text[cursor] !== '"') {
    return undefined;
  }

  const terminator = `"${'#'.repeat(hashes)}`;
  const terminatorIndex = text.indexOf(terminator, cursor + 1);
  const end = terminatorIndex === -1 ? text.length : terminatorIndex + terminator.length;
  return { end };
}

function blockCommentEnd(text, start) {
  let depth = 0;
  for (let index = start; index < text.length; index += 1) {
    if (text.startsWith('/*', index)) {
      depth += 1;
      index += 1;
      continue;
    }
    if (text.startsWith('*/', index)) {
      depth -= 1;
      index += 1;
      if (depth === 0) {
        return index + 1;
      }
    }
  }
  return text.length;
}

function maskSegment(segment) {
  return segment.replace(/[^\n]/g, ' ');
}

function matchingBraceIndex(text, openBrace) {
  let depth = 0;
  for (let index = openBrace; index < text.length; index += 1) {
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

function lineNumberAt(text, index) {
  let line = 1;
  for (let i = 0; i < index; i += 1) {
    if (text.charCodeAt(i) === 10) {
      line += 1;
    }
  }
  return line;
}

function sourceLineAt(text, index) {
  const lineStart = text.lastIndexOf('\n', index) + 1;
  const lineEnd = text.indexOf('\n', index);
  return text.slice(lineStart, lineEnd === -1 ? text.length : lineEnd).trim();
}

function normalizePath(path) {
  return path.split('\\').join('/');
}

function regexpUnion(regexps) {
  return new RegExp(regexps.map((regexp) => regexp.source).join('|'), 'gm');
}
