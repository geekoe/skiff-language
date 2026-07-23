import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { captureCheckedCommand } from '../lib/command-execution.mjs';
import {
  packageServiceEcosystemSmokeFixtureCargoArgs,
} from '../lib/package-service-ecosystem-smoke-real.mjs';
import {
  packageServiceGenerationFixtureRoot,
} from '../lib/package-service-generation-lifecycle-smoke-real.mjs';
import {
  readPackageServiceGenerationFixtureReceipt,
  validatePackageServiceGenerationFixturePair,
} from '../lib/package-service-generation-lifecycle-smoke-oracle.mjs';

const checkout = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('A and B author through the Rust fixture binary into one immutable store', async () => {
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-r05-generation-fixture-'));
  const artifactRoot = join(tempRoot, 'artifacts');
  const environment = 'r05-generation-fixture-combined';
  await mkdir(artifactRoot);
  try {
    const receipts = {};
    for (const candidate of ['A', 'B']) {
      const outcome = await captureCheckedCommand(
        'cargo',
        packageServiceEcosystemSmokeFixtureCargoArgs({
          checkout,
          fixtureRoot: packageServiceGenerationFixtureRoot(checkout, candidate),
          artifactRoot,
          environment,
        }),
        { cwd: checkout },
      );
      receipts[candidate] = readPackageServiceGenerationFixtureReceipt(
        outcome.stdout,
        environment,
      );
    }

    const pair = validatePackageServiceGenerationFixturePair(
      receipts.A,
      receipts.B,
    );
    assert.notEqual(pair.A.packageRecordPath, pair.B.packageRecordPath);
    const [recordA, recordB] = await Promise.all([
      readFile(join(artifactRoot, pair.A.packageRecordPath), 'utf8'),
      readFile(join(artifactRoot, pair.B.packageRecordPath), 'utf8'),
    ]);
    assert.equal(JSON.parse(recordA).packageBuildId, pair.A.packageBuildId);
    assert.equal(JSON.parse(recordB).packageBuildId, pair.B.packageBuildId);
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }
});
