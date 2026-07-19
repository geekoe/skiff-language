const terminalPublicItemKinds = [
  'const',
  'enum',
  'fn',
  'mod',
  'static',
  'struct',
  'trait',
  'type',
  'union',
];

export const terminalPublicShapeRegistry = [
  {
    owner: 'compiled',
    root: 'compiler/compiled/src',
    publicItems: {
      struct: ['CompiledPackage'],
      enum: ['ProjectionInputBuildError'],
      fn: [
        'build_projection_input',
        'compile_parsed_publication_sources',
        'compile_source_model',
      ],
      const: [],
      mod: ['projection_input'],
      static: [],
      trait: [],
      type: [],
      union: [],
    },
    publicExports: ['ProjectionInputBuildError'],
    structFields: {
      CompiledPackage: ['lowered', 'model'],
    },
    handoffs: [
      {
        name: 'build_projection_input',
        parameterName: 'compiled',
        parameterType: '&CompiledPackage',
        returnType: 'Result<ProjectionInput, ProjectionInputBuildError>',
      },
    ],
  },
  {
    owner: 'projection-input',
    root: 'compiler/projection-input/src',
    publicItems: {
      struct: [
        'ConfigRequirementDependencyStepProjection',
        'ConfigRequirementProjection',
        'ConfigRequirementProvenanceProjection',
        'ConfigRequirementPublicationProjection',
        'ConfigRequirementSetProjection',
        'ConfigRequirementsSeed',
        'ConfigSourcePositionProjection',
        'ConfigSourceSpanProjection',
        'DuplicateProjectionPackageCallableSignature',
        'EntryFunctionSignature',
        'EntryParamSpec',
        'EntryTypeSpec',
        'ExportBindingProjection',
        'ExportCallableProjection',
        'ExportPublicInstanceInterfaceProjection',
        'ExportPublicInstanceMethodProjection',
        'ExportPublicInstanceProjection',
        'ExportSchemaProjection',
        'ExportSymbolProjection',
        'PackageAbiType',
        'PackageEntrypointFunctionProjection',
        'PackageEntrypointProjectionFacts',
        'ProjectionAbiDeclarationIds',
        'ProjectionCallableEffectFacts',
        'ProjectionDeclarationKey',
        'ProjectionEntrypointAbiIndex',
        'ProjectionExecutableKey',
        'ProjectionFileArtifactSource',
        'ProjectionInput',
        'ProjectionLoweringFacts',
        'ProjectionPackageCallableKey',
        'ProjectionPackageCallableSignatureFacts',
        'ProjectionSourceFacts',
        'ProjectionSourceFactsParts',
        'ProjectionSourceMetadata',
        'ProjectionSourceSymbolKey',
        'ProjectionSyntheticEntrypointExecutable',
        'ProjectionSyntheticEntrypointIndex',
        'ProjectionSyntheticEntrypointModule',
        'ProjectionView',
        'PublicCallableProjection',
        'PublicInstanceInterfaceProjection',
        'PublicInstanceProjection',
        'PublicModuleExportProjection',
        'PublicSymbolProjection',
        'PublicTypeProjection',
        'PublicationApiProjectionSeed',
        'PublicationResourceProjectionInput',
      ],
      enum: [
        'ConfigRequirementAccessProjection',
        'ConfigRequirementScopeProjection',
        'PackageAbiTypeDescriptor',
        'ProjectionSourceDeclarationKind',
        'ProjectionSyntheticEntrypointExecutableKind',
        'PublicCallableKindProjection',
        'PublicSymbolKindProjection',
        'PublicTypeKindProjection',
      ],
      const: [],
      fn: ['canonical_package_public_path'],
      mod: [],
      static: [],
      trait: [],
      type: [],
      union: [],
    },
    publicExports: [
      'DuplicateProjectionPackageCallableSignature',
      'ProjectionCallableEffectFacts',
      'ProjectionExecutableKey',
      'ProjectionPackageCallableKey',
      'ProjectionPackageCallableSignatureFacts',
      'canonical_package_public_path',
    ],
    structFields: {
      ProjectionInput: [
        'callable_signatures',
        'file_ir_units',
        'lowering',
        'resources',
        'source',
        'source_metadata',
      ],
    },
    handoffs: [],
  },
];

assertTerminalPublicShapeRegistry();

export async function collectTerminalPublicShapeViolations(files, tools) {
  const violations = [];
  for (const entry of terminalPublicShapeRegistry) {
    const ownedFiles = files.filter(
      (file) => file.relPath === entry.root || file.relPath.startsWith(`${entry.root}/`),
    );
    if (ownedFiles.length === 0) {
      continue;
    }

    const declarations = [];
    const exports = [];
    for (const file of ownedFiles) {
      const text = await tools.readText(file);
      for (const declaration of terminalPublicItemDeclarations(text, tools)) {
        declarations.push({ ...declaration, file, text });
        if (!entry.publicItems[declaration.kind].includes(declaration.name)) {
          violations.push(
            terminalPublicShapeMatch(
              entry,
              file,
              text,
              declaration,
              `undeclared public ${declaration.kind}`,
              tools,
            ),
          );
        }
      }
      for (const exported of terminalPublicExports(text)) {
        exports.push({ ...exported, file, text });
        if (!entry.publicExports.includes(exported.name)) {
          violations.push(
            terminalPublicShapeMatch(
              entry,
              file,
              text,
              exported,
              'undeclared public re-export',
              tools,
            ),
          );
        }
      }
    }

    for (const kind of terminalPublicItemKinds) {
      for (const name of entry.publicItems[kind]) {
        const matchingDeclarations = declarations.filter(
          (candidate) => candidate.kind === kind && candidate.name === name,
        );
        if (matchingDeclarations.length === 0) {
          violations.push(terminalMissingPublicShapeMatch(entry, `${kind} ${name}`));
          continue;
        }
        for (const duplicate of matchingDeclarations.slice(1)) {
          violations.push(
            terminalPublicShapeMatch(
              entry,
              duplicate.file,
              duplicate.text,
              duplicate,
              `duplicate canonical public ${kind} ${name}`,
              tools,
            ),
          );
        }
      }
    }
    for (const name of entry.publicExports) {
      const matchingExports = exports.filter((candidate) => candidate.name === name);
      if (matchingExports.length === 0) {
        violations.push(terminalMissingPublicShapeMatch(entry, `re-export ${name}`));
        continue;
      }
      for (const duplicate of matchingExports.slice(1)) {
        violations.push(
          terminalPublicShapeMatch(
            entry,
            duplicate.file,
            duplicate.text,
            duplicate,
            `duplicate canonical public re-export ${name}`,
            tools,
          ),
        );
      }
    }

    for (const [structName, expectedFields] of Object.entries(entry.structFields)) {
      const declaration = declarations.find(
        (candidate) => candidate.kind === 'struct' && candidate.name === structName,
      );
      if (declaration === undefined) {
        continue;
      }
      const actualFields = terminalNamedStructFields(declaration.text, declaration, tools);
      if (JSON.stringify(actualFields) !== JSON.stringify(expectedFields)) {
        violations.push(
          terminalPublicShapeMatch(
            entry,
            declaration.file,
            declaration.text,
            declaration,
            `frozen ${structName} fields expected=${expectedFields.join(',')} actual=${actualFields.join(',')}`,
            tools,
          ),
        );
      }
    }

    for (const handoff of entry.handoffs) {
      const declaration = declarations.find(
        (candidate) => candidate.kind === 'fn' && candidate.name === handoff.name,
      );
      if (declaration === undefined) {
        continue;
      }
      const expectedSignature = terminalHandoffSignature(handoff);
      const actualSignature = terminalFunctionSignature(declaration.text, declaration.index);
      if (actualSignature !== expectedSignature) {
        violations.push(
          terminalPublicShapeMatch(
            entry,
            declaration.file,
            declaration.text,
            declaration,
            `canonical handoff expected=${expectedSignature} actual=${actualSignature}`,
            tools,
          ),
        );
      }
    }
  }
  return violations;
}

export async function runTerminalPublicShapeSelfTest(tools) {
  const canonicalFiles = terminalPublicShapeFixtureFiles();
  const canonicalFailures = await collectTerminalPublicShapeViolations(canonicalFiles, tools);
  if (canonicalFailures.length !== 0) {
    throw new Error(`canonical terminal public-shape fixture failed: ${JSON.stringify(canonicalFailures)}`);
  }

  const compiledRoot = terminalPublicShapeRegistry.find((entry) => entry.owner === 'compiled').root;
  const projectionInputRoot = terminalPublicShapeRegistry.find(
    (entry) => entry.owner === 'projection-input',
  ).root;
  await expectTerminalPublicShapeSelfTestFailure(
    'renamed aggregate carrying compiled payload plus publication metadata/config',
    [
      ...canonicalFiles,
      terminalFixtureFile(
        compiledRoot,
        'renamed_aggregate.rs',
        `pub struct ReleaseBundle {
    compiled_payload: CompiledPackage,
    manifest_data: ManifestData,
    activation_config: Config,
}
`,
      ),
    ],
    /undeclared public struct/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'renamed adapter accepting an aggregate and returning projection input',
    [
      ...canonicalFiles,
      terminalFixtureFile(
        compiledRoot,
        'renamed_adapter.rs',
        `pub fn map_release_bundle(bundle: &ReleaseBundle) -> ProjectionInput {
    todo!()
}
`,
      ),
    ],
    /undeclared public fn/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'canonical compiled aggregate gains publication config ownership',
    terminalMutateFixture(canonicalFiles, compiledRoot, (text) =>
      text.replace('    model: (),\n}', '    model: (),\n    release_config: (),\n}'),
    ),
    /frozen CompiledPackage fields/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'canonical handoff accepts a publication aggregate instead of CompiledPackage',
    terminalMutateFixture(canonicalFiles, compiledRoot, (text) =>
      text.replace('compiled: &CompiledPackage', 'compiled: &ReleaseBundle'),
    ),
    /canonical handoff/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'canonical handoff regresses to the old infallible signature',
    terminalMutateFixture(canonicalFiles, compiledRoot, (text) =>
      text.replace(
        '-> Result<ProjectionInput, ProjectionInputBuildError>',
        '-> ProjectionInput',
      ),
    ),
    /canonical handoff/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'compiled error declaration is removed',
    terminalMutateFixture(canonicalFiles, compiledRoot, (text) =>
      text.replace('pub enum ProjectionInputBuildError { Fixture }\n', ''),
    ),
    /missing canonical public enum ProjectionInputBuildError/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'compiled error re-export is removed',
    terminalMutateFixture(canonicalFiles, compiledRoot, (text) =>
      text.replace('pub use fixture_exports::{ProjectionInputBuildError};\n', ''),
    ),
    /missing canonical public re-export ProjectionInputBuildError/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'projection input gains an unregistered field',
    terminalMutateFixture(canonicalFiles, projectionInputRoot, (text) =>
      text.replace(
        '    source_metadata: (),\n}',
        '    source_metadata: (),\n    deployment_hint: (),\n}',
      ),
    ),
    /frozen ProjectionInput fields/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'projection input gains an unregistered callable DTO',
    [
      ...canonicalFiles,
      terminalFixtureFile(
        projectionInputRoot,
        'extra_callable_dto.rs',
        'pub struct ExtraCallableSignatureProjection;\n',
      ),
    ],
    /undeclared public struct/,
    tools,
  );
  await expectTerminalPublicShapeSelfTestFailure(
    'canonical package public-path helper is renamed',
    terminalMutateFixture(canonicalFiles, projectionInputRoot, (text) =>
      text.replaceAll('canonical_package_public_path', 'renamed_package_public_path'),
    ),
    {
      count: 4,
      patterns: [
        /undeclared public fn/,
        /undeclared public re-export/,
        /missing canonical public fn canonical_package_public_path/,
        /missing canonical public re-export canonical_package_public_path/,
      ],
    },
    tools,
  );

  console.log('Compiler boundary terminal public-shape self-test passed (11 cases).');
}

function assertTerminalPublicShapeRegistry() {
  const owners = new Set();
  const roots = new Set();
  for (const entry of terminalPublicShapeRegistry) {
    if (owners.has(entry.owner) || roots.has(entry.root)) {
      throw new Error(`duplicate terminal compiler public-shape owner/root: ${entry.owner} ${entry.root}`);
    }
    owners.add(entry.owner);
    roots.add(entry.root);

    const publicNames = new Set();
    for (const kind of terminalPublicItemKinds) {
      const names = entry.publicItems[kind];
      assertSortedUniqueStrings(names, `${entry.owner} public ${kind}`);
      for (const name of names) {
        if (publicNames.has(name)) {
          throw new Error(`${entry.owner} terminal public item is registered twice: ${name}`);
        }
        publicNames.add(name);
      }
    }
    assertSortedUniqueStrings(entry.publicExports, `${entry.owner} public exports`);
    for (const name of entry.publicExports) {
      if (!publicNames.has(name)) {
        throw new Error(`${entry.owner} public export is not a registered declaration: ${name}`);
      }
    }
    for (const [name, fields] of Object.entries(entry.structFields)) {
      if (!entry.publicItems.struct.includes(name)) {
        throw new Error(`${entry.owner} frozen field shape references unknown struct: ${name}`);
      }
      assertSortedUniqueStrings(fields, `${entry.owner} ${name} fields`);
    }
    const handoffNames = new Set();
    for (const handoff of entry.handoffs) {
      if (!entry.publicItems.fn.includes(handoff.name)) {
        throw new Error(`${entry.owner} handoff references unknown public function: ${handoff.name}`);
      }
      if (handoffNames.has(handoff.name)) {
        throw new Error(`${entry.owner} handoff is registered twice: ${handoff.name}`);
      }
      handoffNames.add(handoff.name);
      for (const key of ['parameterName', 'parameterType', 'returnType']) {
        if (typeof handoff[key] !== 'string' || handoff[key] === '') {
          throw new Error(`${entry.owner} handoff ${handoff.name} is missing ${key}`);
        }
      }
    }
  }
}

function assertSortedUniqueStrings(values, label) {
  if (!Array.isArray(values)) {
    throw new Error(`${label} must be an array`);
  }
  const sorted = [...new Set(values)].sort();
  if (JSON.stringify(values) !== JSON.stringify(sorted)) {
    throw new Error(`${label} must contain sorted unique strings: ${sorted.join(', ')}`);
  }
}

function terminalPublicItemDeclarations(text, tools) {
  const declarations = [];
  const itemRegexp =
    /^[ \t]*pub[ \t]+(const|enum|mod|static|struct|trait|type|union)[ \t]+(?!fn\b)([A-Za-z_][A-Za-z0-9_]*)/gm;
  for (const match of text.matchAll(itemRegexp)) {
    declarations.push({
      kind: match[1],
      name: match[2],
      index: match.index ?? 0,
      matched: match[0].trim(),
    });
  }
  for (const declaration of tools.publicFunctionDeclarations(text)) {
    if (tools.implNameAt(text, declaration.index) === undefined) {
      declarations.push({ ...declaration, kind: 'fn' });
    }
  }
  return declarations.sort((left, right) => left.index - right.index);
}

function terminalPublicExports(text) {
  const exports = [];
  const regexp = /^[ \t]*pub[ \t]+use[ \t]+([^;]+);/gm;
  for (const match of text.matchAll(regexp)) {
    for (const name of terminalPublicExportNames(match[1])) {
      exports.push({
        name,
        index: match.index ?? 0,
        matched: `pub use ${match[1].replace(/\s+/g, ' ').trim()};`,
      });
    }
  }
  return exports;
}

function terminalPublicExportNames(body) {
  const openBrace = body.indexOf('{');
  const closeBrace = body.lastIndexOf('}');
  const entries = openBrace === -1 || closeBrace < openBrace
    ? [body]
    : body.slice(openBrace + 1, closeBrace).split(',');
  return entries
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const alias = /\bas\s+([A-Za-z_][A-Za-z0-9_]*)$/.exec(entry);
      if (alias) {
        return alias[1];
      }
      return /([A-Za-z_][A-Za-z0-9_]*)$/.exec(entry)?.[1];
    })
    .filter((name) => name !== undefined && name !== 'self');
}

function terminalNamedStructFields(text, declaration, tools) {
  const semicolon = text.indexOf(';', declaration.index);
  const openBrace = text.indexOf('{', declaration.index);
  if (openBrace === -1 || (semicolon !== -1 && semicolon < openBrace)) {
    return [];
  }
  const closeBrace = tools.matchingBraceIndex(text, openBrace);
  if (closeBrace === -1) {
    return [];
  }
  const fields = [];
  const fieldRegexp = /^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?([A-Za-z_][A-Za-z0-9_]*)[ \t]*:/gm;
  for (const match of text.slice(openBrace + 1, closeBrace).matchAll(fieldRegexp)) {
    fields.push(match[1]);
  }
  return [...new Set(fields)].sort();
}

function terminalHandoffSignature(handoff) {
  return normalizeTerminalFunctionSignature([
      `pub fn ${handoff.name}`,
      `(${handoff.parameterName}: ${handoff.parameterType})`,
      `-> ${handoff.returnType}`,
    ].join(''));
}

function terminalFunctionSignature(text, declarationIndex) {
  const openBrace = text.indexOf('{', declarationIndex);
  const semicolon = text.indexOf(';', declarationIndex);
  const end = openBrace === -1 || (semicolon !== -1 && semicolon < openBrace)
    ? semicolon
    : openBrace;
  if (end === -1) {
    return '<unclosed-function-signature>';
  }
  return normalizeTerminalFunctionSignature(text.slice(declarationIndex, end));
}

function normalizeTerminalFunctionSignature(signature) {
  return signature.replace(/\s+/g, '').replace(/,\)/g, ')');
}

function terminalPublicShapeMatch(entry, file, text, declaration, pattern, tools) {
  return {
    id: 'terminal_compiler_frozen_public_shape',
    owner: entry.owner,
    phase: '2',
    pattern,
    regexp: /terminal compiler frozen public shape/,
    remove_when: 'compiled and projection-input expose only their frozen canonical public items and handoff',
    severity: 'deny',
    relPath: file.relPath,
    line: tools.lineNumberAt(text, declaration.index),
    matched: declaration.matched,
  };
}

function terminalMissingPublicShapeMatch(entry, missing) {
  return {
    id: 'terminal_compiler_frozen_public_shape',
    owner: entry.owner,
    phase: '2',
    pattern: `missing canonical public ${missing}`,
    regexp: /terminal compiler frozen public shape/,
    remove_when: 'compiled and projection-input expose only their frozen canonical public items and handoff',
    severity: 'deny',
    relPath: entry.root,
    line: 1,
    matched: `<missing ${missing}>`,
  };
}

function terminalPublicShapeFixtureFiles() {
  return terminalPublicShapeRegistry.map((entry) => {
    const declarations = [];
    for (const kind of terminalPublicItemKinds) {
      for (const name of entry.publicItems[kind]) {
        const fields = entry.structFields[name];
        if (kind === 'struct' && fields !== undefined) {
          declarations.push(
            `pub struct ${name} {\n${fields.map((field) => `    ${field}: (),`).join('\n')}\n}\n`,
          );
          continue;
        }
        const handoff = entry.handoffs.find((candidate) => candidate.name === name);
        if (kind === 'fn' && handoff !== undefined) {
          declarations.push(
            `pub fn ${handoff.name}(`
              + `${handoff.parameterName}: ${handoff.parameterType}`
              + `) -> ${handoff.returnType} {\n    todo!()\n}\n`,
          );
          continue;
        }
        declarations.push(terminalFixtureDeclaration(kind, name));
      }
    }
    if (entry.publicExports.length > 0) {
      declarations.push(`pub use fixture_exports::{${entry.publicExports.join(', ')}};\n`);
    }
    return terminalFixtureFile(
      entry.root,
      'terminal_shape_fixture.rs',
      declarations.join('\n'),
    );
  });
}

function terminalFixtureDeclaration(kind, name) {
  switch (kind) {
    case 'const':
      return `pub const ${name}: () = ();\n`;
    case 'enum':
      return `pub enum ${name} { Fixture }\n`;
    case 'fn':
      return `pub fn ${name}() {}\n`;
    case 'mod':
      return `pub mod ${name} {}\n`;
    case 'static':
      return `pub static ${name}: () = ();\n`;
    case 'struct':
      return `pub struct ${name};\n`;
    case 'trait':
      return `pub trait ${name} {}\n`;
    case 'type':
      return `pub type ${name} = ();\n`;
    case 'union':
      return `pub union ${name} { value: () }\n`;
    default:
      throw new Error(`unsupported terminal public fixture item kind: ${kind}`);
  }
}

function terminalFixtureFile(root, name, text) {
  return {
    absPath: `<terminal-public-shape-fixture:${root}/${name}>`,
    relPath: `${root}/${name}`,
    text,
  };
}

function terminalMutateFixture(files, root, mutate) {
  return files.map((file) => file.relPath.startsWith(`${root}/`)
    ? { ...file, text: mutate(file.text) }
    : file);
}

async function expectTerminalPublicShapeSelfTestFailure(name, files, expectedPattern, tools) {
  const failures = await collectTerminalPublicShapeViolations(files, tools);
  const expectation = expectedPattern instanceof RegExp
    ? { count: 1, patterns: [expectedPattern] }
    : expectedPattern;
  if (
    failures.length !== expectation.count
    || !expectation.patterns.every((pattern) =>
      failures.some((failure) => pattern.test(failure.pattern)))
  ) {
    throw new Error(
      `${name}: expected ${expectation.count} failure(s) matching ${expectation.patterns.join(', ')}, got ${JSON.stringify(failures)}`,
    );
  }
}
