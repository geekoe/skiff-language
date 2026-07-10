#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const canonicalRelPath = 'artifact-identity/src/lib.rs';
const duplicateDefinitionRegexp =
  /\bstruct\s+OperationAbiIdentityInput\b|\bfn\s+operation_abi_identity\s*\(/g;

const canonicalRequirements = [
  {
    name: 'OperationAbiIdentityInput',
    regexp: /\bpub\s+struct\s+OperationAbiIdentityInput\b/,
  },
  {
    name: 'operation_abi_hash',
    regexp: /\bpub\s+fn\s+operation_abi_hash\s*\(/,
  },
  {
    name: 'operation_abi_identity',
    regexp: /\bpub\s+fn\s+operation_abi_identity\s*\(/,
  },
  {
    name: 'public_function_operation_abi_id',
    regexp: /\bpub\s+fn\s+public_function_operation_abi_id\s*\(/,
  },
  {
    name: 'public_instance_method_operation_abi_id',
    regexp: /\bpub\s+fn\s+public_instance_method_operation_abi_id\s*\(/,
  },
];

const adapterRequirements = [
  {
    relPath: 'compiler/driver/shared/operation_abi_identity.rs',
    helper: 'public_function_operation_abi_id',
    regexp: /\bskiff_compiler_emission::identity::public_function_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/driver/shared/operation_abi_identity.rs',
    helper: 'public_instance_method_operation_abi_id',
    regexp: /\bskiff_compiler_emission::identity::public_instance_method_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'public_function_operation_abi_id',
    regexp: /\bskiff_artifact_identity::public_function_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'public_instance_method_operation_abi_id',
    regexp: /\bskiff_artifact_identity::public_instance_method_operation_abi_id\b/,
  },
];

const options = parseArgs(process.argv.slice(2));

if (options.help) {
  printUsage();
} else if (options.selfTest) {
  runSelfTest();
} else {
  await runCheck();
}

async function runCheck() {
  const failures = [];
  const files = await collectCandidateRustFiles(root);
  const canonicalText = stripInlineTestModules(await readFile(join(root, canonicalRelPath), 'utf8'));

  for (const requirement of canonicalRequirements) {
    if (!requirement.regexp.test(canonicalText)) {
      failures.push(`${canonicalRelPath} is missing canonical ${requirement.name}`);
    }
  }

  for (const requirement of adapterRequirements) {
    const text = await readFile(join(root, requirement.relPath), 'utf8');
    if (!requirement.regexp.test(text)) {
      failures.push(
        `${requirement.relPath} must delegate ${requirement.helper} to skiff_artifact_identity`,
      );
    }
  }

  for (const violation of collectDuplicateDefinitionViolations(files)) {
    failures.push(
      `${violation.relPath}:${violation.line} duplicate operation ABI identity projection ${violation.matched}`,
    );
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    process.exitCode = 1;
    return;
  }

  console.log('Operation ABI identity single-source check passed.');
}

function collectDuplicateDefinitionViolations(files) {
  const violations = [];

  for (const file of files) {
    if (file.relPath === canonicalRelPath || !isProductionRustFile(file.relPath)) {
      continue;
    }
    const text = stripInlineTestModules(file.text);
    for (const match of text.matchAll(duplicateDefinitionRegexp)) {
      violations.push({
        relPath: file.relPath,
        line: lineNumberAt(text, match.index ?? 0),
        matched: match[0],
      });
    }
  }

  return violations;
}

async function collectCandidateRustFiles(repoRoot) {
  const files = [];
  await collectRustFiles(join(repoRoot, 'artifact-identity/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler'), files);
  return files;
}

async function collectRustFiles(directory, files) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      return;
    }
    throw error;
  }

  for (const entry of entries) {
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
      relPath: normalizePath(relative(root, absPath)),
      text: await readFile(absPath, 'utf8'),
    });
  }
}

function isProductionRustFile(relPath) {
  if (relPath.endsWith('/tests.rs') || relPath.split('/').includes('tests')) {
    return false;
  }
  if (relPath.startsWith('artifact-identity/src/')) {
    return true;
  }
  if (relPath.startsWith('compiler/tests/')) {
    return false;
  }
  return (
    relPath.startsWith('compiler/driver/')
    || relPath.startsWith('compiler/core/src/')
    || relPath.startsWith('compiler/source/src/')
    || relPath.startsWith('compiler/lowering/src/')
    || relPath.startsWith('compiler/projection-input/src/')
    || relPath.startsWith('compiler/compiled/src/')
    || relPath.startsWith('compiler/projection/src/')
    || relPath.startsWith('compiler/emission/src/')
  );
}

function runSelfTest() {
  const cases = [
    {
      name: 'allows canonical artifact identity definitions',
      files: [
        {
          relPath: canonicalRelPath,
          text: 'pub struct OperationAbiIdentityInput;\npub fn operation_abi_identity() {}\n',
        },
      ],
      expectedViolations: 0,
    },
    {
      name: 'rejects compiler duplicate struct',
      files: [
        {
          relPath: 'compiler/driver/shared/operation_abi_identity.rs',
          text: 'struct OperationAbiIdentityInput;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects artifact-identity non-canonical duplicate function',
      files: [
        {
          relPath: 'artifact-identity/src/other.rs',
          text: 'fn operation_abi_identity() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'ignores compiler test files',
      files: [
        {
          relPath: 'compiler/tests/operation_identity.rs',
          text: 'struct OperationAbiIdentityInput;\nfn operation_abi_identity() {}\n',
        },
      ],
      expectedViolations: 0,
    },
    {
      name: 'ignores cfg test modules',
      files: [
        {
          relPath: 'compiler/driver/shared/operation_abi_identity.rs',
          text: '#[cfg(test)]\nmod tests { struct OperationAbiIdentityInput; }\n',
        },
      ],
      expectedViolations: 0,
    },
  ];

  const failures = [];
  for (const testCase of cases) {
    const violations = collectDuplicateDefinitionViolations(testCase.files);
    if (violations.length !== testCase.expectedViolations) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedViolations} violation(s), got ${violations.length}`,
      );
    }
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    process.exitCode = 1;
    return;
  }

  console.log('Operation ABI identity single-source self-test passed.');
}

function stripInlineTestModules(text) {
  let output = text;
  let searchIndex = 0;
  while (searchIndex < output.length) {
    const attrIndex = output.indexOf('#[cfg(test)]', searchIndex);
    if (attrIndex === -1) {
      break;
    }
    const removal = cfgTestItemRange(output, attrIndex);
    if (removal === undefined) {
      searchIndex = attrIndex + 1;
      continue;
    }
    const replacement = output.slice(removal.start, removal.end).replace(/[^\n]/g, ' ');
    output = output.slice(0, removal.start) + replacement + output.slice(removal.end);
    searchIndex = removal.start + replacement.length;
  }
  return output;
}

function cfgTestItemRange(text, attrIndex) {
  const attrMatch = /^#\[cfg\(test\)\]/.exec(text.slice(attrIndex));
  if (!attrMatch) {
    return undefined;
  }
  let index = attrIndex + attrMatch[0].length;
  while (index < text.length && /\s/.test(text[index])) {
    index += 1;
  }
  const nextSemicolon = text.indexOf(';', index);
  const nextBrace = text.indexOf('{', index);
  if (nextSemicolon !== -1 && (nextBrace === -1 || nextSemicolon < nextBrace)) {
    return { start: attrIndex, end: nextSemicolon + 1 };
  }
  if (nextBrace !== -1) {
    const closeBrace = matchingBraceIndex(text, nextBrace);
    if (closeBrace !== -1) {
      return { start: attrIndex, end: closeBrace + 1 };
    }
  }
  const nextLine = text.indexOf('\n', index);
  if (nextLine !== -1) {
    return { start: attrIndex, end: nextLine + 1 };
  }
  return { start: attrIndex, end: text.length };
}

function matchingBraceIndex(text, openBrace) {
  let depth = 0;
  for (let index = openBrace; index < text.length; index += 1) {
    const char = text[index];
    if (char === '{') {
      depth += 1;
    } else if (char === '}') {
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
  for (let cursor = 0; cursor < index; cursor += 1) {
    if (text.charCodeAt(cursor) === 10) {
      line += 1;
    }
  }
  return line;
}

function parseArgs(argv) {
  const parsed = {
    help: false,
    selfTest: false,
  };

  for (const arg of argv) {
    if (arg === '-h' || arg === '--help') {
      parsed.help = true;
      continue;
    }
    if (arg === '--self-test') {
      parsed.selfTest = true;
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }

  return parsed;
}

function printUsage() {
  console.log(`Usage: node scripts/check-operation-abi-identity-single-source.mjs [--self-test]

Checks that artifact-identity/src/lib.rs is the only production owner of
OperationAbiIdentityInput and operation_abi_identity byte projection logic.`);
}

function normalizePath(path) {
  return path.split('\\').join('/');
}
