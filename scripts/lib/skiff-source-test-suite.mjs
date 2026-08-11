import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { runOwnedCommand } from './owned-command.mjs';
import {
  canonicalSkiffSourceTestRegistry,
  createCanonicalSkiffSourceTestPlan,
} from './skiff-source-test-registry.mjs';
import { bootstrapCanonicalArgs } from './isolated-test-runtime-instance.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultSkiffRoot = resolve(scriptDir, '..', '..');
const hostFixtureRelativeRoot = join('test-runner', 'fixtures', 'package-service-host');
const hostConsumerRelativeRoot = join(hostFixtureRelativeRoot, 'consumer');
const hostTestRelativeRoot = join(hostFixtureRelativeRoot, 'consumer-tests');
const hostReceiptSchemaVersion = 'skiff-package-service-host-fixture-v2';

export function skiffSourceTestRunnerCargoArgs({
  skiffRoot,
  root,
  artifactRoot,
  baseConfigSnapshot,
}) {
  return [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--bin',
    'skiff-test-runner',
    '--',
    root,
    '--artifact-root',
    artifactRoot,
    '--platform-source-root',
    resolve(skiffRoot),
    ...(baseConfigSnapshot === undefined
      ? []
      : ['--base-config-snapshot', baseConfigSnapshot]),
    '--deny-skips',
    '--require-tests',
  ];
}

export function skiffSourceSubjectPublishArgs({
  skiffRoot,
  subjectRoot,
  artifactRoot,
}) {
  return [
    join(skiffRoot, 'scripts', 'skiff.mjs'),
    'package',
    'publish',
    subjectRoot,
    '--artifact-root',
    artifactRoot,
  ];
}

export function skiffSourceArtifactBootstrapCargoArgs({
  skiffRoot,
  artifactRoot,
  profile,
}) {
  return bootstrapCanonicalArgs({ skiffRoot, artifactRoot, profile });
}

export function packageServiceHostFixturePaths({ skiffRoot, tempRoot }) {
  const fixtureRoot = resolve(skiffRoot, hostFixtureRelativeRoot);
  return Object.freeze({
    fixtureRoot,
    consumerRoot: resolve(skiffRoot, hostConsumerRelativeRoot),
    testRoot: resolve(skiffRoot, hostTestRelativeRoot),
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
  profile,
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
    '--platform-source-root',
    resolve(skiffRoot),
    '--profile',
    profile,
  ];
}

export async function readPackageServiceHostFixtureReceipt(path, expectedProfile) {
  const receipt = JSON.parse(await readFile(path, 'utf8'));
  exactKeys(receipt, [
    'baseConfigSnapshot',
    'contracts',
    'deployments',
    'profile',
    'packages',
    'schemaVersion',
  ], 'host fixture receipt');
  if (receipt.schemaVersion !== hostReceiptSchemaVersion) {
    throw new Error(`host fixture receipt schemaVersion must be ${hostReceiptSchemaVersion}`);
  }
  if (receipt.profile !== expectedProfile) {
    throw new Error(`host fixture receipt profile must be ${expectedProfile}`);
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
  exactKeys(receipt.baseConfigSnapshot, ['snapshotId'], 'base config snapshot');
  if (
    !/^skiff-runtime-config-snapshot-v1:[a-f0-9]{32}$/.test(
      receipt.baseConfigSnapshot.snapshotId,
    )
  ) {
    throw new Error('base config snapshot snapshotId must be canonical');
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
  log('[skiff-tests] phase startup: isolated-runtime');
  await runtimeOwner({
    skiffRoot,
    runTest: async (isolatedEnv, signal, stack) => {
      if (stack?.sourceArtifactRoot === undefined) {
        throw new Error('isolated runtime owner omitted the canonical source artifact root');
      }
      const profile = requiredText(
        isolatedEnv.SKIFF_TEST_ENVIRONMENT,
        'isolated runtime profile',
      );
      log(`[skiff-tests] bootstrapping source artifacts: ${stack.sourceArtifactRoot}`);
      await runCommand(
        'cargo',
        skiffSourceArtifactBootstrapCargoArgs({
          skiffRoot,
          artifactRoot: stack.sourceArtifactRoot,
          profile,
        }),
        { cwd: skiffRoot, env: isolatedEnv, signal },
      );
      for (const entry of plan) {
        if (entry.absoluteSubjectRoot !== undefined) {
          log(`[skiff-tests] publishing ${entry.id} subject: ${entry.subjectRoot}`);
          await runCommand(
            process.execPath,
            skiffSourceSubjectPublishArgs({
              skiffRoot,
              subjectRoot: entry.absoluteSubjectRoot,
              artifactRoot: stack.sourceArtifactRoot,
            }),
            { cwd: skiffRoot, env: isolatedEnv, signal },
          );
        }
        log(`[skiff-tests] running ${entry.id}: ${entry.root}`);
        await runCommand(
          'cargo',
          skiffSourceTestRunnerCargoArgs({
            skiffRoot,
            root: entry.absoluteRoot,
            artifactRoot: stack.sourceArtifactRoot,
          }),
          { cwd: skiffRoot, env: isolatedEnv, signal },
        );
      }
      if (typeof stack.tempRoot !== 'string' || stack.tempRoot.length === 0) {
        throw new Error('isolated runtime owner omitted its temporary workspace');
      }
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
          profile,
        }),
        { cwd: skiffRoot, env: isolatedEnv, signal },
      );
      const receipt = await readHostReceipt(host.receipt, profile);
      log(`[skiff-tests] running package-service-host: ${hostTestRelativeRoot}`);
      await runCommand(
        'cargo',
        skiffSourceTestRunnerCargoArgs({
          skiffRoot,
          root: host.testRoot,
          artifactRoot: stack.sourceArtifactRoot,
          baseConfigSnapshot: receipt.baseConfigSnapshot.snapshotId,
        }),
        { cwd: skiffRoot, env: isolatedEnv, signal },
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

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}
