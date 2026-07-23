const identity = (prefix, character) => `${prefix}:${character.repeat(64)}`;

export const smokeFixtureIdentities = Object.freeze({
  assembly: identity('skiff-runtime-assembly-v1:sha256', 'a'),
  bootstrapAssembly: identity('skiff-runtime-assembly-v1:sha256', '0'),
  productionBuild: identity('skiff-package-build-v4:sha256', '1'),
  productionAbi: identity('skiff-package-local-abi-v3:sha256', '2'),
  overlayBuild: identity('skiff-package-build-v4:sha256', '3'),
  overlayAbi: identity('skiff-package-local-abi-v3:sha256', '4'),
  packageTestProtocol: identity('skiff-service-protocol-v2:sha256', '5'),
  smokeProtocol: identity('skiff-service-protocol-v2:sha256', '6'),
  packageTestDeployment: identity('skiff-deployment-artifact-v1:sha256', '7'),
  smokeDeployment: identity('skiff-deployment-artifact-v1:sha256', '8'),
  packageTestOperation: identity('skiff-contract-operation-v1:sha256', '9'),
  unaryOperation: identity('skiff-contract-operation-v1:sha256', 'b'),
  websocketOperation: identity('skiff-contract-operation-v1:sha256', 'c'),
  stdBuild:
    'skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4',
  stdAbi: identity('skiff-package-local-abi-v3:sha256', 'd'),
});

export function validSmokeFixtureReceipt(environment) {
  const production = {
    packageId: 'test.skiff/package-service-websocket-smoke',
    packageVersion: '1.0.0',
    packageBuildId: smokeFixtureIdentities.productionBuild,
    packageLocalAbiIdentity: smokeFixtureIdentities.productionAbi,
  };
  const overlay = {
    packageId: production.packageId,
    packageVersion: production.packageVersion,
    packageBuildId: smokeFixtureIdentities.overlayBuild,
    packageLocalAbiIdentity: smokeFixtureIdentities.overlayAbi,
  };
  const packageTestContract = {
    serviceId:
      'test.skiff/package/test.skiff/package-service-websocket-smoke',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: smokeFixtureIdentities.packageTestProtocol,
  };
  const smokeContract = {
    serviceId: 'test.skiff/ecosystem-smoke',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: smokeFixtureIdentities.smokeProtocol,
  };
  const packageTestDeployment = {
    serviceId: packageTestContract.serviceId,
    contractVersion: packageTestContract.contractVersion,
    deploymentRevision: `test-${hash(smokeFixtureIdentities.overlayBuild)}`,
    deploymentArtifactIdentity: smokeFixtureIdentities.packageTestDeployment,
  };
  const smokeDeployment = {
    serviceId: smokeContract.serviceId,
    contractVersion: smokeContract.contractVersion,
    deploymentRevision: `smoke-${hash(smokeFixtureIdentities.productionBuild)}`,
    deploymentArtifactIdentity: smokeFixtureIdentities.smokeDeployment,
  };
  return {
    schemaVersion: 'skiff-package-service-smoke-fixture-v1',
    environment,
    bootstrap: null,
    candidate: {
      assembly: { assemblyIdentity: smokeFixtureIdentities.assembly },
      production,
      overlay,
      overlayRecordPath:
        `records/package-artifacts/test~dskiff~spackage-service-websocket-smoke/1.0.0/${hash(overlay.packageBuildId)}/package.json`,
      entrypoints: [
        {
          kind: 'packageTest',
          name: 'normal source fixture compiles',
          host: 'case-0.package-test.skiff.localhost',
          method: 'POST',
          path: '/__skiff/package-test/0',
          deployment: packageTestDeployment,
          contract: packageTestContract,
          operation: smokeFixtureIdentities.packageTestOperation,
        },
        {
          kind: 'unary',
          name: 'marker',
          host: 'ecosystem-smoke.skiff.localhost',
          method: 'POST',
          path: '/probe',
          deployment: smokeDeployment,
          contract: smokeContract,
          operation: smokeFixtureIdentities.unaryOperation,
        },
        {
          kind: 'websocket',
          name: 'websocket',
          host: 'ecosystem-smoke.skiff.localhost',
          method: null,
          path: '/socket',
          deployment: smokeDeployment,
          contract: smokeContract,
          operation: smokeFixtureIdentities.websocketOperation,
        },
      ],
    },
  };
}

export function validBootstrapReceipt(environment) {
  const artifact = {
    packageId: 'skiff.run/std',
    packageVersion: '1.0.0',
    packageBuildId: smokeFixtureIdentities.stdBuild,
    packageLocalAbiIdentity: smokeFixtureIdentities.stdAbi,
  };
  const recordPath =
    `records/package-artifacts/skiff~drun~sstd/1.0.0/${hash(artifact.packageBuildId)}/package.json`;
  return {
    schemaVersion: 'skiff-package-service-bootstrap-v1',
    environment,
    bootstrap: {
      assembly: { assemblyIdentity: smokeFixtureIdentities.bootstrapAssembly },
      generation: 0,
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

export function validActivationReceipt(environment) {
  return {
    request: {
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment,
      activationId: 'p5-f27c-test',
      expectedGeneration: 0,
      assembly: { assemblyIdentity: smokeFixtureIdentities.assembly },
    },
    response: {
      ok: true,
      committed: {
        generation: 1,
        assembly: { assemblyIdentity: smokeFixtureIdentities.assembly },
      },
      activeAssembly: {
        environment,
        generation: 1,
        assemblyIdentity: smokeFixtureIdentities.assembly,
      },
      replicas: [],
    },
  };
}

export function readyAssemblyHealth(environment, overrides = {}) {
  const replicaId = 'runtime-f27c';
  const base = {
    ok: true,
    activeAssembly: {
      environment,
      generation: 1,
      assemblyIdentity: smokeFixtureIdentities.assembly,
      ingressCount: 3,
    },
    pendingActivation: null,
    capabilityConnections: [{
      runtimeId: replicaId,
      connected: true,
      registeredAt: '2026-07-23T00:00:00.000Z',
      capabilities: { runtimeProgram: true },
    }],
    replicas: [{
      replicaId,
      environment,
      generation: 1,
      assemblyIdentity: smokeFixtureIdentities.assembly,
      state: 'healthy',
      connected: true,
      inFlightCount: 0,
      registeredAt: '2026-07-23T00:00:00.000Z',
    }],
  };
  return {
    ...base,
    ...overrides,
    activeAssembly: {
      ...base.activeAssembly,
      ...(overrides.activeAssembly ?? {}),
    },
  };
}

function hash(identityValue) {
  return identityValue.slice(identityValue.lastIndexOf(':') + 1);
}
