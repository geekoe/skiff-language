import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import { joinRuntimeAssemblyDeployments } from '../src/router/runtimeAssemblyDeploymentSnapshot.js';
import type { DecodedRuntimeAssemblyRecord } from '../src/router/runtimeAssemblySnapshot.js';
import {
  writeCompilerGeneratedFixtureArtifactRoot,
  writeCurrentScopeCompilerGeneratedArtifactRoot,
} from './helpers/compilerArtifacts.js';

describe('compiler generated HTTP gateway compatibility', () => {
  it(
    'joins and loads the typed-null gateway from exact current records',
    async () => {
      const root = await mkdtemp(join(tmpdir(), 'skiff-router-authoring-'));
      try {
        const generated = await writeCompilerGeneratedFixtureArtifactRoot(root);
        expect(generated.packageValue.schemaVersion).toBe('skiff-package-artifact-v9');
        expect(generated.packageArtifact.artifact.packageBuildId).toMatch(
          /^skiff-package-build-v10:sha256:[0-9a-f]{64}$/
        );
        expect(generated.packageValue.files).toEqual([
          expect.objectContaining({
            fileIrIdentity: expect.stringMatching(
              /^skiff-file-ir-v9:sha256:[0-9a-f]{64}$/
            ),
          }),
        ]);
        expect(generated.packageValue).not.toHaveProperty('serviceCallRoots');
        const packageLocalAbi = recordField(
          generated.packageValue,
          'packageLocalAbi'
        );
        expect(packageLocalAbi.publicSymbols).toEqual({});
        const implementationSymbols = recordField(
          packageLocalAbi,
          'implementationSymbols'
        );
        expect(Object.keys(implementationSymbols)).toEqual([
          'main.__skiffHttpPing',
          'main.ping',
        ]);
        expect(recordField(implementationSymbols, 'main.ping').signature).toEqual({
          maySuspend: false,
          parameters: [],
          returnType: {
            kind: 'local',
            localType: { kind: 'builtin', name: 'string' },
          },
          typeParams: [],
        });
        expect(
          recordField(implementationSymbols, 'main.__skiffHttpPing').signature
        ).toEqual({
          maySuspend: false,
          parameters: [
            {
              name: 'body',
              ty: {
                kind: 'local',
                localType: { kind: 'builtin', name: 'null' },
              },
            },
          ],
          returnType: {
            kind: 'local',
            localType: { kind: 'builtin', name: 'string' },
          },
          typeParams: [],
        });

        expect(generated.contractValue.schemaVersion).toBe('skiff-service-contract-v5');
        expect(generated.contractValue.operations).toEqual({});
        expect(generated.serviceContract.contract.serviceProtocolIdentity).toMatch(
          /^skiff-service-protocol-v5:sha256:[0-9a-f]{64}$/
        );
        expect(generated.deploymentValue.schemaVersion).toBe('skiff-service-deployment-v4');
        expect(
          generated.serviceDeployment.deployment.deploymentArtifactIdentity
        ).toMatch(
          /^skiff-deployment-artifact-v4:sha256:[0-9a-f]{64}$/
        );
        expect(generated.deploymentValue.operationBindings).toEqual([]);

        const gatewayEntries = recordField(
          generated.deploymentValue,
          'gatewayEntries'
        );
        expect(Object.keys(gatewayEntries)).toEqual(['ping']);
        const pingGateway = recordField(gatewayEntries, 'ping');
        const gatewayEntryIdentity = stringField(
          pingGateway,
          'gatewayEntryIdentity'
        );
        expect(gatewayEntryIdentity).toBe(
          'skiff-gateway-entry-v2:sha256:94d4fb9ed499a8e4717ac6a46eb716a4595445573808f2543b7ea5aeefe83705'
        );
        expect(pingGateway).toEqual({
          gatewayEntryIdentity,
          protocolSurface: {
            protocol: {
              kind: 'http',
              surface: {
                adapterKind: 'typedJson',
                dispatchMode: 'unary',
                externalSources: [{ kind: 'http.body' }],
                requestBodySchema: { kind: 'null' },
                responseSchema: { kind: 'string' },
                streamItemSchema: null,
              },
            },
            externalErrorProjection: { kind: 'fixed', version: 'v1' },
          },
          handler:
            'pkg-callable:example.com/websocket_fixture_implementation:top-level:main.__skiffHttpPing',
          pre: null,
          guard: null,
          adapterPlan: {
            kind: 'typedJson',
            args: [{ param: 'body', source: { kind: 'http.body' } }],
          },
        });
        expect(generated.deploymentValue.ingress).toEqual([
          {
            selector: {
              protocol: 'http',
              method: 'GET',
              path: '/ping',
            },
            gatewayEntryKey: 'ping',
          },
        ]);

        expect(generated.assemblyValue.schemaVersion).toBe('skiff-runtime-assembly-v3');
        expect(generated.assemblyValue.roots).toEqual([
          generated.serviceDeployment.deployment,
        ]);
        expect(generated.assemblyValue.resolvedDeployments).toEqual([
          generated.serviceDeployment.deployment,
        ]);
        expect(generated.assemblyValue.resolvedContracts).toEqual([
          generated.serviceContract.contract,
        ]);
        expect(generated.assemblyValue.resolvedPackages).toEqual([
          generated.packageArtifact.artifact,
        ]);
        expect(generated.assemblyValue.gatewayIngress).toEqual([
          {
            selector: {
              protocol: 'http',
              method: 'GET',
              path: '/ping',
            },
            deployment: generated.serviceDeployment.deployment,
            gatewayEntryKey: 'ping',
            gatewayEntryIdentity,
          },
        ]);

        const loaded = joinRuntimeAssemblyDeployments(
          generated.assemblyValue as unknown as DecodedRuntimeAssemblyRecord,
          [generated.deploymentValue]
        );
        expect(loaded.assemblyIdentity).toBe(
          generated.runtimeAssembly.assembly.assemblyIdentity
        );
        expect(loaded.resolvedContracts).toContainEqual(
          generated.serviceContract.contract
        );
        expect(loaded.gatewayIngress).toEqual([
          {
            selector: {
              protocol: 'http',
              method: 'GET',
              path: '/ping',
            },
            deployment: generated.serviceDeployment.deployment,
            gatewayEntryKey: 'ping',
            gatewayEntryIdentity,
            adapterKind: 'typedJson',
            operationMode: 'unary',
            timeoutMs: 120_000,
          },
        ]);

        const filesystemLoaded = await new FilesystemRuntimeAssemblySnapshotLoader(
          root
        ).load(generated.runtimeAssembly.assembly);
        expect(filesystemLoaded).toEqual(loaded);
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    },
    120_000
  );

  it(
    'loads the exact S0 current-scope source artifact closure',
    async () => {
      const root = await mkdtemp(join(tmpdir(), 'skiff-router-current-scope-'));
      try {
        const generated =
          await writeCurrentScopeCompilerGeneratedArtifactRoot(root);
        expect(generated.receipt.baseAssembly.assemblyIdentity).toBe(
          'skiff-runtime-assembly-v3:sha256:ce8c979de4c6786ee9c2fbf2ad01fbfa2271b33a074682e2e66f5a77654f6688'
        );
        expect(
          generated.receipt.packages.consumer.packageBuildId
        ).toBe(
          'skiff-package-build-v10:sha256:9b03476e93f5ccb66dc69ff899f4a8fb9c68593e70c5aeda94d4e865aab688ad'
        );
        expect(
          generated.receipt.contracts.consumer.serviceProtocolIdentity
        ).toBe(
          'skiff-service-protocol-v5:sha256:9ea7ac440bd594ef31632c1c1914b40f2e92957e7fb0f73f587f4cb4d8563fa5'
        );
        expect(
          generated.receipt.deployments.consumer.deploymentArtifactIdentity
        ).toBe(
          'skiff-deployment-artifact-v4:sha256:bfa01d12d90d7a9e5af9da153b63862270a52eaffe59383a4563cff2a0dde2a4'
        );

        const loaded = await new FilesystemRuntimeAssemblySnapshotLoader(
          root
        ).load(generated.receipt.baseAssembly);
        expect(loaded.assemblyIdentity).toBe(
          generated.receipt.baseAssembly.assemblyIdentity
        );
        expect(loaded.resolvedDeployments).toEqual(
          expect.arrayContaining([
            generated.receipt.deployments.consumer,
            generated.receipt.deployments.provider,
          ])
        );
        expect(loaded.resolvedContracts).toEqual(
          expect.arrayContaining([
            generated.receipt.contracts.consumer,
            generated.receipt.contracts.payments,
          ])
        );
        expect(loaded.actorMethods).toEqual([
          expect.objectContaining({
            declarationOwner: expect.objectContaining({
              actorSymbol: 'Counter',
            }),
          }),
        ]);
        expect(
          loaded.gatewayIngress.map((binding) => ({
            protocol: binding.selector.protocol,
            path: binding.selector.path,
            mode: binding.operationMode,
            gatewayEntryIdentity: binding.gatewayEntryIdentity,
          }))
        ).toEqual([
          {
            protocol: 'http',
            path: '/current-scope/stream',
            mode: 'serverStream',
            gatewayEntryIdentity:
              'skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0',
          },
          {
            protocol: 'http',
            path: '/current-scope/unary',
            mode: 'unary',
            gatewayEntryIdentity:
              'skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2',
          },
          {
            protocol: 'webSocket',
            path: '/current-scope/socket',
            mode: 'unary',
            gatewayEntryIdentity:
              'skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d',
          },
        ]);
      } finally {
        await rm(root, { recursive: true, force: true });
      }
    },
    120_000
  );
});

function recordField(
  value: Record<string, unknown>,
  field: string
): Record<string, unknown> {
  const fieldValue = value[field];
  if (
    fieldValue === null ||
    typeof fieldValue !== 'object' ||
    Array.isArray(fieldValue)
  ) {
    throw new Error(`${field} must be an object`);
  }
  return fieldValue as Record<string, unknown>;
}

function stringField(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== 'string' || fieldValue.length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return fieldValue;
}
