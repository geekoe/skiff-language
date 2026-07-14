const ROOT_CRATE = 'matrix_crate';
const ALLOWED_ID = '1:1';
const DENIED_ID = '2:1';
const LOCAL_ID = '0:900';

function publicItem(name, inner, visibility = 'public') {
  return { attrs: [], docs: null, inner, links: {}, name, visibility };
}

function resolved(id, name, args = null) {
  return { resolved_path: { args, id, name } };
}

function pairType() {
  return {
    tuple: [resolved(ALLOWED_ID, 'Allowed'), resolved(DENIED_ID, 'Denied')],
  };
}

function emptyGenerics(extra = {}) {
  return { params: [], where_predicates: [], ...extra };
}

function descriptor(id, name, inner, options = {}) {
  return {
    id,
    item: publicItem(name, inner, options.visibility),
    path: options.path ?? [ROOT_CRATE, name ?? id],
  };
}

function rustdoc(descriptors, options = {}) {
  const index = {
    '0:0': publicItem(ROOT_CRATE, {
      module: { is_crate: true, items: options.rootItems ?? descriptors.map(({ id }) => id) },
    }),
  };
  const paths = {
    '0:0': { crate_id: 0, kind: 'module', path: [ROOT_CRATE] },
    [ALLOWED_ID]: { crate_id: 1, kind: 'struct', path: ['allowed_dep', 'Allowed'] },
    [DENIED_ID]: { crate_id: 2, kind: 'struct', path: ['forbidden_dep', 'Denied'] },
  };
  for (const entry of descriptors) {
    index[entry.id] = entry.item;
    if (entry.path !== false) {
      paths[entry.id] = { crate_id: 0, kind: 'item', path: entry.path };
    }
  }
  return {
    external_crates: {
      1: { name: 'allowed_dep' },
      2: { name: 'forbidden_dep' },
      ...options.externalCrates,
    },
    index: { ...index, ...options.index },
    paths: { ...paths, ...options.paths },
    root: '0:0',
  };
}

function violation(site, referencedPath = 'forbidden_dep::Denied', crateName = 'forbidden_dep') {
  return { crateName, referencedPath, site };
}

function graphCase(id, rustdocJson, expectedViolations, referenceExpectation = 'allowed-and-denied') {
  return Object.freeze({
    expectedViolations: Object.freeze(expectedViolations),
    id,
    referenceExpectation,
    rustdoc: rustdocJson,
  });
}

function aliasCase(id, type, expectedViolations, referenceExpectation) {
  return graphCase(
    id,
    rustdoc([
      descriptor('0:1', 'Case', {
        type_alias: { generics: emptyGenerics(), type },
      }),
    ]),
    expectedViolations,
    referenceExpectation,
  );
}

const innerVariantCases = [
  graphCase(
    'inner-module',
    rustdoc([
      descriptor('0:1', 'Nested', { module: { items: ['0:2'] } }),
      descriptor(
        '0:2',
        'Alias',
        { type_alias: { generics: emptyGenerics(), type: pairType() } },
        { path: [ROOT_CRATE, 'Nested', 'Alias'] },
      ),
    ], { rootItems: ['0:1'] }),
    [violation('matrix_crate::Nested::Alias target')],
  ),
  graphCase(
    'inner-use-id',
    rustdoc([
      descriptor('0:1', 'AllowedUse', { use: { id: ALLOWED_ID, name: 'Allowed' } }),
      descriptor('0:2', 'DeniedUse', { use: { id: DENIED_ID, name: 'Denied' } }),
    ]),
    [violation('matrix_crate::DeniedUse re-export')],
  ),
  graphCase(
    'inner-use-target',
    rustdoc([
      descriptor('0:1', 'AllowedUse', { use: { name: 'Allowed', target: ALLOWED_ID } }),
      descriptor('0:2', 'DeniedUse', { use: { name: 'Denied', target: DENIED_ID } }),
    ]),
    [violation('matrix_crate::DeniedUse re-export')],
  ),
  ...['struct', 'union'].map((kind) => graphCase(
    `inner-${kind}`,
    rustdoc([
      descriptor('0:1', 'Record', {
        [kind]: {
          generics: emptyGenerics(),
          impls: [],
          kind: { plain: { fields: ['0:2', '0:3'] } },
        },
      }),
      descriptor('0:2', 'allowed', { struct_field: resolved(ALLOWED_ID, 'Allowed') }),
      descriptor('0:3', 'denied', { struct_field: resolved(DENIED_ID, 'Denied') }),
    ], { rootItems: ['0:1'] }),
    [violation('matrix_crate::denied field type')],
  )),
  graphCase(
    'inner-struct-field',
    rustdoc([
      descriptor('0:1', 'field', { struct_field: pairType() }),
    ]),
    [violation('matrix_crate::field field type')],
  ),
  graphCase(
    'inner-enum',
    rustdoc([
      descriptor('0:1', 'Choice', {
        enum: { generics: emptyGenerics(), impls: [], variants: ['0:2', '0:3'] },
      }),
      descriptor('0:2', 'Allowed', { variant: { kind: { tuple: ['0:4'] } } }),
      descriptor('0:3', 'Denied', { variant: { kind: { tuple: ['0:5'] } } }),
      descriptor('0:4', '0', { struct_field: resolved(ALLOWED_ID, 'Allowed') }, { visibility: 'default' }),
      descriptor('0:5', '0', { struct_field: resolved(DENIED_ID, 'Denied') }, { visibility: 'default' }),
    ], { rootItems: ['0:1'] }),
    [violation('matrix_crate::0 field type')],
  ),
  graphCase(
    'inner-variant',
    rustdoc([
      descriptor('0:1', 'Variant', { variant: { kind: { tuple: ['0:2', '0:3'] } } }),
      descriptor('0:2', 'allowed', { struct_field: resolved(ALLOWED_ID, 'Allowed') }, { visibility: 'default' }),
      descriptor('0:3', 'denied', { struct_field: resolved(DENIED_ID, 'Denied') }, { visibility: 'default' }),
    ], { rootItems: ['0:1'] }),
    [violation('matrix_crate::denied field type')],
  ),
  graphCase(
    'inner-function',
    rustdoc([
      descriptor('0:1', 'call', {
        function: {
          generics: emptyGenerics(),
          sig: { inputs: [['allowed', resolved(ALLOWED_ID, 'Allowed')]], output: resolved(DENIED_ID, 'Denied') },
        },
      }),
    ]),
    [violation('matrix_crate::call signature output')],
  ),
  aliasCase('inner-type-alias', pairType(), [violation('matrix_crate::Case target')]),
  graphCase(
    'inner-impl',
    rustdoc([
      descriptor('0:1', 'Impl', {
        impl: { for: pairType(), generics: emptyGenerics(), items: [], trait: null },
      }),
    ]),
    [violation('matrix_crate::Impl for type')],
  ),
  graphCase(
    'inner-trait',
    rustdoc([
      descriptor('0:1', 'Trait', {
        trait: { bounds: [resolved(ALLOWED_ID, 'Allowed'), resolved(DENIED_ID, 'Denied')], generics: emptyGenerics(), items: [] },
      }),
    ]),
    [violation('matrix_crate::Trait bounds')],
  ),
  graphCase(
    'inner-trait-alias',
    rustdoc([
      descriptor('0:1', 'TraitAlias', {
        trait_alias: { generics: emptyGenerics(), params: [resolved(ALLOWED_ID, 'Allowed'), resolved(DENIED_ID, 'Denied')] },
      }),
    ]),
    [violation('matrix_crate::TraitAlias bounds')],
  ),
  graphCase(
    'inner-assoc-type',
    rustdoc([
      descriptor('0:1', 'Assoc', {
        assoc_type: { bounds: [], generics: emptyGenerics(), type: pairType() },
      }),
    ]),
    [violation('matrix_crate::Assoc default')],
  ),
  ...['assoc_const', 'constant', 'static'].map((kind) => graphCase(
    `inner-${kind.replace('_', '-')}`,
    rustdoc([descriptor('0:1', 'VALUE', { [kind]: { type: pairType() } })]),
    [violation('matrix_crate::VALUE type')],
  )),
  graphCase(
    'inner-extern-crate',
    rustdoc([
      descriptor('0:1', 'allowed_dep', { extern_crate: { id: ALLOWED_ID } }),
      descriptor('0:2', 'forbidden_dep', { extern_crate: { id: DENIED_ID } }),
    ]),
    [violation('matrix_crate::forbidden_dep')],
  ),
];

const typeVariantCases = [
  aliasCase('type-resolved-path', pairType(), [violation('matrix_crate::Case target')]),
  aliasCase(
    'type-qualified-path',
    {
      qualified_path: {
        args: null,
        self_type: resolved(ALLOWED_ID, 'Allowed'),
        trait: { args: null, id: DENIED_ID, name: 'Denied' },
      },
    },
    [violation('matrix_crate::Case target trait')],
  ),
  ...['borrowed_ref', 'raw_pointer', 'slice', 'pat'].map((kind) => aliasCase(
    `type-${kind.replace('_', '-')}`,
    { [kind]: { type: pairType() } },
    [violation('matrix_crate::Case target')],
  )),
  aliasCase(
    'type-array-with-length-signature',
    {
      array: {
        length: { type: resolved(DENIED_ID, 'Denied') },
        type: resolved(ALLOWED_ID, 'Allowed'),
      },
    },
    [violation('matrix_crate::Case target type')],
  ),
  aliasCase('type-tuple', pairType(), [violation('matrix_crate::Case target')]),
  aliasCase(
    'type-function-pointer',
    {
      function_pointer: {
        sig: {
          inputs: [['allowed', resolved(ALLOWED_ID, 'Allowed')]],
          output: resolved(DENIED_ID, 'Denied'),
        },
      },
    },
    [violation('matrix_crate::Case target function pointer output')],
  ),
  ...['dyn_trait', 'impl_trait'].map((kind) => aliasCase(
    `type-${kind.replace('_', '-')}`,
    { [kind]: [resolved(ALLOWED_ID, 'Allowed'), resolved(DENIED_ID, 'Denied')] },
    [violation('matrix_crate::Case target')],
  )),
  ...[
    ['generic', { generic: 'T' }],
    ['primitive', { primitive: 'u64' }],
    ['infer', { infer: null }],
    ['never', { never: null }],
  ].map(([id, type]) => aliasCase(`type-${id}`, type, [], 'reference-free-terminal')),
];

function structKindCase(owner, kindName, kind, descriptors = [], expected = [], expectation) {
  const inner = owner === 'struct'
    ? { struct: { generics: emptyGenerics(), impls: [], kind } }
    : { variant: { kind } };
  return graphCase(
    `${owner}-kind-${kindName}`,
    rustdoc([descriptor('0:1', owner === 'struct' ? 'Record' : 'Variant', inner), ...descriptors], {
      rootItems: ['0:1'],
    }),
    expected,
    expectation,
  );
}

const shapeCases = [
  structKindCase(
    'struct',
    'plain',
    { plain: { fields: ['0:2', '0:3'] } },
    [
      descriptor('0:2', 'allowed', { struct_field: resolved(ALLOWED_ID, 'Allowed') }),
      descriptor('0:3', 'denied', { struct_field: resolved(DENIED_ID, 'Denied') }),
    ],
    [violation('matrix_crate::denied field type')],
  ),
  structKindCase(
    'struct',
    'tuple',
    { tuple: ['0:2', '0:3'] },
    [
      descriptor('0:2', 'allowed', { struct_field: resolved(ALLOWED_ID, 'Allowed') }),
      descriptor('0:3', 'denied', { struct_field: resolved(DENIED_ID, 'Denied') }),
    ],
    [violation('matrix_crate::denied field type')],
  ),
  structKindCase('struct', 'unit', { unit: null }, [], [], 'reference-free-terminal'),
  structKindCase(
    'struct',
    'unknown',
    { mystery: { type: pairType() } },
    [],
    [violation('matrix_crate::Record fields type')],
  ),
  structKindCase(
    'variant',
    'plain',
    { plain: { fields: ['0:2', '0:3'] } },
    [
      descriptor('0:2', 'allowed', { struct_field: resolved(ALLOWED_ID, 'Allowed') }, { visibility: 'default' }),
      descriptor('0:3', 'denied', { struct_field: resolved(DENIED_ID, 'Denied') }, { visibility: 'default' }),
    ],
    [violation('matrix_crate::denied field type')],
  ),
  structKindCase(
    'variant',
    'tuple',
    { tuple: ['0:2', '0:3'] },
    [
      descriptor('0:2', 'allowed', { struct_field: resolved(ALLOWED_ID, 'Allowed') }, { visibility: 'default' }),
      descriptor('0:3', 'denied', { struct_field: resolved(DENIED_ID, 'Denied') }, { visibility: 'default' }),
    ],
    [violation('matrix_crate::denied field type')],
  ),
  structKindCase('variant', 'unit', { unit: null }, [], [], 'reference-free-terminal'),
  structKindCase(
    'variant',
    'unknown',
    { mystery: { type: pairType() } },
    [],
    [
      violation('matrix_crate::Variant fields type'),
      violation('matrix_crate::Variant type'),
    ],
  ),
];

function localGenericPath(args) {
  return { resolved_path: { args, id: LOCAL_ID, name: 'Wrapper' } };
}

function localPathOptions() {
  return {
    index: { [LOCAL_ID]: publicItem('Wrapper', { struct: { generics: emptyGenerics(), impls: [], kind: { unit: null } } }) },
    paths: { [LOCAL_ID]: { crate_id: 0, kind: 'struct', path: [ROOT_CRATE, 'Wrapper'] } },
  };
}

const genericCases = [
  graphCase(
    'generic-args-angle-bracketed',
    rustdoc([
      descriptor('0:1', 'Case', {
        type_alias: {
          generics: emptyGenerics(),
          type: localGenericPath({
            angle_bracketed: {
              args: [{ type: resolved(ALLOWED_ID, 'Allowed') }],
              constraints: [{ binding: { type: resolved(DENIED_ID, 'Denied') } }],
            },
          }),
        },
      }),
    ], localPathOptions()),
    [violation('matrix_crate::Case target args constraint type')],
  ),
  graphCase(
    'generic-args-parenthesized',
    rustdoc([
      descriptor('0:1', 'Case', {
        type_alias: {
          generics: emptyGenerics(),
          type: localGenericPath({
            parenthesized: {
              inputs: [resolved(ALLOWED_ID, 'Allowed')],
              output: resolved(DENIED_ID, 'Denied'),
            },
          }),
        },
      }),
    ], localPathOptions()),
    [violation('matrix_crate::Case target args output')],
  ),
  graphCase(
    'generic-args-unknown-shape',
    rustdoc([
      descriptor('0:1', 'Case', {
        type_alias: {
          generics: emptyGenerics(),
          type: localGenericPath({ mystery: { type: pairType() } }),
        },
      }),
    ], localPathOptions()),
    [violation('matrix_crate::Case target args type')],
  ),
  graphCase(
    'generics-params',
    rustdoc([
      descriptor('0:1', 'call', {
        function: {
          generics: emptyGenerics({ params: [{ kind: { type: pairType() }, name: 'T' }] }),
          sig: { inputs: [], output: null },
        },
      }),
    ]),
    [violation('matrix_crate::call generics parameter T type')],
  ),
  ...[
    ['where-predicates-snake', 'where_predicates'],
    ['where-predicates-camel', 'wherePredicates'],
  ].map(([id, key]) => graphCase(
    id,
    rustdoc([
      descriptor('0:1', 'call', {
        function: {
          generics: key === 'where_predicates'
            ? { params: [], where_predicates: [{ type: pairType() }] }
            : { params: [], wherePredicates: [{ type: pairType() }] },
          sig: { inputs: [], output: null },
        },
      }),
    ]),
    [violation('matrix_crate::call generics where predicate type')],
  )),
  graphCase(
    'trait-bounds',
    rustdoc([descriptor('0:1', 'Trait', {
      trait: { bounds: [resolved(ALLOWED_ID, 'Allowed'), resolved(DENIED_ID, 'Denied')], generics: emptyGenerics(), items: [] },
    })]),
    [violation('matrix_crate::Trait bounds')],
  ),
  graphCase(
    'assoc-type-bounds-and-default',
    rustdoc([descriptor('0:1', 'Assoc', {
      assoc_type: {
        bounds: [resolved(ALLOWED_ID, 'Allowed')],
        generics: emptyGenerics(),
        type: resolved(DENIED_ID, 'Denied'),
      },
    })]),
    [violation('matrix_crate::Assoc default')],
  ),
];

const exposureAndReferenceCases = [
  graphCase(
    'visibility-public',
    rustdoc([descriptor('0:1', 'visible', {
      function: { generics: emptyGenerics(), sig: { inputs: [], output: resolved(DENIED_ID, 'Denied') } },
    })]),
    [violation('matrix_crate::visible signature output')],
    'denied-only',
  ),
  graphCase(
    'visibility-default-hidden',
    rustdoc([descriptor('0:1', 'hidden', {
      function: { generics: emptyGenerics(), sig: { inputs: [], output: resolved(DENIED_ID, 'Denied') } },
    }, { visibility: 'default' })]),
    [],
    'denied-reference-suppressed',
  ),
  graphCase(
    'trait-impl-forces-private-item-exposure',
    rustdoc([
      descriptor('0:1', 'Impl', {
        impl: {
          for: resolved(LOCAL_ID, 'Local'),
          generics: emptyGenerics(),
          items: ['0:2'],
          trait: { args: null, id: ALLOWED_ID, name: 'AllowedTrait' },
        },
      }),
      descriptor('0:2', 'hidden_method', {
        function: {
          generics: emptyGenerics(),
          sig: { inputs: [['dep', resolved(DENIED_ID, 'Denied')]], output: null },
        },
      }, { visibility: 'default' }),
    ], {
      index: { [LOCAL_ID]: publicItem('Local', { struct: { generics: emptyGenerics(), impls: [], kind: { unit: null } } }) },
      paths: { [LOCAL_ID]: { crate_id: 0, kind: 'struct', path: [ROOT_CRATE, 'Local'] } },
      rootItems: ['0:1'],
    }),
    [violation('matrix_crate::hidden_method signature input dep')],
  ),
  graphCase(
    'reference-external-crate-name',
    aliasCase('unused', resolved(DENIED_ID, 'Denied'), []).rustdoc,
    [violation('matrix_crate::Case target')],
    'denied-only',
  ),
  graphCase(
    'reference-path-first-component-fallback',
    rustdoc([descriptor('0:1', 'Case', {
      type_alias: { generics: emptyGenerics(), type: resolved('9:1', 'Denied') },
    })], {
      paths: { '9:1': { crate_id: 9, kind: 'struct', path: ['path_only_dep', 'Denied'] } },
    }),
    [violation('matrix_crate::Case target', 'path_only_dep::Denied', 'path_only_dep')],
    'denied-only',
  ),
  graphCase(
    'reference-local-index-suppresses-external-looking-path',
    rustdoc([descriptor('0:1', 'Case', {
      type_alias: { generics: emptyGenerics(), type: resolved(LOCAL_ID, 'Local') },
    })], {
      index: { [LOCAL_ID]: publicItem('Local', { struct: { generics: emptyGenerics(), impls: [], kind: { unit: null } } }) },
      paths: { [LOCAL_ID]: { crate_id: 0, kind: 'struct', path: ['forbidden_dep', 'LooksExternal'] } },
    }),
    [],
    'local-reference-suppressed',
  ),
  ...[
    ['reference-missing-path-item-name-fallback', '9:2', 'FallbackItem'],
    ['reference-missing-path-id-fallback', '9:3', undefined],
  ].map(([id, referenceId, itemName]) => graphCase(
    id,
    rustdoc([descriptor('0:1', 'Case', {
      type_alias: { generics: emptyGenerics(), type: resolved(referenceId, itemName ?? 'Unknown') },
    })], {
      externalCrates: { 9: { name: 'fallback_dep' } },
      index: itemName ? { [referenceId]: publicItem(itemName, { mystery: null }) } : {},
      paths: { [referenceId]: { crate_id: 9, kind: 'struct', path: [] } },
    }),
    [violation('matrix_crate::Case target', itemName ?? referenceId, 'fallback_dep')],
    'denied-only',
  )),
  graphCase(
    'dedup-seen-item',
    rustdoc([
      descriptor('0:1', 'First', { use: { id: '0:3', name: 'Shared' } }),
      descriptor('0:2', 'Second', { use: { id: '0:3', name: 'Shared' } }),
      descriptor('0:3', 'Shared', {
        type_alias: { generics: emptyGenerics(), type: resolved(DENIED_ID, 'Denied') },
      }),
    ], { rootItems: ['0:1', '0:2'] }),
    [violation('matrix_crate::Shared target')],
    'denied-only-deduped',
  ),
  graphCase(
    'dedup-seen-violation',
    rustdoc([descriptor('0:1', 'call', {
      function: {
        generics: emptyGenerics(),
        sig: {
          inputs: [
            ['dep', resolved(DENIED_ID, 'Denied')],
            ['dep', resolved(DENIED_ID, 'Denied')],
          ],
          output: null,
        },
      },
    })]),
    [violation('matrix_crate::call signature input dep')],
    'denied-only-deduped',
  ),
  graphCase(
    'stable-violation-sort',
    rustdoc([descriptor('0:1', 'call', {
      function: {
        generics: emptyGenerics(),
        sig: {
          inputs: [
            ['zeta', resolved(DENIED_ID, 'Denied')],
            ['alpha', resolved(DENIED_ID, 'Denied')],
          ],
          output: null,
        },
      },
    })]),
    [
      violation('matrix_crate::call signature input alpha'),
      violation('matrix_crate::call signature input zeta'),
    ],
    'denied-only-sorted',
  ),
  graphCase(
    'unknown-inner-currently-ignored',
    rustdoc([descriptor('0:1', 'Unknown', { mystery: { type: pairType() } })]),
    [],
    'current-unknown-inner-no-op',
  ),
  aliasCase(
    'unknown-signature-recursive-fallback',
    { mystery: { nested: { type: pairType() } } },
    [violation('matrix_crate::Case target type')],
  ),
];

export const GRAPH_CASES = Object.freeze([
  ...innerVariantCases,
  ...typeVariantCases,
  ...shapeCases,
  ...genericCases,
  ...exposureAndReferenceCases,
]);

export const GRAPH_MATRIX_EXPECTED_IDS = Object.freeze([
  ...[
    'module', 'use-id', 'use-target', 'struct', 'union', 'struct-field', 'enum',
    'variant', 'function', 'type-alias', 'impl', 'trait', 'trait-alias', 'assoc-type',
    'assoc-const', 'constant', 'static', 'extern-crate',
  ].map((id) => `inner-${id}`),
  ...[
    'resolved-path', 'qualified-path', 'borrowed-ref', 'raw-pointer', 'slice', 'pat',
    'array-with-length-signature', 'tuple', 'function-pointer', 'dyn-trait',
    'impl-trait', 'generic', 'primitive', 'infer', 'never',
  ].map((id) => `type-${id}`),
  ...['struct', 'variant'].flatMap((owner) =>
    ['plain', 'tuple', 'unit', 'unknown'].map((kind) => `${owner}-kind-${kind}`)),
  'generic-args-angle-bracketed',
  'generic-args-parenthesized',
  'generic-args-unknown-shape',
  'generics-params',
  'where-predicates-snake',
  'where-predicates-camel',
  'trait-bounds',
  'assoc-type-bounds-and-default',
  'visibility-public',
  'visibility-default-hidden',
  'trait-impl-forces-private-item-exposure',
  'reference-external-crate-name',
  'reference-path-first-component-fallback',
  'reference-local-index-suppresses-external-looking-path',
  'reference-missing-path-item-name-fallback',
  'reference-missing-path-id-fallback',
  'dedup-seen-item',
  'dedup-seen-violation',
  'stable-violation-sort',
  'unknown-inner-currently-ignored',
  'unknown-signature-recursive-fallback',
]);
