import assert from 'node:assert/strict';
import test from 'node:test';

import {
  MANAGED_CRATE_HELP_NAMES,
  MANAGED_CRATE_NAMES,
  managedCrateConfig,
  publicApiConfigForCrate,
  uniqueCrates,
} from '../lib/crate-public-api-policy.mjs';

test('managed public API policy declares only the two terminal producer owners', () => {
  assert.deepEqual(MANAGED_CRATE_NAMES, [
    'skiff-compiler-contract',
    'skiff-compiler',
  ]);
  assert.deepEqual(MANAGED_CRATE_HELP_NAMES, [
    'skiff-compiler-contract',
    'skiff-compiler',
  ]);
  assert.equal(new Set(MANAGED_CRATE_NAMES).size, 2);
  assert.deepEqual(new Set(MANAGED_CRATE_HELP_NAMES), new Set(MANAGED_CRATE_NAMES));
});

test('policy snapshots and configs cannot mutate the canonical records', () => {
  assert.equal(Object.isFrozen(MANAGED_CRATE_NAMES), true);
  assert.equal(Object.isFrozen(MANAGED_CRATE_HELP_NAMES), true);
  assert.throws(() => MANAGED_CRATE_NAMES.push('mutated'), TypeError);

  const config = managedCrateConfig('skiff-compiler-contract');
  assert.equal(Object.isFrozen(config), true);
  assert.equal(Object.isFrozen(config.allowedCrates), true);
  assert.throws(() => config.allowedCrates.push('mutated'), TypeError);
  assert.deepEqual(
    managedCrateConfig('skiff-compiler-contract').allowedCrates,
    [
      'skiff-compiler-contract',
      'skiff-artifact-model',
      'skiff-artifact-identity',
      'std',
      'core',
      'alloc',
      'serde',
      'serde_json',
    ],
  );

  const facadeAllowed = managedCrateConfig('skiff-compiler').allowedCrates;
  assert.deepEqual(facadeAllowed, [
    'skiff-compiler',
    'skiff-compiler-contract',
    'skiff-compiler-input-model',
    'skiff-compiler-input',
    'skiff-compiler-source',
    'skiff-compiler-emission',
    'skiff-artifact-model',
    'skiff-syntax',
    'std',
    'core',
    'alloc',
    'serde',
    'serde_json',
  ]);
  for (const deletedOwner of [
    'skiff-compiler-publication-abi',
    'skiff-compiler-compiled',
    'skiff-compiler-lowering',
    'skiff-compiler-projection-input',
    'skiff-compiler-projection',
  ]) {
    assert.equal(facadeAllowed.includes(deletedOwner), false);
  }
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
