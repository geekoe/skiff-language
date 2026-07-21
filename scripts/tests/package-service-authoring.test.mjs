import assert from 'node:assert/strict';
import { mkdtemp, readFile, rename, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import {
  parseObjectArgs,
  requestAssemblyActivation,
  runCompilerAuthoring,
} from '../lib/package-service-authoring.mjs';
import {
  contractCoordinate,
  readReceiptRecord,
  writeContractRoot,
  writePackageRoot,
} from './package-service-fixtures.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

test('contract-first publish compiles a consumer with no provider package', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-contract-first-'));
  const artifactRoot = join(temp, 'artifacts');
  const contractRoot = join(temp, 'contract');
  const consumerRoot = join(temp, 'consumer');
  await writeContractRoot(contractRoot);
  await writePackageRoot(consumerRoot, {
    packageId: 'example.com/consumer',
    contracts: [contractCoordinate()],
    api: 'run: main.run\n',
    source: 'function run() -> string { return health/health() }\n',
  });

  const contract = await runCompilerAuthoring({
    skiffRoot,
    kind: 'contract',
    action: 'publish',
    root: contractRoot,
    artifactRoot,
  });
  assert.ok(contract.serviceContractReceipt);
  assert.ok(contract.serviceContractPointerReceipt);
  assert.equal('artifactReceipt' in contract, false);
  assert.equal('pointerReceipt' in contract, false);

  const packageResult = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: consumerRoot,
    artifactRoot,
  });
  const artifact = await readReceiptRecord(artifactRoot, packageResult.packageArtifactReceipt);
  assert.equal(artifact.packageId, 'example.com/consumer');
  assert.equal(artifact.contractRequirements.length, 1);
  assert.equal(artifact.contractRequirements[0].alias, 'health');
  assert.equal(artifact.serviceRequirements.length, 1);
  assert.equal(JSON.stringify(artifact).includes('providerPackageId'), false);
  assert.equal(JSON.stringify(artifact).includes('deploymentRevision'), false);
});

test('missing and tampered published contracts fail at the compiler input boundary', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-contract-negative-'));
  const contractRoot = join(temp, 'contract');
  const packageRoot = join(temp, 'package');
  const artifactRoot = join(temp, 'artifacts');
  await writeContractRoot(contractRoot);
  await writePackageRoot(packageRoot, {
    packageId: 'example.com/consumer',
    contracts: [contractCoordinate()],
    api: 'run: main.run\n',
    source: 'function run() -> string { return health/health() }\n',
  });

  await assert.rejects(
    runCompilerAuthoring({ skiffRoot, kind: 'package', action: 'build', root: packageRoot, artifactRoot }),
    /no published ServiceContract pointer/,
  );

  const published = await runCompilerAuthoring({
    skiffRoot,
    kind: 'contract',
    action: 'publish',
    root: contractRoot,
    artifactRoot,
  });
  const recordPath = join(artifactRoot, published.serviceContractReceipt.recordPath);
  const record = JSON.parse(await readFile(recordPath, 'utf8'));
  record.diagnosticText.service = 'tampered';
  await writeFile(recordPath, `${JSON.stringify(record)}\n`);
  await assert.rejects(
    runCompilerAuthoring({ skiffRoot, kind: 'package', action: 'build', root: packageRoot, artifactRoot }),
    /canonical|identity|protocol|contract dependency/i,
  );

  await rename(recordPath, `${recordPath}.hidden`);
  await assert.rejects(
    runCompilerAuthoring({ skiffRoot, kind: 'package', action: 'build', root: packageRoot, artifactRoot }),
    /read|No such file|not found/i,
  );
});

test('duplicate dependency aliases and retired options are rejected without compatibility paths', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-authoring-alias-negative-'));
  const packageRoot = join(temp, 'package');
  await writePackageRoot(packageRoot, {
    packageId: 'example.com/duplicate-alias',
    contracts: [contractCoordinate('same'), {
      alias: 'same',
      serviceId: 'example.com/other',
      contractVersion: '1.0.0',
    }],
  });
  await assert.rejects(
    runCompilerAuthoring({
      skiffRoot,
      kind: 'package',
      action: 'build',
      root: packageRoot,
      artifactRoot: join(temp, 'artifacts'),
    }),
    /duplicate alias same/,
  );
  assert.throws(
    () => parseObjectArgs('package', 'build', [packageRoot, '--artifact-root', join(temp, 'artifacts'), '--service-artifact-root', temp]),
    /unknown option --service-artifact-root/,
  );
});

test('activation request construction rejects values outside the frozen T01 wire boundary', async () => {
  const base = {
    activationId: 'activation-1',
    expectedGeneration: 0,
    environment: 'dev',
    assembly: {
      assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${'1'.repeat(64)}`,
    },
  };
  let requests = 0;
  const fetchImpl = async () => {
    requests += 1;
    return new Response('{}');
  };
  for (const override of [
    { expectedGeneration: -0 },
    { expectedGeneration: Number.MAX_SAFE_INTEGER },
    { activationId: 'not visible ascii space' },
    { environment: 'x'.repeat(201) },
    { assembly: { ...base.assembly, buildId: 'legacy' } },
  ]) {
    await assert.rejects(
      requestAssemblyActivation({ ...base, ...override, fetchImpl }),
      /activation|RuntimeAssembly/,
    );
  }
  assert.equal(requests, 0);
});
