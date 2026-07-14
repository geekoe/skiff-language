import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MANAGED_CRATE_HELP_NAMES,
  MANAGED_CRATE_NAMES,
  managedCrateConfig,
  publicApiConfigForCrate,
  uniqueCrates,
} from '../lib/crate-public-api-policy.mjs';

test('managed public API policy derives distinct gate and help orders from one source', () => {
  assert.deepEqual(MANAGED_CRATE_NAMES, [
    'skiff-compiler-publication-abi',
    'skiff-compiler-input-model',
    'skiff-compiler-input',
    'skiff-compiler-projection-input',
    'skiff-compiler-source',
    'skiff-compiler-lowering',
    'skiff-compiler-compiled',
    'skiff-compiler-projection',
  ]);
  assert.deepEqual(MANAGED_CRATE_HELP_NAMES, [
    'skiff-compiler-publication-abi',
    'skiff-compiler-input-model',
    'skiff-compiler-input',
    'skiff-compiler-source',
    'skiff-compiler-lowering',
    'skiff-compiler-compiled',
    'skiff-compiler-projection',
    'skiff-compiler-projection-input',
  ]);
  assert.equal(new Set(MANAGED_CRATE_NAMES).size, 8);
  assert.deepEqual(new Set(MANAGED_CRATE_HELP_NAMES), new Set(MANAGED_CRATE_NAMES));
});

test('policy snapshots and configs cannot mutate the canonical records', () => {
  assert.equal(Object.isFrozen(MANAGED_CRATE_NAMES), true);
  assert.equal(Object.isFrozen(MANAGED_CRATE_HELP_NAMES), true);
  assert.throws(() => MANAGED_CRATE_NAMES.push('mutated'), TypeError);

  const config = managedCrateConfig('skiff-compiler-projection-input');
  assert.equal(Object.isFrozen(config), true);
  assert.equal(Object.isFrozen(config.allowedCrates), true);
  assert.throws(() => config.allowedCrates.push('mutated'), TypeError);
  assert.deepEqual(
    managedCrateConfig('skiff-compiler-projection-input').allowedCrates,
    [
      'skiff-compiler-projection-input',
      'skiff-compiler-core',
      'skiff-artifact-model',
      'std',
      'core',
      'alloc',
      'serde',
      'serde_json',
    ],
  );
});

test('normalized allow-list dedup preserves first spelling and insertion order', () => {
  assert.deepEqual(
    uniqueCrates(['first-crate', 'second', 'first_crate', 'third', 'second']),
    ['first-crate', 'second', 'third'],
  );
  const extras = ['external-name', 'external_name', 'last'];
  const config = publicApiConfigForCrate('unmanaged-crate', extras);
  extras.push('late-mutation');
  assert.deepEqual(config.allowedCrates, [
    'unmanaged-crate',
    'std',
    'core',
    'alloc',
    'external-name',
    'last',
  ]);
  assert.equal(Object.isFrozen(config.allowedCrates), true);
});
