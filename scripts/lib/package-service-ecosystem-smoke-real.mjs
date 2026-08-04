import { join } from 'node:path';

export function packageServiceEcosystemSmokeFixtureCargoArgs({
  checkout,
  fixtureRoot,
  artifactRoot,
  profile,
}) {
  return [
    'run',
    '--quiet',
    '--locked',
    '--manifest-path',
    join(checkout, 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-package-service-smoke-fixture',
    '--',
    fixtureRoot,
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    checkout,
    '--profile',
    profile,
  ];
}
