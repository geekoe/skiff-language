import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const sourceCompileDownstreamStageImports = crateModuleImportRegexp([
  'lowering',
  'projection',
  'emission',
  'compiled',
]);
const loweringForbiddenImports = regexpUnion([
  crateModuleImportRegexp(['input', 'source_compile', 'compiled', 'projection', 'emission']),
  /\bskiff_compiler\b/,
  /\bskiff_compiler_(?:input_model|input|compiled|projection|emission)\b/,
]);
const compiledStageDriverImports = regexpUnion([
  crateModuleImportRegexp(['input', 'projection', 'emission']),
  crateSharedSubmoduleImportRegexp('parser'),
  /\bskiff_compiler\b/,
  /\bskiff_compiler_(?:input_model|input|projection|emission)\b/,
  /\bskiff_compiler_source::(?:\w+::)*parser\b/,
  /\bserde_yaml\b/,
  /\bstd\s*::\s*fs\b/,
  /\bstd\s*::\s*\{[^;]*?\bfs\b/,
]);
const projectionInputImports = crateModuleImportRegexp(['input']);
const projectionEmissionParserImports = crateSharedSubmoduleImportRegexp('parser');
const projectionProductionForbiddenImports = regexpUnion([
  crateModuleImportRegexp(['input', 'source_compile', 'lowering', 'compiled']),
  crateSharedSubmoduleImportRegexp('parser'),
  crateSharedSubmoduleImportRegexp('ast'),
  /\bskiff_syntax\b/,
  /\bskiff_compiler\b/,
  /\bskiff_compiler_(?:input_model|input|source|lowering|compiled|projection|emission)\b/,
  /\b(?:CompiledPublication|PackagePublication|SourceCompileModel|SourceSymbolKey|SourceDeclarationKind)\b/,
]);
const emissionProductionForbiddenImports = regexpUnion([
  crateModuleImportRegexp(['input', 'source_compile', 'lowering', 'compiled']),
  crateSharedSubmoduleImportRegexp('parser'),
  crateSharedSubmoduleImportRegexp('ast'),
  /\bskiff_syntax\b/,
  /\bskiff_compiler\b/,
  /\bskiff_compiler_(?:input_model|input|source|lowering|compiled|emission)\b/,
  /\b(?:CompiledPublication|PackagePublication|SourceCompileModel)\b/,
]);
const projectionInputForbiddenImports = regexpUnion([
  /\bskiff_syntax\b/,
  /\bskiff_compiler\b/,
  /\bskiff_compiler_(?:input_model|input|source|lowering|projection|emission|compiled)\b/,
  /\b(?:CompiledPublication|PackagePublication|SourceCompileModel|SourceSymbolKey|SourceDeclarationKind)\b/,
]);
const compilerCoreForbiddenImports = regexpUnion([
  /\bskiff_artifact_identity\b/,
  /\bserde_yaml\b/,
  /\bskiff_compiler\b/,
  /\bskiff_compiler_(?:input_model|input|source|lowering|projection_input|compiled|projection|emission)\b/,
  /\bstd\s*::\s*fs\b/,
  /\bstd\s*::\s*\{[^;]*?\bfs\b/,
  crateModuleImportRegexp([
    'input',
    'source_compile',
    'lowering',
    'compiled',
    'projection',
    'emission',
    'facade',
    'publication_error',
  ]),
]);

const denyRules = [
  {
    id: 'compiler_core_no_forbidden_imports',
    owner: 'compiler-core',
    phase: '2',
    roots: ['compiler/core/src'],
    pattern:
      'skiff_artifact_identity|serde_yaml|std::fs|skiff_compiler facade/stage crates|crate::stage modules',
    regexp: compilerCoreForbiddenImports,
    remove_when: 'compiler-core contains only pure cross-stage support and no IO, YAML, facade, or stage dependencies',
  },
  {
    id: 'source_compile_no_downstream_stage_imports',
    owner: 'source_compile',
    phase: 'final',
    roots: ['compiler/source/src'],
    pattern: 'crate::(lowering|projection|emission|compiled)',
    regexp: sourceCompileDownstreamStageImports,
    remove_when: 'source_compile consumes only input, shared, skiff_artifact_model, and its own typed models',
  },
  {
    id: 'lowering_no_forbidden_imports',
    owner: 'lowering',
    phase: 'final',
    roots: ['compiler/lowering/src'],
    pattern: 'input/input-model/compiled/projection/emission/facade dependencies',
    regexp: loweringForbiddenImports,
    remove_when: 'lowering consumes SourceCompileModel and source/core/syntax/artifact-model APIs only',
  },
  {
    id: 'compiled_no_stage_driver_imports',
    owner: 'compiled',
    phase: 'final',
    roots: ['compiler/compiled/src'],
    pattern:
      'facade/input/projection/emission/parser/serde_yaml/std::fs dependencies',
    regexp: compiledStageDriverImports,
    remove_when: 'compiled remains a typed combiner and pipeline owns input/projection/emission orchestration',
  },
  {
    id: 'projection_no_input_imports',
    owner: 'projection',
    phase: 'final',
    roots: ['compiler/projection/src'],
    pattern: 'crate::input',
    regexp: projectionInputImports,
    remove_when: 'projection consumes ProjectionInput/ProjectionView and explicit ProjectionContext DTOs only',
  },
  {
    id: 'projection_no_upstream_stage_imports_phase_7_5',
    owner: 'projection',
    phase: '7.5',
    roots: ['compiler/projection/src'],
    pattern:
      'facade/compiled/source/source_compile/lowering/input/input-model/parser/AST production dependencies',
    regexp: projectionProductionForbiddenImports,
    remove_when:
      'projection crate keeps ProjectionInput-only production entrypoints and no upstream stage references',
  },
  {
    id: 'emission_no_upstream_stage_imports_phase_7_5',
    owner: 'emission',
    phase: '7.5',
    roots: ['compiler/emission/src'],
    pattern:
      'compiled/source/source_compile/lowering/input/input-model/parser/AST production dependencies',
    regexp: emissionProductionForbiddenImports,
    remove_when:
      'Phase 9 extracts emission crate consuming projection output/context without upstream stage references',
  },
  {
    id: 'projection_input_no_forbidden_stage_imports_phase_7_5',
    owner: 'projection-input',
    phase: '7.5',
    roots: ['compiler/projection-input/src'],
    pattern: 'facade/source/lowering/compiled/projection/emission/input/syntax/parser/AST dependencies',
    regexp: projectionInputForbiddenImports,
    remove_when: 'projection-input remains a pure DTO crate depending only on core/artifact-model/value crates',
  },
  {
    id: 'projection_emission_no_parser_imports',
    owner: 'projection/emission',
    phase: 'final',
    roots: ['compiler/projection/src', 'compiler/emission/src'],
    pattern: 'crate::shared::parser',
    regexp: projectionEmissionParserImports,
    remove_when: 'projection/emission consume typed compiler outputs rather than parsing source text',
  },
];

const transitionalLedger = [];

for (const entry of transitionalLedger) {
  for (const key of ['phase', 'owner', 'pattern', 'remove_when']) {
    if (entry[key] === undefined || entry[key] === '') {
      throw new Error(`transitional ledger entry ${entry.id} is missing ${key}`);
    }
  }
}

const rustFiles = await collectCandidateRustFiles(root);
const denials = [];
const warnings = [];

for (const rule of denyRules) {
  denials.push(...(await collectMatches(rule, rustFiles, 'deny')));
}
denials.push(...(await collectProjectionInputPurityViolations(rustFiles)));
denials.push(...(await collectDuplicateOperationAbiIdentityViolations(root)));

for (const entry of transitionalLedger) {
  warnings.push(...(await collectMatches(entry, rustFiles, 'warn')));
}

for (const warning of warnings) {
  console.warn(formatMatch('WARN', warning));
}

for (const denial of denials) {
  console.error(formatMatch('DENY', denial));
}

if (warnings.length === 0 && denials.length === 0) {
  console.log('Compiler boundary check passed with no known violations.');
} else {
  console.log(
    `Compiler boundary check completed: ${denials.length} deny violation(s), ${warnings.length} transitional warning(s).`,
  );
}

if (denials.length > 0) {
  process.exitCode = 1;
}

async function collectMatches(rule, files, severity) {
  const matches = [];
  const rootPrefixes = rule.roots.map((ruleRoot) => normalizePath(ruleRoot));
  for (const file of files) {
    if (
      !rootPrefixes.some(
        (prefix) => file.relPath === prefix || file.relPath.startsWith(`${prefix}/`),
      )
    ) {
      continue;
    }
    const text = stripInlineTestModules(await readFile(file.absPath, 'utf8'));
    for (const match of text.matchAll(rule.regexp)) {
      const line = lineNumberAt(text, match.index ?? 0);
      matches.push({
        ...rule,
        severity,
        relPath: file.relPath,
        line,
        matched: match[0],
      });
    }
  }
  return matches;
}

function projectionInputAllowedPublicMethodNames() {
  return new Set([
    'abi_ids',
    'access',
    'alias',
    'api_entries',
    'api_source',
    'collection_name_mapping',
    'compiled',
    'config',
    'config_requirements',
    'constraints',
    'content_hash',
    'declaring_publication',
    'dependencies',
    'dependency',
    'dependency_lock',
    'dependency_path',
    'effective',
    'entrypoint_abi',
    'executable',
    'export_bindings',
    'file_ir_units',
    'function',
    'function_signature',
    'has_type',
    'http',
    'id',
    'instance',
    'interface',
    'kind',
    'legacy',
    'lowering',
    'manifest',
    'module',
    'module_exports',
    'module_path',
    'new',
    'own',
    'package_entrypoints',
    'path',
    'provenance',
    'provenances',
    'public_callables',
    'public_instances',
    'public_schema_types',
    'public_symbols',
    'publication_api_seed',
    'relative_path',
    'requirements',
    'schema_abi_types_for_module',
    'schema_type_names_for_module',
    'scope',
    'service_actor_metadata',
    'service_db_metadata',
    'service_dependencies',
    'service_ingress',
    'signature',
    'source',
    'source_metadata',
    'source_path',
    'source_root',
    'source_span',
    'symbol',
    'synthetic',
    'synthetic_entrypoints',
    'version',
    'view',
    'websocket',
  ]);
}

function projectionInputDeniedBehaviorMethodNames() {
  return new Set([
    'derive_projection_abi_ids',
    'effective_alias',
    'from_file_ir_units',
    'has_service_storage_metadata',
    'is_has',
    'response_type_ir',
    'signature_with_name',
    'source_text_with_named_types',
    'typed',
    'to_source_symbol',
    'type_ref_ir_source_text_with_local_types',
    'type_ref_source_text',
  ]);
}

function projectionInputDeniedBehaviorMethodsByImpl() {
  return new Map([
    ['ConfigRequirementProjection', new Set(['source_path'])],
  ]);
}

async function collectProjectionInputPurityViolations(files) {
  const matches = [];
  const projectionInputAllowedPublicMethods = projectionInputAllowedPublicMethodNames();
  const projectionInputDeniedBehaviorMethods = projectionInputDeniedBehaviorMethodNames();
  const projectionInputImplDeniedMethods = projectionInputDeniedBehaviorMethodsByImpl();
  for (const file of files) {
    if (!file.relPath.startsWith('compiler/projection-input/src/')) {
      continue;
    }
    const text = stripInlineTestModules(await readFile(file.absPath, 'utf8'));
    for (const match of text.matchAll(/^pub\s+fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/gm)) {
      matches.push(projectionInputPurityMatch(file, text, match, 'public free functions'));
    }
    for (const match of text.matchAll(/^\s+pub\s+fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/gm)) {
      const methodName = match[1];
      const implName = projectionInputImplNameAt(text, match.index ?? 0);
      const implDeniedMethods = projectionInputImplDeniedMethods.get(implName) ?? new Set();
      if (
        projectionInputDeniedBehaviorMethods.has(methodName)
        || implDeniedMethods.has(methodName)
        || !projectionInputAllowedPublicMethods.has(methodName)
      ) {
        matches.push(projectionInputPurityMatch(file, text, match, 'public behavior methods'));
      }
    }
  }
  return matches;
}

async function collectDuplicateOperationAbiIdentityViolations(repoRoot) {
  const files = [];
  await collectRustFiles(join(repoRoot, 'artifact-identity/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler'), files);
  const matches = [];
  const regexp = /\bstruct\s+OperationAbiIdentityInput\b|\bfn\s+operation_abi_identity\s*\(/g;

  for (const file of files) {
    if (
      file.relPath === 'artifact-identity/src/lib.rs'
      || !isOperationAbiIdentityGuardProductionFile(file.relPath)
    ) {
      continue;
    }
    const text = stripInlineTestModules(await readFile(file.absPath, 'utf8'));
    for (const match of text.matchAll(regexp)) {
      matches.push({
        id: 'operation_abi_identity_single_source',
        owner: 'artifact-identity',
        phase: 'final',
        pattern: 'struct OperationAbiIdentityInput or fn operation_abi_identity outside artifact-identity',
        regexp,
        remove_when:
          'artifact-identity remains the only production source of operation ABI identity byte projection',
        severity: 'deny',
        relPath: file.relPath,
        line: lineNumberAt(text, match.index ?? 0),
        matched: match[0],
      });
    }
  }

  return matches;
}

function isOperationAbiIdentityGuardProductionFile(relPath) {
  if (relPath.startsWith('artifact-identity/src/')) {
    if (relPath.endsWith('/tests.rs')) {
      return false;
    }
    return !relPath.split('/').includes('tests');
  }
  return isProductionRustFile(relPath);
}

function projectionInputImplNameAt(text, index) {
  const implRegexp =
    /^\s*impl(?:<[^>{}]*>)?\s+([A-Za-z_][A-Za-z0-9_]*)(?:<[^>{}]*>)?\s*\{/gm;
  for (const match of text.matchAll(implRegexp)) {
    const openBrace = text.indexOf('{', match.index ?? 0);
    if (openBrace === -1 || openBrace > index) {
      continue;
    }
    const closeBrace = matchingBraceIndex(text, openBrace);
    if (closeBrace !== -1 && index < closeBrace) {
      return match[1];
    }
  }
  return undefined;
}

function projectionInputPurityMatch(file, text, match, pattern) {
  return {
    id: 'projection_input_pure_dto_api_phase_7_5',
    owner: 'projection-input',
    phase: '7.5',
    pattern,
    regexp: /projection-input DTO purity/,
    remove_when:
      'projection-input remains a narrow DTO handoff crate with behavior in compiled/projection/core',
    severity: 'deny',
    relPath: file.relPath,
    line: lineNumberAt(text, match.index ?? 0),
    matched: match[0],
  };
}

async function collectCandidateRustFiles(repoRoot) {
  const files = [];
  await collectRustFiles(join(repoRoot, 'compiler/core/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/source/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/lowering/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/projection-input/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/compiled/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/projection/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/emission/src'), files);
  await collectRustFiles(join(repoRoot, 'compiler/driver'), files);
  await collectRustFiles(join(repoRoot, 'compiler/tests'), files);
  return files.filter((file) => isProductionRustFile(file.relPath));
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
    });
  }
}

function isProductionRustFile(relPath) {
  if (relPath.startsWith('compiler/tests/')) {
    return false;
  }
  if (
    !relPath.startsWith('compiler/driver/')
    && !relPath.startsWith('compiler/core/src/')
    && !relPath.startsWith('compiler/source/src/')
    && !relPath.startsWith('compiler/lowering/src/')
    && !relPath.startsWith('compiler/projection-input/src/')
    && !relPath.startsWith('compiler/compiled/src/')
    && !relPath.startsWith('compiler/projection/src/')
    && !relPath.startsWith('compiler/emission/src/')
  ) {
    return false;
  }
  if (relPath.endsWith('/tests.rs')) {
    return false;
  }
  return !relPath.split('/').includes('tests');
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

function formatMatch(label, match) {
  return `${label} ${match.relPath}:${match.line} ${match.id} phase=${match.phase} owner=${match.owner} pattern="${match.pattern}" matched="${match.matched}" remove_when="${match.remove_when}"`;
}

function normalizePath(path) {
  return path.split('\\').join('/');
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

function crateModuleImportRegexp(modules) {
  const alternatives = modules.map(escapeRegExp).join('|');
  return new RegExp(
    [
      String.raw`\bcrate\s*::\s*(?:${alternatives})\s*(?:::|\b)`,
      String.raw`\bcrate\s*::\s*\{[^;]*?\b(?:${alternatives})\s*::`,
    ].join('|'),
    'g',
  );
}

function crateSharedSubmoduleImportRegexp(submodule) {
  const escaped = escapeRegExp(submodule);
  return new RegExp(
    [
      String.raw`\bcrate\s*::\s*shared\s*::\s*${escaped}\b`,
      String.raw`\bcrate\s*::\s*shared\s*::\s*\{[^;]*?\b${escaped}\s*::`,
      String.raw`\bcrate\s*::\s*\{[^;]*?\bshared\s*::\s*${escaped}\b`,
      String.raw`\bcrate\s*::\s*\{[^;]*?\bshared\s*::\s*\{[^;]*?\b${escaped}\s*::`,
    ].join('|'),
    'g',
  );
}

function regexpUnion(regexps) {
  return new RegExp(regexps.map((regexp) => regexp.source).join('|'), 'g');
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
