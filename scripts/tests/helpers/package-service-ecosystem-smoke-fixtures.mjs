const identity = (prefix, character) => `${prefix}:${character.repeat(64)}`;

export const smokeFixtureIdentities = Object.freeze({
  assembly: identity('skiff-runtime-assembly-v3:sha256', 'a'),
  bootstrapAssembly: identity('skiff-runtime-assembly-v3:sha256', '0'),
  productionBuild: identity('skiff-package-build-v10:sha256', '1'),
  productionAbi: identity('skiff-package-local-abi-v7:sha256', '2'),
  overlayBuild: identity('skiff-package-build-v10:sha256', '3'),
  overlayAbi: identity('skiff-package-local-abi-v7:sha256', '4'),
  packageTestDeployment: identity('skiff-deployment-artifact-v2:sha256', '7'),
  smokeDeployment: identity('skiff-deployment-artifact-v2:sha256', '8'),
  packageTestGateway:
    'skiff-gateway-entry-v1:sha256:cfcfced94f984612809ce837f81e975016b09f206925389d95e925e087fc32d4',
  smokeProbeGateway:
    'skiff-gateway-entry-v1:sha256:adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653',
  stdAbi: identity('skiff-package-local-abi-v7:sha256', 'd'),
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
  const packageTestDeployment = {
    serviceId:
      'test.skiff/test.skiff/package-service-websocket-smoke/case-0',
    contractVersion: '1.0.0',
    deploymentRevision: `test-${hash(smokeFixtureIdentities.overlayBuild)}`,
    deploymentArtifactIdentity: smokeFixtureIdentities.packageTestDeployment,
  };
  const smokeDeployment = {
    serviceId: 'test.skiff/ecosystem-smoke',
    contractVersion: '1.0.0',
    deploymentRevision: `smoke-${hash(smokeFixtureIdentities.productionBuild)}`,
    deploymentArtifactIdentity: smokeFixtureIdentities.smokeDeployment,
  };
  return {
    schemaVersion: 'skiff-package-service-smoke-fixture-v2',
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
          deployment: packageTestDeployment,
          gatewayEntryKey: 'run',
          gatewayEntryIdentity: smokeFixtureIdentities.packageTestGateway,
          mode: 'unary',
          selector: {
            protocol: 'http',
            host: 'case-0.package-test.skiff.localhost',
            method: 'POST',
            path: '/__skiff/package-test/0',
          },
        },
        {
          deployment: smokeDeployment,
          gatewayEntryKey: 'probe',
          gatewayEntryIdentity: smokeFixtureIdentities.smokeProbeGateway,
          mode: 'unary',
          selector: {
            protocol: 'http',
            host: 'ecosystem-smoke.skiff.localhost',
            method: 'POST',
            path: '/probe',
          },
        },
      ],
    },
  };
}

export function validBootstrapReceipt(environment, {
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
      ingressCount: 2,
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
