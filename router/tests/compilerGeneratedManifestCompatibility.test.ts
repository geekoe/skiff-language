import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import { writeCompilerGeneratedFixtureArtifactRoot } from './helpers/compilerArtifacts.js';

describe('compiler generated RuntimeAssembly compatibility', () => {
  it(
    'loads current package authoring output through the production Router loader',
    async () => {
      const root = await mkdtemp(join(tmpdir(), 'skiff-router-authoring-'));
      try {
        const generated = await writeCompilerGeneratedFixtureArtifactRoot(root);
        expect(generated.packageValue.schemaVersion).toBe('skiff-package-artifact-v3');
        expect(generated.contractValue.schemaVersion).toBe('skiff-service-contract-v3');
        expect(generated.serviceContract.contract.serviceProtocolIdentity).toMatch(
          /^skiff-service-protocol-v3:sha256:[0-9a-f]{64}$/
        );
        expect(generated.deploymentValue.schemaVersion).toBe('skiff-service-deployment-v1');
        expect(generated.assemblyValue.schemaVersion).toBe('skiff-runtime-assembly-v1');

        const loaded = await new FilesystemRuntimeAssemblySnapshotLoader(root).load(
          generated.runtimeAssembly.assembly
        );
        expect(loaded.assemblyIdentity).toBe(
          generated.runtimeAssembly.assembly.assemblyIdentity
        );
        expect(loaded.resolvedContracts).toContainEqual(
          generated.serviceContract.contract
        );
        expect(loaded.globalIngress).toContainEqual(expect.objectContaining({
          selector: {
            protocol: 'http',
            host: 'websocket-fixture.skiff.localhost',
            method: 'GET',
            path: '/ping',
          },
          deployment: generated.serviceDeployment.deployment,
          contract: generated.serviceContract.contract,
          contractOperationId: expect.stringMatching(
            /^skiff-contract-operation-v1:sha256:[0-9a-f]{64}$/
          ),
        }));
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    },
    120_000
  );
});
