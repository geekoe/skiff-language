import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  packageServiceEcosystemSmokeExpectedMarker,
  packageServiceEcosystemSmokeFixtureCargoArgs,
  packageServiceEcosystemSmokeFixtureRoot
} from '../lib/package-service-ecosystem-smoke-real.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('real ecosystem smoke uses the checked-in normal-source WebSocket fixture', async () => {
  const fixtureRoot = packageServiceEcosystemSmokeFixtureRoot(checkout);
  assert.equal(
    fixtureRoot,
    join(checkout, 'test-runner', 'fixtures', 'package-service-websocket-smoke')
  );
  const source = await readFile(join(fixtureRoot, 'main.skiff'), 'utf8');
  assert.match(source, /WebSocketIngressEvent<null>/);
  assert.match(source, /sendTextToConnection\(event\.receiveEvent\.connection\.id, marker\(\)\)/);
  assert.match(source, new RegExp(packageServiceEcosystemSmokeExpectedMarker));

  assert.deepEqual(
    packageServiceEcosystemSmokeFixtureCargoArgs({
      checkout,
      fixtureRoot,
      artifactRoot: '/tmp/f23d-artifacts',
      environment: 'f23d-test'
    }),
    [
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
      '/tmp/f23d-artifacts',
      '--platform-source-root',
      checkout,
      '--environment',
      'f23d-test'
    ]
  );
});
