import { readdir, readFile, stat } from 'node:fs/promises';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  collectTerminalPublicShapeViolations,
  runTerminalPublicShapeSelfTest,
  terminalPublicShapeRegistry,
} from './lib/compiler-terminal-public-shape.mjs';

const defaultRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const terminalPublicShapeTools = {
  implNameAt: projectionInputImplNameAt,
  lineNumberAt,
  matchingBraceIndex,
  publicFunctionDeclarations: projectionInputPublicFunctionDeclarations,
  readText: async (file) =>
    stripInlineTestModules(file.text ?? (await readFile(file.absPath, 'utf8'))),
};
const projectionInputFrozenFreeFunctions = new Set(
  terminalPublicShapeRegistry.find((entry) => entry.owner === 'projection-input').publicItems.fn,
);

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
  /\bskiff_compiler_(?:input_model|input|source|compiled|emission)\b/,
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
const terminalCompilerLegacyShape = regexpUnion([
  /\b(?:PublicationInput(?:Kind|Error)?|PublicationKind|CompiledPublication|LoweredPublication)\b/,
  /\b(?:PublicationAbiUnit|PackageUnit|ServiceUnit|ServiceAssembly)\b/,
  /\b(?:package_unit|service_unit|service_assembly|serviceAssembly)\b/,
  /\b(?:compile|project|emit|assemble)_(?:publication|service_(?:publication|unit|assembly))(?:_[A-Za-z0-9_]+)?\b/,
  /\b[A-Za-z0-9_]*ServicePublication[A-Za-z0-9_]*\b/,
  /\b[A-Za-z0-9_]*service(?:_[a-z0-9]+)*_publication[A-Za-z0-9_]*\b/,
  /\bPublicationServiceFacade\b/,
  /\b(?:Legacy|Compat|Compatibility)(?:Publication|Compile|Compiler|Package|Service)?(?:Adapter|Facade|Provider|Input|Output)\b/,
  /\b(?:legacy|compat|compatibility)(?:_[a-z0-9]+)*_(?:adapter|facade|provider|input|output)\b/,
  /\b(?:infer|resolve|select)_(?:service_)?provider(?:_[A-Za-z0-9_]+)?\b/,
  /\b(?:ProviderInference|ProviderResolver|InferredProvider|ResolvedProvider)\b/,
  /\bprovider_(?:inference|resolver|selection)\b/,
]);

const denyRules = [
  {
    id: 'terminal_compiler_shape_no_legacy_publication_or_provider_paths',
    owner: 'terminal-package-and-contract-producers',
    phase: '2',
    roots: ['compiler'],
    pattern:
      'legacy publication sum types, PackageUnit/ServiceUnit/service assembly producers, service publication facades, compatibility adapters, or provider inference',
    regexp: terminalCompilerLegacyShape,
    remove_when:
      'compiler public shape remains exactly the package compile and code-free contract compile producers',
  },
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
      'crate::lowering or compiled/source/source_compile/input/input-model/parser/AST production dependencies',
    regexp: emissionProductionForbiddenImports,
    remove_when:
      'emission consumes the frozen lowering MIR crate plus projection output/context without monolith or other upstream stage references',
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

const options = parseArgs(process.argv.slice(2));
if (options.help) {
  printUsage();
} else if (options.selfTest) {
  await runTerminalPublicShapeSelfTest(terminalPublicShapeTools);
} else {
  await assertRootDirectory(options.root);
  await runCheck(options.root);
}

async function runCheck(repoRoot) {
  const rustFiles = await collectCandidateRustFiles(repoRoot);
  const denials = [];
  const warnings = [];

  for (const rule of denyRules) {
    denials.push(...(await collectMatches(rule, rustFiles, 'deny')));
  }
  denials.push(...(await collectProjectionInputPurityViolations(rustFiles)));
  denials.push(
    ...(await collectTerminalPublicShapeViolations(rustFiles, terminalPublicShapeTools)),
  );

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
    const text = stripInlineTestModules(file.text ?? (await readFile(file.absPath, 'utf8')));
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

function projectionInputDeniedBehaviorMethodsByImpl() {
  return new Map([
    ['ProjectionSourceFacts', new Set(['derive_projection_abi_ids'])],
    ['PackageDependencyProjectionInfo', new Set(['effective_alias'])],
    ['ProjectionEntrypointAbiIndex', new Set(['from_file_ir_units'])],
    ['ProjectionLoweringFacts', new Set(['has_service_storage_metadata'])],
    ['ConfigRequirementProjection', new Set(['is_has', 'typed', 'source_path'])],
    [
      'EntryTypeSpec',
      new Set([
        'response_type_ir',
        'source_text_with_named_types',
        'type_ref_ir_source_text_with_local_types',
        'type_ref_source_text',
      ]),
    ],
    ['EntryFunctionSignature', new Set(['signature_with_name'])],
    ['ProjectionSourceSymbolKey', new Set(['to_source_symbol'])],
  ]);
}

async function collectProjectionInputPurityViolations(files) {
  const matches = [];
  const projectionInputImplDeniedMethods = projectionInputDeniedBehaviorMethodsByImpl();
  for (const file of files) {
    if (!file.relPath.startsWith('compiler/projection-input/src/')) {
      continue;
    }
    const text = stripInlineTestModules(file.text ?? (await readFile(file.absPath, 'utf8')));
    for (const declaration of projectionInputPublicFunctionDeclarations(text)) {
      const implName = projectionInputImplNameAt(text, declaration.index);
      if (implName === undefined) {
        if (projectionInputFrozenFreeFunctions.has(declaration.name)) {
          continue;
        }
        matches.push(
          projectionInputPurityMatch(
            file,
            text,
            declaration,
            'public free functions',
          ),
        );
        continue;
      }
      const implDeniedMethods = projectionInputImplDeniedMethods.get(implName) ?? new Set();
      if (implDeniedMethods.has(declaration.name)) {
        matches.push(
          projectionInputPurityMatch(
            file,
            text,
            declaration,
            'known non-DTO public behavior',
          ),
        );
      }
    }
  }
  return matches;
}

function projectionInputPublicFunctionDeclarations(text) {
  const declarations = [];
  const regexp =
    /^[ \t]*(?:#\[[^\r\n]*\][ \t]*(?:\r?\n[ \t]*)?)*pub[ \t]+(?:(?:const|async|unsafe)[ \t]+)*(?:extern(?:[ \t]+"[^"\r\n]*")?[ \t]+)?fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)(?:[ \t]*<[^>{}\r\n]*>)?[ \t]*\(/gm;
  for (const match of text.matchAll(regexp)) {
    const matched = match[0];
    const pubOffset = matched.lastIndexOf('pub');
    declarations.push({
      name: match[1],
      index: (match.index ?? 0) + pubOffset,
      matched: matched.slice(pubOffset),
    });
  }
  return declarations;
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

function projectionInputPurityMatch(file, text, declaration, pattern) {
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
    line: lineNumberAt(text, declaration.index),
    matched: declaration.matched,
  };
}

async function collectCandidateRustFiles(repoRoot) {
  const files = [];
  await collectRustFiles(join(repoRoot, 'compiler'), files, repoRoot);
  await Promise.all(
    files.map(async (file) => {
      file.text = await readFile(file.absPath, 'utf8');
    }),
  );
  const cfgTestOnlyFiles = collectCfgTestOnlyModuleFiles(files);
  return files.filter(
    (file) => isProductionRustFile(file.relPath) && !cfgTestOnlyFiles.has(file.relPath),
  );
}

async function collectRustFiles(directory, files, repoRoot) {
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
      await collectRustFiles(absPath, files, repoRoot);
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

function collectCfgTestOnlyModuleFiles(files) {
  const productionReachable = 1;
  const testReachable = 2;
  const filesByPath = new Map(files.map((file) => [normalizePath(resolve(file.absPath)), file]));
  const reachability = new Map();
  const queue = [];

  const enqueue = (file, state) => {
    const current = reachability.get(file.relPath) ?? 0;
    if ((current & state) !== 0) {
      return;
    }
    reachability.set(file.relPath, current | state);
    queue.push({ file, state });
  };

  for (const file of files) {
    if (isRustCrateRoot(file.relPath)) {
      enqueue(file, productionReachable);
    }
  }

  while (queue.length > 0) {
    const { file, state } = queue.shift();
    for (const declaration of externalRustModuleDeclarations(file.text)) {
      const child = resolveExternalRustModule(file, declaration, filesByPath);
      if (child === undefined) {
        continue;
      }
      const childState = state === testReachable || declaration.cfgTest
        ? testReachable
        : productionReachable;
      enqueue(child, childState);
    }
  }

  return new Set(
    [...reachability.entries()]
      .filter(([, state]) => state === testReachable)
      .map(([relPath]) => relPath),
  );
}

function isRustCrateRoot(relPath) {
  const fileName = basename(relPath);
  return fileName === 'lib.rs'
    || fileName === 'main.rs'
    || /\/src\/bin\/[^/]+\.rs$/.test(relPath);
}

function externalRustModuleDeclarations(text) {
  const declarations = [];
  const regexp =
    /((?:^[ \t]*#\[[^\r\n]*\][ \t]*\r?\n)*)^[ \t]*(?:pub(?:\([^\r\n)]*\))?[ \t]+)?mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;/gm;
  for (const match of text.matchAll(regexp)) {
    const attributes = match[1];
    declarations.push({
      cfgTest: /#\[\s*cfg\s*\(\s*test\s*\)\s*\]/.test(attributes),
      name: match[2],
      path: /#\[\s*path\s*=\s*"([^"]+)"\s*\]/.exec(attributes)?.[1],
    });
  }
  return declarations;
}

function resolveExternalRustModule(file, declaration, filesByPath) {
  const candidates = [];
  if (declaration.path !== undefined) {
    candidates.push(resolve(dirname(file.absPath), declaration.path));
  } else {
    const fileName = basename(file.absPath);
    const moduleDirectory = ['lib.rs', 'main.rs', 'mod.rs'].includes(fileName)
      ? dirname(file.absPath)
      : join(dirname(file.absPath), fileName.slice(0, -'.rs'.length));
    candidates.push(
      join(moduleDirectory, `${declaration.name}.rs`),
      join(moduleDirectory, declaration.name, 'mod.rs'),
    );
  }
  for (const candidate of candidates) {
    const child = filesByPath.get(normalizePath(resolve(candidate)));
    if (child !== undefined) {
      return child;
    }
  }
  return undefined;
}

function isProductionRustFile(relPath) {
  if (!relPath.startsWith('compiler/') || relPath.startsWith('compiler/tests/')) {
    return false;
  }
  if (relPath.endsWith('/tests.rs')) {
    return false;
  }
  return !relPath.split('/').some((part) => part === 'tests' || part === 'test_support' || part === 'test_support.rs');
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

function parseArgs(argv) {
  const options = { help: false, root: defaultRoot, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
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
      const value = arg.slice('--root='.length);
      if (!value) {
        throw new Error('--root requires a directory path');
      }
      options.root = resolve(value);
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  return options;
}

async function assertRootDirectory(repoRoot) {
  let rootStat;
  try {
    rootStat = await stat(repoRoot);
  } catch (error) {
    if (error && error.code === 'ENOENT') {
      throw new Error(`compiler boundary root does not exist: ${repoRoot}`);
    }
    throw error;
  }
  if (!rootStat.isDirectory()) {
    throw new Error(`compiler boundary root is not a directory: ${repoRoot}`);
  }
}

function printUsage() {
  console.log(`Usage: node scripts/check-compiler-boundaries.mjs [--root <path>] [--self-test]

Checks compiler production source boundaries. --root is reserved for hermetic fixtures.
--self-test runs registry-derived terminal public-shape mutations without scanning the repository.`);
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
