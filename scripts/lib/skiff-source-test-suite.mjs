import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { runOwnedCommand } from './owned-command.mjs';
import {
  canonicalSkiffSourceTestRegistry,
  createCanonicalSkiffSourceTestPlan,
} from './skiff-source-test-registry.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultSkiffRoot = resolve(scriptDir, '..', '..');
const hostFixtureRelativeRoot = join('test-runner', 'fixtures', 'package-service-host');
const hostConsumerRelativeRoot = join(hostFixtureRelativeRoot, 'consumer');
const hostReceiptSchemaVersion = 'skiff-package-service-host-fixture-v1';

export function skiffSourceTestRunnerCargoArgs({
  skiffRoot,
  root,
  artifactRoot,
  baseAssembly,
}) {
  return [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--',
    root,
    '--artifact-root',
    artifactRoot,
    ...(baseAssembly === undefined ? [] : ['--base-assembly', baseAssembly]),
    '--deny-skips',
    '--require-tests',
  ];
}

export function packageServiceHostFixturePaths({ skiffRoot, tempRoot }) {
  const fixtureRoot = resolve(skiffRoot, hostFixtureRelativeRoot);
  return Object.freeze({
    fixtureRoot,
    consumerRoot: resolve(skiffRoot, hostConsumerRelativeRoot),
    workRoot: join(tempRoot, 'package-service-host-work'),
    receipt: join(tempRoot, 'package-service-host-receipt.json'),
  });
}

export function packageServiceHostFixturePrepareCargoArgs({
  skiffRoot,
  fixtureRoot,
  artifactRoot,
  workRoot,
  receipt,
  environment,
}) {
  return [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-package-service-smoke-fixture',
    '--',
    '--prepare-host-base',
    fixtureRoot,
    '--work-root',
    workRoot,
    '--receipt',
    receipt,
    '--artifact-root',
    artifactRoot,
    '--environment',
    environment,
  ];
}

export async function readPackageServiceHostFixtureReceipt(path, expectedEnvironment) {
  const receipt = JSON.parse(await readFile(path, 'utf8'));
  exactKeys(receipt, [
    'baseAssembly',
    'contracts',
    'deployments',
    'environment',
    'packages',
    'schemaVersion',
  ], 'host fixture receipt');
  if (receipt.schemaVersion !== hostReceiptSchemaVersion) {
    throw new Error(`host fixture receipt schemaVersion must be ${hostReceiptSchemaVersion}`);
  }
  if (receipt.environment !== expectedEnvironment) {
    throw new Error(`host fixture receipt environment must be ${expectedEnvironment}`);
  }
  exactKeys(receipt.contracts, ['consumer', 'payments'], 'host fixture contracts');
  exactKeys(receipt.packages, ['consumer', 'helper', 'provider'], 'host fixture packages');
  exactKeys(receipt.deployments, ['consumer', 'provider'], 'host fixture deployments');
  validateContractRef(receipt.contracts.payments, 'payments contract');
  validateContractRef(receipt.contracts.consumer, 'consumer contract');
  validatePackageRef(receipt.packages.helper, 'helper package');
  validatePackageRef(receipt.packages.provider, 'provider package');
  validatePackageRef(receipt.packages.consumer, 'consumer package');
  validateDeploymentRef(receipt.deployments.provider, 'provider deployment');
  validateDeploymentRef(receipt.deployments.consumer, 'consumer deployment');
  exactKeys(receipt.baseAssembly, ['assemblyIdentity'], 'base assembly');
  const assemblyIdentity = requiredText(
    receipt.baseAssembly.assemblyIdentity,
    'base assembly assemblyIdentity',
  );
  if (!/^skiff-runtime-assembly-v1:sha256:[a-f0-9]{64}$/.test(assemblyIdentity)) {
    throw new Error('base assembly assemblyIdentity must be canonical');
  }
  return receipt;
}

export async function runCanonicalSkiffSourceTests({
  skiffRoot = defaultSkiffRoot,
  registry = canonicalSkiffSourceTestRegistry,
  runtimeOwner = runInIsolatedTestRuntime,
  runCommand = runOwnedCommand,
  readHostReceipt = readPackageServiceHostFixtureReceipt,
  log = console.log,
} = {}) {
  const plan = createCanonicalSkiffSourceTestPlan({ skiffRoot, registry });
  await runtimeOwner({
    skiffRoot,
    runTest: async (isolatedEnv, signal, stack) => {
      if (stack?.sourceArtifactRoot === undefined) {
        throw new Error('isolated runtime owner omitted the canonical source artifact root');
      }
      for (const [index, entry] of plan.entries()) {
        log(`[skiff-tests] running ${entry.id}: ${entry.root}`);
        await runCommand(
          'cargo',
          skiffSourceTestRunnerCargoArgs({
            skiffRoot,
            root: entry.absoluteRoot,
            artifactRoot: stack.sourceArtifactRoot,
          }),
          {
            cwd: skiffRoot,
            env: {
              ...isolatedEnv,
              SKIFF_TEST_EXPECTED_GENERATION: String(index),
            },
            signal,
          },
        );
      }
      if (typeof stack.tempRoot !== 'string' || stack.tempRoot.length === 0) {
        throw new Error('isolated runtime owner omitted its temporary workspace');
      }
      const environment = requiredText(
        isolatedEnv.SKIFF_TEST_ENVIRONMENT,
        'isolated runtime environment',
      );
      const host = packageServiceHostFixturePaths({ skiffRoot, tempRoot: stack.tempRoot });
      log(`[skiff-tests] preparing package-service-host: ${host.fixtureRoot}`);
      await runCommand(
        'cargo',
        packageServiceHostFixturePrepareCargoArgs({
          skiffRoot,
          fixtureRoot: host.fixtureRoot,
          artifactRoot: stack.sourceArtifactRoot,
          workRoot: host.workRoot,
          receipt: host.receipt,
          environment,
        }),
        { cwd: skiffRoot, env: isolatedEnv, signal },
      );
      const receipt = await readHostReceipt(host.receipt, environment);
      log(`[skiff-tests] running package-service-host: ${hostConsumerRelativeRoot}`);
      await runCommand(
        'cargo',
        skiffSourceTestRunnerCargoArgs({
          skiffRoot,
          root: host.consumerRoot,
          artifactRoot: stack.sourceArtifactRoot,
          baseAssembly: receipt.baseAssembly.assemblyIdentity,
        }),
        {
          cwd: skiffRoot,
          env: {
            ...isolatedEnv,
            SKIFF_TEST_EXPECTED_GENERATION: String(plan.length),
          },
          signal,
        },
      );
    },
  });
  return plan;
}

function validateContractRef(value, label) {
  exactKeys(
    value,
    ['contractVersion', 'serviceId', 'serviceProtocolIdentity'],
    label,
  );
  requiredText(value.contractVersion, `${label} contractVersion`);
  requiredText(value.serviceId, `${label} serviceId`);
  requiredText(value.serviceProtocolIdentity, `${label} serviceProtocolIdentity`);
}

function validatePackageRef(value, label) {
  exactKeys(
    value,
    ['packageBuildId', 'packageId', 'packageLocalAbiIdentity', 'packageVersion'],
    label,
  );
  requiredText(value.packageBuildId, `${label} packageBuildId`);
  requiredText(value.packageId, `${label} packageId`);
  requiredText(value.packageLocalAbiIdentity, `${label} packageLocalAbiIdentity`);
  requiredText(value.packageVersion, `${label} packageVersion`);
}

function validateDeploymentRef(value, label) {
  exactKeys(
    value,
    ['contractVersion', 'deploymentArtifactIdentity', 'deploymentRevision', 'serviceId'],
    label,
  );
  requiredText(value.contractVersion, `${label} contractVersion`);
  requiredText(value.deploymentArtifactIdentity, `${label} deploymentArtifactIdentity`);
  requiredText(value.deploymentRevision, `${label} deploymentRevision`);
  requiredText(value.serviceId, `${label} serviceId`);
}

function exactKeys(value, expected, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  if (
    actual.length !== sortedExpected.length
    || actual.some((key, index) => key !== sortedExpected[index])
  ) {
    throw new Error(`${label} must contain exactly ${sortedExpected.join(', ')}`);
  }
}

function requiredText(value, label) {
  if (typeof value !== 'string' || value.trim() !== value || value.length === 0) {
    throw new Error(`${label} must be a non-empty trimmed string`);
  }
  return value;
}
