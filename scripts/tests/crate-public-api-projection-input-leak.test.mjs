import assert from 'node:assert/strict';
import test from 'node:test';

import {
  emptyGenerics,
  externalPath,
  fakeRustdoc,
  localPath,
  publicItem,
  resolvedType,
} from '../fixtures/crate-public-api-rustdoc.mjs';
import { checkPublicApi } from '../lib/crate-public-api-graph.mjs';
import { managedCrateConfig } from '../lib/crate-public-api-policy.mjs';

test('compiler public shape rejects the four projection-input leaks', () => {
  const result = checkPublicApi(projectionInputLeakRustdoc(), {
    crateName: 'skiff-compiler',
    allowedCrates: managedCrateConfig('skiff-compiler').allowedCrates,
  });

  assert.equal(result.violations.length, 4);
  assert.ok(
    result.violations.every(
      (violation) => violation.crateName === 'skiff_compiler_projection_input',
    ),
  );
  for (const siteFragment of [
    'ResolvedPackageSchema re-export',
    'ResolvedPackageSchemaError re-export',
    'with_resolved_package_schemas signature input schemas',
    'resolved_package_schemas signature output',
  ]) {
    assert.ok(
      result.violations.some((violation) => violation.site.includes(siteFragment)),
      `missing projection-input leak at ${siteFragment}`,
    );
  }
});

function projectionInputLeakRustdoc() {
  const rustdoc = fakeRustdoc({
    rootItems: ['0:1', '0:2', '0:3'],
    index: {
      '0:1': publicItem('PackageCompileInput', {
        struct: {
          generics: emptyGenerics(),
          kind: { plain: { fields: [] } },
          impls: ['0:10'],
        },
      }),
      '0:2': publicItem(null, {
        use: {
          id: '9:1',
          name: 'ResolvedPackageSchema',
          source: 'skiff_compiler_projection_input::ResolvedPackageSchema',
        },
      }),
      '0:3': publicItem(null, {
        use: {
          id: '9:2',
          name: 'ResolvedPackageSchemaError',
          source: 'skiff_compiler_projection_input::ResolvedPackageSchemaError',
        },
      }),
      '0:10': publicItem(null, {
        impl: {
          for: resolvedType('0:1', 'PackageCompileInput'),
          generics: emptyGenerics(),
          items: ['0:11', '0:12'],
          trait: null,
        },
      }),
      '0:11': publicItem('with_resolved_package_schemas', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [['schemas', resolvedType('9:1', 'ResolvedPackageSchema')]],
            output: resolvedType('0:1', 'PackageCompileInput'),
          },
        },
      }),
      '0:12': publicItem('resolved_package_schemas', {
        function: {
          generics: emptyGenerics(),
          sig: {
            inputs: [],
            output: resolvedType('9:1', 'ResolvedPackageSchema'),
          },
        },
      }),
    },
    paths: {
      '0:1': localPath('PackageCompileInput', 'struct'),
      '0:2': localPath('ResolvedPackageSchema', 'use'),
      '0:3': localPath('ResolvedPackageSchemaError', 'use'),
      '9:1': externalPath(
        9,
        ['skiff_compiler_projection_input', 'ResolvedPackageSchema'],
        'struct',
      ),
      '9:2': externalPath(
        9,
        ['skiff_compiler_projection_input', 'ResolvedPackageSchemaError'],
        'enum',
      ),
    },
  });
  rustdoc.external_crates[9] = { name: 'skiff_compiler_projection_input' };
  return rustdoc;
}
