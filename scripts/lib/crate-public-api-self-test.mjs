import {
  fakeAllowedRustdoc,
  fakeDeniedRustdoc,
} from '../fixtures/crate-public-api-rustdoc.mjs';
import { resolveConfiguredPackages } from './crate-public-api-gate.mjs';
import { checkPublicApi } from './crate-public-api-graph.mjs';
import {
  MANAGED_CRATE_NAMES,
  managedCrateConfig,
} from './crate-public-api-policy.mjs';

export function runCratePublicApiSelfTest({ stdout }) {
  const configuredPackages = MANAGED_CRATE_NAMES.map((name) => ({ name }));
  assertEqual(
    resolveConfiguredPackages({ packages: configuredPackages }, MANAGED_CRATE_NAMES).length,
    MANAGED_CRATE_NAMES.length,
    'all-configured fixture should include every managed crate',
  );
  let missingConfiguredError;
  try {
    resolveConfiguredPackages(
      { packages: configuredPackages.slice(1) },
      MANAGED_CRATE_NAMES,
    );
  } catch (error) {
    missingConfiguredError = error;
  }
  assert(
    missingConfiguredError?.message.includes('configured public API crate(s) missing from workspace'),
    'all-configured must fail closed when a managed crate is absent',
  );

  const crateName = 'skiff-compiler-contract';
  const config = {
    crateName,
    allowedCrates: managedCrateConfig(crateName).allowedCrates,
  };

  const allowedResult = checkPublicApi(fakeAllowedRustdoc(), config);
  assertEqual(allowedResult.violations.length, 0, 'allowed fake rustdoc should pass');

  const deniedResult = checkPublicApi(fakeDeniedRustdoc(), config);
  const deniedCrates = new Set(deniedResult.violations.map((violation) => violation.crateName));
  for (const deniedCrate of [
    'skiff_compiler_compiled',
    'skiff_compiler_source',
    'skiff_compiler_lowering',
    'skiff_syntax',
  ]) {
    assert(
      deniedCrates.has(deniedCrate),
      `denied fake rustdoc should report forbidden crate ${deniedCrate}`,
    );
  }

  assert(
    deniedResult.violations.some((violation) => violation.site.includes('re-export')),
    'denied fake rustdoc should cover re-export checks',
  );
  assert(
    deniedResult.violations.some(
      (violation) =>
        violation.site.includes('ProjectionEnum variant::SourceBacked')
        && violation.referencedPath.endsWith('SourceCompileModel'),
    ),
    'denied fake rustdoc should cover enum variant field checks',
  );
  assert(
    deniedResult.violations.some((violation) => violation.site.includes('signature input dep')),
    'denied fake rustdoc should cover public function signature checks',
  );
  assert(
    deniedResult.violations.some((violation) => violation.site.includes('impl')),
    'denied fake rustdoc should cover exposed impl method checks',
  );
  assert(
    deniedResult.violations.some((violation) => violation.site.includes('trait_impl_method')),
    'denied fake rustdoc should cover exposed trait impl method checks',
  );
  assert(
    !deniedResult.violations.some((violation) => violation.site.includes('private_helper')),
    'private impl methods should not be checked as public API',
  );

  stdout.write(
    `Self-test passed: allowed fixture 0 violation(s), denied fixture ${deniedResult.violations.length} violation(s).\n`,
  );
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`self-test failed: ${message}`);
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`self-test failed: ${message}; expected ${expected}, got ${actual}`);
  }
}
