export function fakeAllowedRustdoc() {
  return fakeRustdoc({
    rootItems: ['0:1', '0:10', '0:20'],
    index: {
      '0:1': publicItem('ContractDefinitionInput', {
        struct: {
          generics: emptyGenerics(),
          kind: { plain: { fields: ['0:2', '0:3'] } },
          impls: ['0:30'],
        },
      }),
      '0:2': publicItem('artifact', {
        struct_field: resolvedType('3:1', 'ArtifactPublicationId'),
      }),
      '0:3': publicItem('identity', {
        struct_field: resolvedType('2:1', 'ContractIdentity'),
      }),
      '0:10': publicItem('JsonDoc', {
        type_alias: {
          generics: emptyGenerics(),
          type: resolvedType('4:1', 'Value'),
        },
      }),
      '0:20': publicItem('compile_contract', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['input', resolvedType('0:1', 'ContractDefinitionInput')]],
            output: resolvedType('1:1', 'String'),
          },
        },
      }),
      '0:30': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'ContractDefinitionInput'),
          generics: emptyGenerics(),
          items: ['0:31'],
          trait: null,
        },
      }),
      '0:31': publicItem('contract_identity', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [],
            output: resolvedType('3:1', 'ArtifactPublicationId'),
          },
        },
      }),
    },
    paths: {
      '0:1': localPath('ContractDefinitionInput', 'struct'),
      '0:10': localPath('JsonDoc', 'type_alias'),
      '0:20': localPath('compile_contract', 'function'),
      '1:1': externalPath(1, ['alloc', 'string', 'String'], 'struct'),
      '2:1': externalPath(2, ['skiff_artifact_identity', 'ContractIdentity'], 'struct'),
      '3:1': externalPath(3, ['skiff_artifact_model', 'ArtifactPublicationId'], 'struct'),
      '4:1': externalPath(4, ['serde_json', 'Value'], 'enum'),
    },
  });
}

export function fakeDeniedRustdoc() {
  return fakeRustdoc({
    rootItems: ['0:1', '0:10', '0:20', '0:40', '0:50', '0:70'],
    index: {
      '0:1': publicItem('ContractDefinitionInput', {
        struct: {
          generics: emptyGenerics(),
          kind: { plain: { fields: ['0:2'] } },
          impls: ['0:30', '0:60'],
        },
      }),
      '0:2': publicItem('compiled', {
        struct_field: resolvedType('5:1', 'CompiledPublication'),
      }),
      '0:10': publicItem('source_model', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['dep', resolvedType('6:1', 'SourceCompileModel')]],
            output: null,
          },
        },
      }),
      '0:20': publicItem(null, {
        use: {
          id: '8:1',
          name: 'AstNode',
          source: 'skiff_syntax::ast::AstNode',
        },
      }),
      '0:30': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'ContractDefinitionInput'),
          generics: emptyGenerics(),
          items: ['0:31', '0:32', '0:33'],
          trait: null,
        },
      }),
      '0:31': publicItem('from_lowering', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['lowered', resolvedType('7:1', 'LoweringPrivateModel')]],
            output: resolvedType('0:1', 'ContractDefinitionInput'),
          },
        },
      }),
      '0:32': privateItem('private_helper', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['compiled', resolvedType('5:1', 'CompiledPublication')]],
            output: null,
          },
        },
      }),
      '0:33': publicItem('with_canonical_artifact_store', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['store', resolvedType('9:1', 'CanonicalArtifactStore')]],
            output: resolvedType('0:1', 'ContractDefinitionInput'),
          },
        },
      }),
      '0:40': publicItem('BadAlias', {
        type_alias: {
          generics: emptyGenerics(),
          type: resolvedType('8:2', 'ParserState'),
        },
      }),
      '0:50': publicItem('ProjectionEnum', {
        enum: {
          generics: emptyGenerics(),
          variants: ['0:51'],
          impls: [],
        },
      }),
      '0:51': publicItem('SourceBacked', {
        variant: {
          kind: { tuple: ['0:52'] },
        },
      }),
      '0:52': privateItem('0', {
        struct_field: resolvedType('6:1', 'SourceCompileModel'),
      }),
      '0:60': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'ContractDefinitionInput'),
          generics: emptyGenerics(),
          items: ['0:61'],
          trait: {
            args: null,
            id: '2:1',
            name: 'ProjectionTrait',
          },
        },
      }),
      '0:61': privateItem('trait_impl_method', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['compiled', resolvedType('5:1', 'CompiledPublication')]],
            output: null,
          },
        },
      }),
      '0:70': publicItem('publish_package_artifact_records', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['store', resolvedType('9:1', 'CanonicalArtifactStore')]],
            output: null,
          },
        },
      }),
    },
    paths: {
      '0:1': localPath('ContractDefinitionInput', 'struct'),
      '0:10': localPath('source_model', 'function'),
      '0:20': localPath('AstNode', 'use'),
      '0:40': localPath('BadAlias', 'type_alias'),
      '0:50': localPath('ProjectionEnum', 'enum'),
      '5:1': externalPath(5, ['skiff_compiler_compiled', 'CompiledPublication'], 'struct'),
      '6:1': externalPath(6, ['skiff_compiler_source', 'SourceCompileModel'], 'struct'),
      '7:1': externalPath(7, ['skiff_compiler_lowering', 'LoweringPrivateModel'], 'struct'),
      '8:1': externalPath(8, ['skiff_syntax', 'ast', 'AstNode'], 'struct'),
      '8:2': externalPath(8, ['skiff_syntax', 'parser', 'ParserState'], 'struct'),
      '9:1': externalPath(
        9,
        ['skiff_deployment', 'storage', 'CanonicalArtifactStore'],
        'struct',
      ),
    },
  });
}

export function fakeRustdoc({ rootItems, index, paths }) {
  return {
    crate_version: '0.0.0',
    external_crates: {
      1: { name: 'alloc' },
      2: { name: 'skiff_artifact_identity' },
      3: { name: 'skiff_artifact_model' },
      4: { name: 'serde_json' },
      5: { name: 'skiff_compiler_compiled' },
      6: { name: 'skiff_compiler_source' },
      7: { name: 'skiff_compiler_lowering' },
      8: { name: 'skiff_syntax' },
      9: { name: 'skiff_deployment' },
    },
    format_version: 0,
    index: {
      '0:0': publicItem('skiff_compiler_contract', {
        module: {
          is_crate: true,
          items: rootItems,
        },
      }),
      ...index,
    },
    paths: {
      '0:0': {
        crate_id: 0,
        kind: 'module',
        path: ['skiff_compiler_contract'],
      },
      ...paths,
    },
    root: '0:0',
  };
}

export function publicItem(name, inner) {
  return {
    attrs: [],
    docs: null,
    id: undefined,
    inner,
    links: {},
    name,
    visibility: 'public',
  };
}

export function privateItem(name, inner) {
  return {
    ...publicItem(name, inner),
    visibility: 'default',
  };
}

export function emptyGenerics() {
  return {
    params: [],
    where_predicates: [],
  };
}

export function resolvedType(id, name) {
  return {
    resolved_path: {
      args: null,
      id,
      name,
    },
  };
}

export function localPath(name, kind) {
  return {
    crate_id: 0,
    kind,
    path: ['skiff_compiler_contract', name],
  };
}

export function externalPath(crateId, path, kind) {
  return {
    crate_id: crateId,
    kind,
    path,
  };
}
