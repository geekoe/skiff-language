import assert from 'node:assert/strict';
import { access, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

import { runCompilerAuthoring } from '../lib/package-service-authoring.mjs';
import {
  contractCoordinate,
  readReceiptRecord,
  writeAssemblyRoot,
  writeContractRoot,
  writeDeploymentRoot,
  writePackageRoot,
} from './package-service-fixtures.mjs';

const skiffRoot = resolve(import.meta.dirname, '..', '..');

test('four typed producers write immutable records and object-specific pointer receipts', async () => {
  const fixture = await canonicalFixture('store-round-trip');
  const { artifactRoot, contractRoot, packageRoot, deploymentRoot, assemblyRoot } = fixture;

  const contractBuild = await author('contract', 'build', contractRoot, artifactRoot);
  assert.ok(contractBuild.serviceContractReceipt);
  assert.equal(contractBuild.serviceContractPointerReceipt, undefined);
  const contractPublish = await author('contract', 'publish', contractRoot, artifactRoot);
  assert.ok(contractPublish.serviceContractPointerReceipt);

  const packageBuild = await author('package', 'build', packageRoot, artifactRoot);
  assert.ok(packageBuild.packageArtifactReceipt);
  assert.equal(packageBuild.packagePointerReceipt, undefined);
  const packagePublish = await author('package', 'publish', packageRoot, artifactRoot);
  assert.ok(packagePublish.packagePointerReceipt);

  const contractRecord = await readReceiptRecord(artifactRoot, contractPublish.serviceContractReceipt);
  const operationId = Object.keys(contractRecord.operations)[0];
  await writeDeploymentRoot(deploymentRoot, {
    contract: contractPublish.serviceContractReceipt.contract,
    implementation: packagePublish.packageArtifactReceipt.artifact,
    operationId,
  });
  const deploymentPublish = await author('deployment', 'publish', deploymentRoot, artifactRoot);
  assert.ok(deploymentPublish.serviceDeploymentReceipt);
  assert.ok(deploymentPublish.serviceDeploymentPointerReceipt);

  await writeAssemblyRoot(
    assemblyRoot,
    'test',
    [deploymentPublish.serviceDeploymentReceipt.deployment],
  );
  const assemblyBuild = await author('assembly', 'build', assemblyRoot, artifactRoot);
  assert.ok(assemblyBuild.runtimeAssemblyReceipt);
  assert.equal(assemblyBuild.runtimeAssemblyPointerReceipt, undefined);
  const assemblyPublish = await author('assembly', 'publish', assemblyRoot, artifactRoot);
  assert.ok(assemblyPublish.runtimeAssemblyPointerReceipt);

  for (const receipt of [
    contractPublish.serviceContractReceipt,
    packagePublish.packageArtifactReceipt,
    deploymentPublish.serviceDeploymentReceipt,
    assemblyPublish.runtimeAssemblyReceipt,
  ]) {
    await access(join(artifactRoot, receipt.recordPath));
  }
  for (const pointer of [
    contractPublish.serviceContractPointerReceipt,
    packagePublish.packagePointerReceipt,
    deploymentPublish.serviceDeploymentPointerReceipt,
    assemblyPublish.runtimeAssemblyPointerReceipt,
  ]) {
    await access(join(artifactRoot, pointer.pointerPath));
  }
});

test('deployment projection rejects an operation mapping that mismatches the published contract', async () => {
  const fixture = await canonicalFixture('deployment-mismatch');
  const contract = await author('contract', 'publish', fixture.contractRoot, fixture.artifactRoot);
  const packageResult = await author('package', 'publish', fixture.packageRoot, fixture.artifactRoot);
  await writeDeploymentRoot(fixture.deploymentRoot, {
    contract: contract.serviceContractReceipt.contract,
    implementation: packageResult.packageArtifactReceipt.artifact,
    operationId: 'skiff-contract-operation-v1:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
  });
  await assert.rejects(
    author('deployment', 'build', fixture.deploymentRoot, fixture.artifactRoot),
    /operation|binding|contract/i,
  );
});

test('assembly follows published provider deployments from an exact root closure', async () => {
  const fixture = await canonicalFixture('root-closure');
  const healthContract = await author(
    'contract',
    'publish',
    fixture.contractRoot,
    fixture.artifactRoot,
  );
  const healthRecord = await readReceiptRecord(
    fixture.artifactRoot,
    healthContract.serviceContractReceipt,
  );
  const healthOperationId = Object.keys(healthRecord.operations)[0];
  const provider = await author('package', 'publish', fixture.packageRoot, fixture.artifactRoot);
  await writeDeploymentRoot(fixture.deploymentRoot, {
    contract: healthContract.serviceContractReceipt.contract,
    implementation: provider.packageArtifactReceipt.artifact,
    operationId: healthOperationId,
  });
  await author('deployment', 'publish', fixture.deploymentRoot, fixture.artifactRoot);

  const frontendContractRoot = join(fixture.temp, 'frontend-contract');
  await writeContractRoot(frontendContractRoot, { serviceId: 'example.com/frontend' });
  const frontendContract = await author(
    'contract',
    'publish',
    frontendContractRoot,
    fixture.artifactRoot,
  );
  const frontendRecord = await readReceiptRecord(
    fixture.artifactRoot,
    frontendContract.serviceContractReceipt,
  );
  const frontendOperationId = Object.keys(frontendRecord.operations)[0];
  const consumerRoot = join(fixture.temp, 'frontend-package');
  await writePackageRoot(consumerRoot, {
    packageId: 'example.com/frontend',
    contracts: [contractCoordinate()],
    source: [
      'function health() -> string { return "frontend" }',
      'function callProvider() -> string { return health/health() }',
      '',
    ].join('\n'),
  });
  const consumer = await author('package', 'publish', consumerRoot, fixture.artifactRoot);
  const consumerArtifact = await readReceiptRecord(
    fixture.artifactRoot,
    consumer.packageArtifactReceipt,
  );
  const frontendDeploymentRoot = join(fixture.temp, 'frontend-deployment');
  await writeDeploymentRoot(frontendDeploymentRoot, {
    contract: frontendContract.serviceContractReceipt.contract,
    implementation: consumer.packageArtifactReceipt.artifact,
    operationId: frontendOperationId,
    serviceSelectors: [{
      key: {
        callerPackageBuildId: consumer.packageArtifactReceipt.artifact.packageBuildId,
        serviceRequirementSlot: consumerArtifact.serviceRequirements[0].serviceBindingSlot,
      },
      contract: healthContract.serviceContractReceipt.contract,
    }],
  });
  const frontendDeployment = await author(
    'deployment',
    'publish',
    frontendDeploymentRoot,
    fixture.artifactRoot,
  );
  await writeAssemblyRoot(fixture.assemblyRoot, 'test', [
    frontendDeployment.serviceDeploymentReceipt.deployment,
  ]);
  const assembly = await author('assembly', 'build', fixture.assemblyRoot, fixture.artifactRoot);
  const record = await readReceiptRecord(fixture.artifactRoot, assembly.runtimeAssemblyReceipt);
  assert.equal(record.roots.length, 1);
  assert.equal(record.resolvedDeployments.length, 2);
});

test('assembly resolution rejects duplicate providers for a package service requirement', async () => {
  const fixture = await canonicalFixture('duplicate-provider');
  const contract = await author('contract', 'publish', fixture.contractRoot, fixture.artifactRoot);
  const contractRecord = await readReceiptRecord(
    fixture.artifactRoot,
    contract.serviceContractReceipt,
  );
  const operationId = Object.keys(contractRecord.operations)[0];

  const provider = await author('package', 'publish', fixture.packageRoot, fixture.artifactRoot);
  await writeDeploymentRoot(fixture.deploymentRoot, {
    contract: contract.serviceContractReceipt.contract,
    implementation: provider.packageArtifactReceipt.artifact,
    operationId,
    deploymentRevision: 'provider-a',
  });
  const providerDeployment = await author('deployment', 'publish', fixture.deploymentRoot, fixture.artifactRoot);

  const consumerRoot = join(fixture.temp, 'consumer');
  await writePackageRoot(consumerRoot, {
    packageId: 'example.com/consumer-provider',
    contracts: [contractCoordinate()],
    source: [
      'function health() -> string { return "consumer" }',
      'function callProvider() -> string { return health/health() }',
      '',
    ].join('\n'),
  });
  const consumer = await author('package', 'publish', consumerRoot, fixture.artifactRoot);
  const consumerArtifact = await readReceiptRecord(
    fixture.artifactRoot,
    consumer.packageArtifactReceipt,
  );
  assert.equal(consumerArtifact.serviceRequirements.length, 1);
  const consumerDeploymentRoot = join(fixture.temp, 'consumer-deployment');
  await writeDeploymentRoot(consumerDeploymentRoot, {
    contract: contract.serviceContractReceipt.contract,
    implementation: consumer.packageArtifactReceipt.artifact,
    operationId,
    deploymentRevision: 'provider-b',
    serviceSelectors: [{
      key: {
        callerPackageBuildId: consumer.packageArtifactReceipt.artifact.packageBuildId,
        serviceRequirementSlot: consumerArtifact.serviceRequirements[0].serviceBindingSlot,
      },
      contract: contract.serviceContractReceipt.contract,
    }],
  });
  const consumerDeployment = await author(
    'deployment',
    'publish',
    consumerDeploymentRoot,
    fixture.artifactRoot,
  );
  await writeAssemblyRoot(fixture.assemblyRoot, 'test', [
    providerDeployment.serviceDeploymentReceipt.deployment,
    consumerDeployment.serviceDeploymentReceipt.deployment,
  ]);
  await assert.rejects(
    author('assembly', 'build', fixture.assemblyRoot, fixture.artifactRoot),
    /ambiguous|provider/i,
  );
});

async function canonicalFixture(name) {
  const temp = await mkdtemp(join(tmpdir(), `skiff-${name}-`));
  const fixture = {
    temp,
    artifactRoot: join(temp, 'artifacts'),
    contractRoot: join(temp, 'contract'),
    packageRoot: join(temp, 'package'),
    deploymentRoot: join(temp, 'deployment'),
    assemblyRoot: join(temp, 'assembly'),
  };
  await writeContractRoot(fixture.contractRoot);
  await writePackageRoot(fixture.packageRoot);
  return fixture;
}

function author(kind, action, root, artifactRoot) {
  return runCompilerAuthoring({ skiffRoot, kind, action, root, artifactRoot });
}
