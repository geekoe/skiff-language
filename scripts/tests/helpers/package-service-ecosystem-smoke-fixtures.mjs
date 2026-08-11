import { createHash } from 'node:crypto';

const identity = (prefix, character) => `${prefix}:${character.repeat(64)}`;
const snapshotIdentity = (character) =>
  `skiff-runtime-config-snapshot-v1:${character.repeat(32)}`;

export const smokeFixtureIdentities = Object.freeze({
  configSnapshot: snapshotIdentity('c'),
  bootstrapConfigSnapshot: snapshotIdentity('0'),
  testServiceBuild: identity('skiff-package-build-v10:sha256', '1'),
  testServiceAbi: identity('skiff-package-local-abi-v7:sha256', '2'),
  testServiceProtocol: identity('skiff-service-protocol-v5:sha256', '3'),
  testServiceDeployment: identity('skiff-deployment-artifact-v2:sha256', '7'),
  packageTestGateway:
    'skiff-gateway-entry-v2:sha256:b97af7d9ff0b9ddbfcb6ea8b19e6173722095c99f1566ccd6b1a6fd2ead3f305',
  smokeProbeGateway:
    'skiff-gateway-entry-v2:sha256:94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705',
  stdAbi: identity('skiff-package-local-abi-v7:sha256', 'd'),
});

export function validSmokeFixtureReceipt(profile) {
  const testService = {
    packageId: 'test.skiff/package-service-websocket-smoke',
    packageVersion: '1.0.0',
    packageBuildId: smokeFixtureIdentities.testServiceBuild,
    packageLocalAbiIdentity: smokeFixtureIdentities.testServiceAbi,
  };
  const serviceId = testCaseServiceId(testService.packageId, 0);
  const contract = {
    serviceId,
    contractVersion: '1.0.0',
    serviceProtocolIdentity: smokeFixtureIdentities.testServiceProtocol,
  };
  const deployment = {
    serviceId,
    contractVersion: '1.0.0',
    deploymentRevision: `test-${hash(smokeFixtureIdentities.testServiceBuild)}-case-0`,
    deploymentArtifactIdentity: smokeFixtureIdentities.testServiceDeployment,
  };
  return {
    schemaVersion: 'skiff-package-service-smoke-fixture-v4',
    profile,
    bootstrap: null,
    candidate: {
      configSnapshot: { snapshotId: smokeFixtureIdentities.configSnapshot },
      testService,
      testServiceRecordPath:
        `records/package-artifacts/test~dskiff~spackage-service-websocket-smoke/1.0.0/${hash(testService.packageBuildId)}/package.json`,
      contracts: [contract],
      deployments: [deployment],
      entrypoints: [
        {
          deployment,
          gatewayEntryKey: 'run',
          gatewayEntryIdentity: smokeFixtureIdentities.packageTestGateway,
          mode: 'unary',
          selector: {
            protocol: 'http',
            method: 'POST',
            path: '/__skiff/test/0',
          },
        },
        {
          deployment,
          gatewayEntryKey: 'probe',
          gatewayEntryIdentity: smokeFixtureIdentities.smokeProbeGateway,
          mode: 'unary',
          selector: {
            protocol: 'http',
            method: 'POST',
            path: '/probe',
          },
        },
      ],
    },
  };
}

export function validBootstrapReceipt(profile, {
  packageBuildId = identity('skiff-package-build-v10:sha256', 'a'),
} = {}) {
  const artifact = {
    packageId: 'skiff.run/std',
    packageVersion: '1.0.0',
    packageBuildId,
    packageLocalAbiIdentity: smokeFixtureIdentities.stdAbi,
  };
  const recordPath =
    `records/package-artifacts/skiff~drun~sstd/1.0.0/${hash(artifact.packageBuildId)}/package.json`;
  return {
    schemaVersion: 'skiff-package-service-bootstrap-v3',
    profile,
    bootstrap: {
      configSnapshot: {
        snapshotId: smokeFixtureIdentities.bootstrapConfigSnapshot,
      },
      std: {
        package: {
          artifact,
          recordPath,
          fileIrRecordPaths: [
            `records/package-artifacts/skiff~drun~sstd/1.0.0/${hash(artifact.packageBuildId)}/file-ir/${'e'.repeat(64)}.json`,
          ],
          resourceRecordPaths: [],
        },
        pointer: {
          schemaVersion: 'skiff-package-artifact-pointer-v1',
          artifact,
          recordPath,
        },
        pointerPath: 'pointers/package-artifacts/skiff~drun~sstd/1.0.0.json',
      },
    },
  };
}

function hash(identityValue) {
  return identityValue.slice(identityValue.lastIndexOf(':') + 1);
}

function testCaseServiceId(packageId, caseIndex) {
  const packageDigest =
    createHash('sha256').update(packageId).digest('hex').slice(0, 16);
  return `test.skiff/p-${packageDigest}/e-0123456789abcdef/case-${caseIndex}`;
}
