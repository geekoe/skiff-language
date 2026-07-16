#!/usr/bin/env node

import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const skippedRustScanDirectories = new Set([
  '.git',
  '.skiff-instance',
  'build',
  'node_modules',
  'target',
]);
const artifactIdentityFacadePath = 'artifact-identity/src/lib.rs';
const ownerRequirements = [
  {
    name: 'FileIrIdentityPayload',
    relPath: 'artifact-identity/src/file_ir.rs',
    regexp: /\bstruct\s+FileIrIdentityPayload\b/,
  },
  {
    name: 'file_ir_identity',
    relPath: 'artifact-identity/src/file_ir.rs',
    regexp: /\bpub\s+fn\s+file_ir_identity\s*\(/,
  },
  {
    name: 'canonical_file_ir_identity_bytes',
    relPath: 'artifact-identity/src/file_ir.rs',
    regexp: /\bpub\s+fn\s+canonical_file_ir_identity_bytes\s*\(/,
  },
  {
    name: 'ServiceUnitStorageIdentityPayload',
    relPath: 'artifact-identity/src/legacy_service.rs',
    regexp: /\bstruct\s+ServiceUnitStorageIdentityPayload\b/,
  },
  {
    name: 'service_unit_identity',
    relPath: 'artifact-identity/src/legacy_service.rs',
    regexp: /\bpub\s+fn\s+service_unit_identity\s*\(/,
  },
  {
    name: 'service_unit_identity_bytes',
    relPath: 'artifact-identity/src/legacy_service.rs',
    regexp: /\bpub\s+fn\s+service_unit_identity_bytes\s*\(/,
  },
  {
    name: 'PackageBuildIdentityPayload',
    relPath: 'artifact-identity/src/package.rs',
    regexp: /\bstruct\s+PackageBuildIdentityPayload\b/,
  },
  {
    name: 'package_build_identity',
    relPath: 'artifact-identity/src/package.rs',
    regexp: /\bpub\s+fn\s+package_build_identity\s*\(/,
  },
  {
    name: 'package_abi_identity',
    relPath: 'artifact-identity/src/package.rs',
    regexp: /\bpub\s+fn\s+package_abi_identity\s*\(/,
  },
  {
    name: 'PublicationAbiIdentityProjection',
    relPath: 'artifact-identity/src/publication.rs',
    regexp: /\bstruct\s+PublicationAbiIdentityProjection\b/,
  },
  {
    name: 'publication_abi_identity',
    relPath: 'artifact-identity/src/publication.rs',
    regexp: /\bpub\s+fn\s+publication_abi_identity\s*\(/,
  },
  {
    name: 'publication_abi_identity_bytes',
    relPath: 'artifact-identity/src/publication.rs',
    regexp: /\bpub\s+fn\s+publication_abi_identity_bytes\s*\(/,
  },
  {
    name: 'OperationAbiIdentityInput',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+struct\s+OperationAbiIdentityInput\b/,
  },
  {
    name: 'operation_abi_hash',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+operation_abi_hash\s*\(/,
  },
  {
    name: 'operation_abi_identity',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+operation_abi_identity\s*\(/,
  },
  {
    name: 'public_function_operation_abi_id',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+public_function_operation_abi_id\s*\(/,
  },
  {
    name: 'public_instance_method_operation_abi_id',
    relPath: 'artifact-identity/src/operation.rs',
    regexp: /\bpub\s+fn\s+public_instance_method_operation_abi_id\s*\(/,
  },
  {
    name: 'PackageTestBuildIdentityPayload',
    relPath: 'artifact-identity/src/package_test.rs',
    regexp: /\bstruct\s+PackageTestBuildIdentityPayload\b/,
  },
  {
    name: 'RuntimeProgramServiceUnitIdentityPayload',
    relPath: 'artifact-identity/src/runtime_program.rs',
    regexp: /\bstruct\s+RuntimeProgramServiceUnitIdentityPayload\b/,
  },
  {
    name: 'canonical_json_value',
    relPath: 'canonical-json/src/lib.rs',
    regexp: /\bpub\s+fn\s+canonical_json_value\s*\(/,
  },
  {
    name: 'canonical_json_number',
    relPath: 'canonical-json/src/lib.rs',
    regexp: /\bpub\s+fn\s+canonical_json_number\s*\(/,
  },
  {
    name: 'canonical_json_bytes',
    relPath: 'canonical-json/src/lib.rs',
    regexp: /\bpub\s+fn\s+canonical_json_bytes\s*</,
  },
];

const exclusiveDefinitionNames = new Set([
  'FileIrIdentityPayload',
  'ServiceUnitStorageIdentityPayload',
  'PackageBuildIdentityPayload',
  'PublicationAbiIdentityProjection',
  'OperationAbiIdentityInput',
  'PackageTestBuildIdentityPayload',
  'RuntimeProgramServiceUnitIdentityPayload',
  'canonical_file_ir_identity_bytes',
  'service_unit_identity_bytes',
  'publication_abi_identity_bytes',
  'operation_abi_identity',
  'canonical_json_value',
  'canonical_json_number',
  'canonical_json_bytes',
]);
const definitionOwnerByName = new Map(
  ownerRequirements
    .filter(({ name }) => exclusiveDefinitionNames.has(name))
    .map(({ name, relPath }) => [name, relPath]),
);
const ownedDefinitionRegexp = new RegExp(
  `\\b(?:struct|fn)\\s+(${[...definitionOwnerByName.keys()].join('|')})\\b`,
  'g',
);

const facadeModules = [
  'constants',
  'error',
  'file_ir',
  'framing',
  'legacy_service',
  'operation',
  'package',
  'package_test',
  'publication',
  'runtime_program',
];

const canonicalDelegationRequirements = [
  {
    relPath: 'artifact-identity/src/framing.rs',
    helper: 'artifact identity canonical bytes',
    regexp: /\bskiff_canonical_json::canonical_json_bytes\b/,
  },
  {
    relPath: 'compiler/core/src/json_utils.rs',
    helper: 'compiler canonical JSON API',
    regexp: /\bpub\s+use\s+skiff_canonical_json\s*::/,
  },
  {
    relPath: 'runtime/linker/src/json_utils.rs',
    helper: 'runtime linker canonical JSON API',
    regexp: /\buse\s+skiff_canonical_json::canonical_json_value\b/,
  },
  {
    relPath: 'runtime/linked-type-plan/src/type_plan.rs',
    helper: 'sort-only linked type key helper',
    regexp: /\bfn\s+sort_json_value\s*\(/,
  },
];

const adapterRequirements = [
  {
    relPath: 'compiler/lowering/src/file_ir/identity.rs',
    helper: 'File IR identity',
    regexp: /\bskiff_artifact_identity::file_ir_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'File IR identity',
    regexp: /\bskiff_artifact_identity::file_ir_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'service-unit identity',
    regexp: /\bskiff_artifact_identity::service_unit_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'package build identity',
    regexp: /\bskiff_artifact_identity::package_build_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'package ABI identity',
    regexp: /\bskiff_artifact_identity::package_abi_identity\b/,
  },
  {
    relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
    helper: 'publication ABI identity',
    regexp: /\bskiff_artifact_identity::publication_abi_identity\b/,
  },
  {
    relPath: 'compiler/publication-abi/src/lib.rs',
    helper: 'public function operation ABI identity',
    regexp: /\bskiff_artifact_identity::public_function_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/publication-abi/src/lib.rs',
    helper: 'public instance operation ABI identity',
    regexp: /\bskiff_artifact_identity::public_instance_method_operation_abi_id\b/,
  },
  {
    relPath: 'compiler/emission/src/emission/identity.rs',
    helper: 'artifact identity emission API',
    regexp: /\bpub\s+use\s+skiff_artifact_identity\s*::/,
  },
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
  const ownerTextByPath = new Map();
  for (const requirement of ownerRequirements) {
    if (!ownerTextByPath.has(requirement.relPath)) {
      ownerTextByPath.set(
        requirement.relPath,
        stripInlineTestModules(await readFile(join(root, requirement.relPath), 'utf8')),
      );
    }
    if (!requirement.regexp.test(ownerTextByPath.get(requirement.relPath))) {
      failures.push(`${requirement.relPath} is missing owned ${requirement.name}`);
    }
  }

  const facadeText = stripInlineTestModules(
    await readFile(join(root, artifactIdentityFacadePath), 'utf8'),
  );
  for (const moduleName of facadeModules) {
    const moduleDeclaration = new RegExp(`\\bmod\\s+${moduleName}\\s*;`);
    if (!moduleDeclaration.test(facadeText)) {
      failures.push(`${artifactIdentityFacadePath} is missing ${moduleName} module declaration`);
    }
  }
  if (/\b(?:struct|enum|fn)\s+\w+/.test(facadeText)) {
    failures.push(`${artifactIdentityFacadePath} must contain declarations and re-exports only`);
  }

  const adapterTextByPath = new Map();
  for (const { relPath } of adapterRequirements) {
    if (!adapterTextByPath.has(relPath)) {
      adapterTextByPath.set(relPath, await readFile(join(root, relPath), 'utf8'));
    }
  }
  failures.push(...collectAdapterRequirementFailures(adapterRequirements, adapterTextByPath));

  const canonicalDelegationTextByPath = new Map();
  for (const { relPath } of canonicalDelegationRequirements) {
    if (!canonicalDelegationTextByPath.has(relPath)) {
      canonicalDelegationTextByPath.set(relPath, await readFile(join(root, relPath), 'utf8'));
    }
  }
  failures.push(
    ...collectDelegationRequirementFailures(
      canonicalDelegationRequirements,
      canonicalDelegationTextByPath,
    ),
  );

  for (const violation of collectOwnedDefinitionViolations(files)) {
    failures.push(
      `${violation.relPath}:${violation.line} ${violation.name} is owned by ${violation.owner}`,
    );
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    process.exitCode = 1;
    return;
  }

  console.log('Artifact identity single-source check passed.');
}

function collectDelegationRequirementFailures(requirements, textByPath) {
  const failures = [];
  for (const requirement of requirements) {
    const text = textByPath.get(requirement.relPath);
    if (text === undefined) {
      failures.push(`${requirement.relPath} is missing required ${requirement.helper}`);
      continue;
    }
    if (!requirement.regexp.test(stripInlineTestModules(text))) {
      failures.push(`${requirement.relPath} is missing required ${requirement.helper} delegation`);
    }
  }
  return failures;
}

function collectAdapterRequirementFailures(requirements, textByPath) {
  const failures = [];
  for (const requirement of requirements) {
    const text = textByPath.get(requirement.relPath);
    if (text === undefined) {
      failures.push(`${requirement.relPath} is missing required ${requirement.helper} adapter`);
      continue;
    }
    const productionText = stripInlineTestModules(text);
    if (!requirement.regexp.test(productionText)) {
      failures.push(
        `${requirement.relPath} must delegate ${requirement.helper} to skiff_artifact_identity`,
      );
    }
  }
  return failures;
}

function collectOwnedDefinitionViolations(files) {
  const violations = [];

  for (const file of files) {
    if (!isProductionRustFile(file.relPath)) {
      continue;
    }
    const text = stripInlineTestModules(file.text);
    for (const match of text.matchAll(ownedDefinitionRegexp)) {
      const name = match[1];
      const owner = definitionOwnerByName.get(name);
      if (owner === file.relPath) {
        continue;
      }
      violations.push({
        relPath: file.relPath,
        line: lineNumberAt(text, match.index ?? 0),
        name,
        owner,
      });
    }
  }

  return violations;
}

async function collectCandidateRustFiles(repoRoot) {
  const files = [];
  await collectRustFiles(repoRoot, files);
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
      if (shouldSkipRustScanDirectory(entry.name)) {
        continue;
      }
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

function shouldSkipRustScanDirectory(name) {
  return skippedRustScanDirectories.has(name);
}

function isProductionRustFile(relPath) {
  if (relPath.endsWith('/tests.rs') || relPath.split('/').includes('tests')) {
    return false;
  }
  return relPath.endsWith('.rs');
}

function runSelfTest() {
  const cases = [
    {
      name: 'allows definitions in their declared owner modules',
      files: [
        {
          relPath: 'artifact-identity/src/operation.rs',
          text: 'pub struct OperationAbiIdentityInput;\npub fn operation_abi_identity() {}\n',
        },
        {
          relPath: 'canonical-json/src/lib.rs',
          text: 'pub fn canonical_json_value() {}\n',
        },
      ],
      expectedViolations: 0,
    },
    {
      name: 'rejects compiler operation identity duplicate struct',
      files: [
        {
          relPath: 'compiler/driver/shared/operation_abi_identity.rs',
          text: 'struct OperationAbiIdentityInput;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects lowering File IR payload duplicate',
      files: [
        {
          relPath: 'compiler/lowering/src/file_ir/identity.rs',
          text: 'struct FileIrIdentityPayload;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects projection package identity payload duplicate',
      files: [
        {
          relPath: 'compiler/projection/src/typed_artifacts/identity.rs',
          text: 'struct PackageBuildIdentityPayload;\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects publication ABI byte projection duplicate',
      files: [
        {
          relPath: 'compiler/publication-abi/src/identity.rs',
          text: 'fn publication_abi_identity_bytes() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects an identity definition in the wrong artifact-identity module',
      files: [
        {
          relPath: 'artifact-identity/src/other.rs',
          text: 'fn operation_abi_identity() {}\n',
        },
      ],
      expectedViolations: 1,
    },
    {
      name: 'rejects a canonical JSON definition outside the leaf owner',
      files: [
        {
          relPath: 'runtime/linker/src/json_utils.rs',
          text: 'fn canonical_json_value() {}\n',
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
    const violations = collectOwnedDefinitionViolations(testCase.files);
    if (violations.length !== testCase.expectedViolations) {
      failures.push(
        `${testCase.name}: expected ${testCase.expectedViolations} violation(s), got ${violations.length}`,
      );
    }
  }

  const adapterFixtureRequirement = [
    {
      relPath: 'compiler/example/src/identity.rs',
      helper: 'fixture identity',
      regexp: /\bskiff_artifact_identity::file_ir_identity\b/,
    },
  ];
  const testOnlyAdapterFailures = collectAdapterRequirementFailures(
    adapterFixtureRequirement,
    new Map([
      [
        'compiler/example/src/identity.rs',
        `#[cfg(test)]
mod tests {
  fn parity() {
    skiff_artifact_identity::file_ir_identity();
  }
}
`,
      ],
    ]),
  );
  if (testOnlyAdapterFailures.length !== 1) {
    failures.push(
      `rejects test-only adapter delegation: expected 1 failure, got ${testOnlyAdapterFailures.length}`,
    );
  }

  const productionAdapterFailures = collectAdapterRequirementFailures(
    adapterFixtureRequirement,
    new Map([
      [
        'compiler/example/src/identity.rs',
        `fn identity() {
  skiff_artifact_identity::file_ir_identity();
}
`,
      ],
    ]),
  );
  if (productionAdapterFailures.length !== 0) {
    failures.push(
      `allows production adapter delegation: expected 0 failures, got ${productionAdapterFailures.length}`,
    );
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    process.exitCode = 1;
    return;
  }

  console.log('Artifact identity single-source self-test passed.');
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
  console.log(`Usage: node scripts/check-artifact-identity-single-source.mjs [--self-test]

Checks that canonical JSON and artifact identity definitions live in their declared
crate/module owners while compiler and runtime consumers use the public owner APIs.`);
}

function normalizePath(path) {
  return path.split('\\').join('/');
}
