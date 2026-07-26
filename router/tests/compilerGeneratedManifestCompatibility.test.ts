import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import { joinRuntimeAssemblyDeployments } from '../src/router/runtimeAssemblyDeploymentSnapshot.js';
import type { DecodedRuntimeAssemblyRecord } from '../src/router/runtimeAssemblySnapshot.js';
import { writeCompilerGeneratedFixtureArtifactRoot } from './helpers/compilerArtifacts.js';

describe('compiler generated HTTP gateway compatibility', () => {
  it(
    'joins the typed-null gateway after isolating the known contract-identity skew',
    async () => {
      const root = await mkdtemp(join(tmpdir(), 'skiff-router-authoring-'));
      try {
        const generated = await writeCompilerGeneratedFixtureArtifactRoot(root);
        expect(generated.packageValue.schemaVersion).toBe('skiff-package-artifact-v7');
        expect(generated.packageArtifact.artifact.packageBuildId).toMatch(
          /^skiff-package-build-v8:sha256:[0-9a-f]{64}$/
        );
        expect(generated.packageValue.files).toEqual([
          expect.objectContaining({
            fileIrIdentity: expect.stringMatching(
              /^skiff-file-ir-v8:sha256:[0-9a-f]{64}$/
            ),
          }),
        ]);
        expect(generated.packageValue.serviceCallRoots).toEqual([]);
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

        expect(generated.contractValue.schemaVersion).toBe('skiff-service-contract-v4');
        expect(generated.contractValue.operations).toEqual({});
        expect(generated.serviceContract.contract.serviceProtocolIdentity).toMatch(
          /^skiff-service-protocol-v4:sha256:[0-9a-f]{64}$/
        );
        expect(generated.deploymentValue.schemaVersion).toBe('skiff-service-deployment-v2');
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
          'skiff-gateway-entry-v1:sha256:adfaa17c077af0388f2b5751bbe4b9ba392ec647f5ce33022c8e8ec83eaf6653'
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
              host: 'websocket-fixture.skiff.localhost',
              method: 'GET',
              path: '/ping',
            },
            gatewayEntryKey: 'ping',
          },
        ]);

        expect(generated.assemblyValue.schemaVersion).toBe('skiff-runtime-assembly-v2');
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
              host: 'websocket-fixture.skiff.localhost',
              method: 'GET',
              path: '/ping',
            },
            deployment: generated.serviceDeployment.deployment,
            gatewayEntryKey: 'ping',
            gatewayEntryIdentity,
          },
        ]);

        // Isolate the Router's known v3-only contract lexical gate so this
        // assertion covers only the generated HTTP gateway/deployment join.
        // The exact fresh artifact remains v4 and is rejected below.
        const routerJoinDeployment = structuredClone(generated.deploymentValue);
        recordField(
          routerJoinDeployment,
          'contract'
        ).serviceProtocolIdentity =
          generated.serviceContract.contract.serviceProtocolIdentity.replace(
            'skiff-service-protocol-v4:',
            'skiff-service-protocol-v3:'
          );
        const loaded = joinRuntimeAssemblyDeployments(
          generated.assemblyValue as unknown as DecodedRuntimeAssemblyRecord,
          [routerJoinDeployment]
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
              host: 'websocket-fixture.skiff.localhost',
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

        await expect(
          new FilesystemRuntimeAssemblySnapshotLoader(root).load(
            generated.runtimeAssembly.assembly
          )
        ).rejects.toThrow(
          /RuntimeAssembly\.resolvedContracts\[0\]\.serviceProtocolIdentity is invalid/
        );
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
