import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import {
  deriveCurrentRuntimeAssemblyServiceDeploymentIdentity,
  deriveWebSocketEntryId
} from '../src/router/runtimeAssemblyDeploymentSnapshot.js';

const roots: string[] = [];
const ASSEMBLY_IDENTITY =
  `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const SERVICE_PROTOCOL_IDENTITY =
  `skiff-service-protocol-v5:sha256:${'b'.repeat(64)}`;
const GATEWAY_IDENTITIES = [
  'skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2',
  'skiff-gateway-entry-v2:sha256:00d40bc2d3aa3da1b1056a2317800b19d1c3ccfaddac8c2bec4145e818aad099',
  'skiff-gateway-entry-v2:sha256:fe171230932018f3bc7aaf13de6b7045b3afe65a7df6d7eeaf4cd394584eb6cf'
] as const;
const WEBSOCKET_GATEWAY_IDENTITY =
  'skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d';
const WEBSOCKET_METHOD_IDENTITY =
  'skiff-gateway-entry-v2:sha256:76fd205e35d35474a2082dd58b914b25b653eeecbfd8b6c96c52d3d070eae331';

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('filesystem RuntimeAssembly snapshot loader', () => {
  it('matches the compiler-owned WebSocketEntryId language-neutral golden', () => {
    expect(deriveWebSocketEntryId('example.com/chat', 'websocket')).toBe(
      'skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616'
    );
  });

  it('loads raw unary, raw server stream and typed unary from exact deployments', async () => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    await writeFixture(root, fixture);

    const loaded = await loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY });

    expect(loaded).toMatchObject({
      schemaVersion: 'skiff-runtime-assembly-v3',
      assemblyIdentity: ASSEMBLY_IDENTITY,
      resolvedContracts: [{
        serviceId: 'skiff.run/echo',
        contractVersion: '1.0.0',
        serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
      }, {
        serviceId: 'skiff.run/echo',
        contractVersion: '1.1.0',
        serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
      }, {
        serviceId: 'skiff.run/echo',
        contractVersion: '1.2.0',
        serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
      }],
      gatewayIngress: [
        {
          gatewayEntryKey: 'rawUnary',
          gatewayEntryIdentity: GATEWAY_IDENTITIES[0],
          adapterKind: 'rawHttp',
          operationMode: 'unary'
        },
        {
          gatewayEntryKey: 'rawStream',
          gatewayEntryIdentity: GATEWAY_IDENTITIES[1],
          adapterKind: 'rawHttp',
          operationMode: 'serverStream'
        },
        {
          gatewayEntryKey: 'typedUnary',
          gatewayEntryIdentity: GATEWAY_IDENTITIES[2],
          adapterKind: 'typedJson',
          operationMode: 'unary'
        }
      ]
    });
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('timeoutMs');
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('handler');
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('adapterPlan');
    expect(loaded.gatewayIngress[0]).not.toHaveProperty('contractOperationId');
  });

  it.each([
    {
      name: 'handler present',
      handler: 'pkg-callable:skiff.run/echo:top-level:main.connect'
    },
    {
      name: 'handler absent',
      handler: null
    }
  ])('loads an exact current WebSocket entry with $name', async ({ handler }) => {
    const root = await fixtureRoot();
    const fixture = websocketFixture(handler);
    await writeFixture(root, fixture);

    const loaded = await loader(root).load({
      assemblyIdentity: ASSEMBLY_IDENTITY
    });
    const binding = loaded.gatewayIngress[0]!;
    expect(binding).toMatchObject({
      selector: {
        protocol: 'webSocket',
        method: null,
        path: '/chat'
      },
      gatewayEntryKey: 'websocket',
      gatewayEntryIdentity: WEBSOCKET_GATEWAY_IDENTITY,
      adapterKind: 'websocketConnect',
      operationMode: 'unary',
      websocketEntryId: deriveWebSocketEntryId(
        'skiff.run/echo',
        'websocket'
      )
    });
    if (handler === null) {
      expect(binding).not.toHaveProperty('handler');
    } else {
      expect(binding.handler).toBe(handler);
    }
  });

  it('loads current physical and JSON-RPC method entries into one captured table', async () => {
    const root = await fixtureRoot();
    const fixture = websocketRpcFixture();
    await writeFixture(root, fixture);

    const loaded = await loader(root).load({
      assemblyIdentity: ASSEMBLY_IDENTITY
    });
    const binding = loaded.gatewayIngress[0]!;

    expect(loaded.gatewayIngress).toHaveLength(1);
    expect(Array.from(binding.websocketMethods!.capture())).toEqual([
      [
        'status',
        expect.objectContaining({
          gatewayEntryIdentity: WEBSOCKET_METHOD_IDENTITY,
          handler:
            'pkg-callable:skiff.run/echo:top-level:main.status',
          profile: 'jsonrpc-2.0-text',
          websocketEntryId: binding.websocketEntryId
        })
      ]
    ]);
  });

  it.each([
    {
      name: 'non-null method',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.ingress[0].selector.method = 'GET';
        fixture.assembly.gatewayIngress[0].selector.method = 'GET';
      }
    },
    {
      name: 'non-compiler key',
      mutate: (fixture: Fixture) => {
        const entry = fixture.deployments[0]!.gatewayEntries.websocket;
        fixture.deployments[0]!.gatewayEntries = { chat: entry };
        fixture.deployments[0]!.ingress[0].gatewayEntryKey = 'chat';
        fixture.assembly.gatewayIngress[0].gatewayEntryKey = 'chat';
      }
    },
    {
      name: 'wrong fixed downlink frame order',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.gatewayEntries.websocket
          .protocolSurface.protocol.surface.downlinkFrames = ['text', 'binary'];
      }
    },
    {
      name: 'handler-absent adapter args',
      mutate: (fixture: Fixture) => {
        const entry = fixture.deployments[0]!.gatewayEntries.websocket;
        entry.handler = null;
        entry.adapterPlan.args = [{
          param: 'request',
          source: { kind: 'websocket.connectRequest' }
        }];
      }
    },
    {
      name: 'legacy pre hook',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.gatewayEntries.websocket.pre =
          `skiff-package-callable-v1:sha256:${'9'.repeat(64)}`;
      }
    },
    {
      name: 'HTTP alias for the WebSocket entry',
      mutate: (fixture: Fixture) => {
        const deploymentIngress = {
          selector: {
            protocol: 'http',
            method: 'GET',
            path: '/chat-http'
          },
          gatewayEntryKey: 'websocket'
        };
        fixture.deployments[0]!.ingress.push(deploymentIngress);
        fixture.assembly.gatewayIngress.push({
          ...structuredClone(fixture.assembly.gatewayIngress[0]),
          selector: structuredClone(deploymentIngress.selector)
        });
      }
    }
  ])('rejects an inexact current WebSocket snapshot: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = websocketFixture(
      'pkg-callable:skiff.run/echo:top-level:main.connect'
    );
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });

  it('validates nested current identity preimages while retaining the declared assembly identity', async () => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    fixture.assembly.roots = [...fixture.assembly.roots].reverse();
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).resolves.toMatchObject({ assemblyIdentity: ASSEMBLY_IDENTITY });
  });

  it.each([
    {
      name: 'ServiceDeployment v3 schema',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.schemaVersion =
          'skiff-service-deployment-v3';
      }
    },
    {
      name: 'DeploymentArtifact v2 identity',
      mutate: (fixture: Fixture) => {
        const legacy =
          `skiff-deployment-artifact-v2:sha256:${'4'.repeat(64)}`;
        fixture.deployments[0]!.deploymentArtifactIdentity = legacy;
        fixture.assembly.resolvedDeployments[0].deploymentArtifactIdentity =
          legacy;
      }
    },
    {
      name: 'GatewayEntry v1 identity',
      mutate: (fixture: Fixture) => {
        const legacy =
          `skiff-gateway-entry-v1:sha256:${'1'.repeat(64)}`;
        fixture.deployments[0]!.gatewayEntries.rawUnary.gatewayEntryIdentity =
          legacy;
        fixture.assembly.gatewayIngress[0].gatewayEntryIdentity = legacy;
      }
    }
  ])('rejects legacy current-reader input: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });

  it.each(['configLiterals', 'secretRefs', 'stateBindings'])(
    'rejects the removed ServiceDeployment field %s',
    async (field) => {
      const root = await fixtureRoot();
      const fixture = canonicalFixture();
      fixture.deployments[0]![field] = [];
      await writeFixture(root, fixture);

      await expect(
        loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
      ).rejects.toThrow(
        /RouterSnapshot\.serviceDeployments\[0\] fields must be exactly/
      );
    }
  );

  it('accepts only PackageArtifact v9 records addressed by package build v10', async () => {
    const packageRef: PackageRefFixture = {
      packageId: 'skiff.run/echo',
      packageVersion: '1.0.0',
      packageBuildId: `skiff-package-build-v10:sha256:${'d'.repeat(64)}`,
      packageLocalAbiIdentity:
        `skiff-package-local-abi-v7:sha256:${'e'.repeat(64)}`
    };

    const current = await fixtureRoot();
    const currentFixture = canonicalFixture();
    currentFixture.assembly.packageLinkPlan.codeSlots = [{ package: packageRef }];
    await writeFixture(current, currentFixture);
    await writeJson(current, packagePath(packageRef), {
      schemaVersion: 'skiff-package-artifact-v9',
      files: []
    });
    await expect(
      loader(current).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).resolves.toMatchObject({ assemblyIdentity: ASSEMBLY_IDENTITY });

    const legacySchema = await fixtureRoot();
    await writeFixture(legacySchema, currentFixture);
    await writeJson(legacySchema, packagePath(packageRef), {
      schemaVersion: 'skiff-package-artifact-v8',
      files: []
    });
    await expect(
      loader(legacySchema).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/schemaVersion must be skiff-package-artifact-v9/);

    const legacyBuild = await fixtureRoot();
    const legacyBuildFixture = canonicalFixture();
    legacyBuildFixture.assembly.packageLinkPlan.codeSlots = [{
      package: {
        ...packageRef,
        packageBuildId: `skiff-package-build-v9:sha256:${'d'.repeat(64)}`
      }
    }];
    await writeFixture(legacyBuild, legacyBuildFixture);
    await expect(
      loader(legacyBuild).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/packageBuildId is invalid/);
  });

  it('rejects a v1 RuntimeAssembly identity prefix before artifact lookup', async () => {
    const root = await fixtureRoot();
    await expect(loader(root).load({
      assemblyIdentity:
        `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`
    })).rejects.toThrow(/reference identity is invalid/);
  });

  it('fails closed for missing, malformed, mismatched and escaping records', async () => {
    const missing = await fixtureRoot();
    await expect(
      loader(missing).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/unavailable/);

    const malformed = await fixtureRoot();
    await writeText(
      malformed,
      assemblyPath(),
      `{"assemblyIdentity":${JSON.stringify(ASSEMBLY_IDENTITY)},"assemblyIdentity":"duplicate"}`
    );
    await expect(
      loader(malformed).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/strict JSON/);

    const mismatched = await fixtureRoot();
    const fixture = canonicalFixture();
    await writeFixture(mismatched, fixture);
    fixture.deployments[0]!.deploymentArtifactIdentity =
      `skiff-deployment-artifact-v4:sha256:${'f'.repeat(64)}`;
    await writeJson(
      mismatched,
      deploymentPath(deploymentRef(fixture.assembly, 0)),
      fixture.deployments[0]!
    );
    await expect(
      loader(mismatched).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/exact ServiceDeployment reference/);

    const escaping = await fixtureRoot();
    const outside = await fixtureRoot();
    const outsideAssembly = join(outside, 'assembly.json');
    await writeFile(outsideAssembly, JSON.stringify(canonicalFixture().assembly));
    const target = join(escaping, assemblyPath());
    await mkdir(dirname(target), { recursive: true });
    await symlink(outsideAssembly, target);
    await expect(
      loader(escaping).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/escapes artifactsPath/);
  });

  it.each([
    {
      name: 'v1 schema',
      mutate: (fixture: Fixture) => {
        fixture.assembly.schemaVersion = 'skiff-runtime-assembly-v1';
      }
    },
    {
      name: 'globalIngress',
      mutate: (fixture: Fixture) => {
        fixture.assembly.globalIngress = fixture.assembly.gatewayIngress;
        delete fixture.assembly.gatewayIngress;
      }
    },
    {
      name: 'contract operation',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].contractOperationId =
          `skiff-contract-operation-v1:sha256:${'c'.repeat(64)}`;
      }
    }
  ])('rejects legacy RuntimeAssembly surface: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });

  it.each([
    {
      name: 'wrong key',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].gatewayEntryKey = 'other';
      }
    },
    {
      name: 'wrong identity',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[0].gatewayEntryIdentity =
          GATEWAY_IDENTITIES[2];
      }
    },
    {
      name: 'missing assembly selector',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress.pop();
      }
    },
    {
      name: 'extra assembly selector',
      mutate: (fixture: Fixture) => {
        const extra = structuredClone(fixture.assembly.gatewayIngress[0]);
        extra.selector.path = '/extra';
        fixture.assembly.gatewayIngress.push(extra);
      }
    },
    {
      name: 'duplicate assembly selector',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress.push(
          structuredClone(fixture.assembly.gatewayIngress[0])
        );
      }
    },
    {
      name: 'missing deployment key',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.ingress[0].gatewayEntryKey = 'missing';
      }
    },
    {
      name: 'duplicate deployment selector',
      mutate: (fixture: Fixture) => {
        fixture.deployments[0]!.ingress.push(
          structuredClone(fixture.deployments[0]!.ingress[0])
        );
      }
    }
  ])('rejects an inexact assembly/deployment join: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });

  it.each([
    {
      name: 'contract service',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.contract.serviceId = 'skiff.run/other';
      }
    },
    {
      name: 'contract version',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.contract.contractVersion = '2.0.0';
      }
    },
    {
      name: 'contract protocol identity',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.contract.serviceProtocolIdentity =
          `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`;
      }
    },
    {
      name: 'revision',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.deploymentRevision = 'other-revision';
      }
    },
    {
      name: 'deployment identity',
      mutate: (deploymentRecord: Record<string, any>) => {
        deploymentRecord.deploymentArtifactIdentity =
          `skiff-deployment-artifact-v4:sha256:${'f'.repeat(64)}`;
      }
    }
  ])('rejects a record with mismatched exact reference field: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    const reference = deploymentRef(fixture.assembly, 0);
    mutate(fixture.deployments[0]!);
    await writeFixture(root, fixture);
    await writeJson(
      root,
      deploymentPath(reference),
      fixture.deployments[0]!
    );

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow(/exact ServiceDeployment reference/);
  });

  it.each([
    {
      name: 'typed JSON server stream',
      mutate: (fixture: Fixture) => {
        const surface = httpSurface(fixture.deployments[2]!, 'typedUnary');
        surface.dispatchMode = 'serverStream';
        surface.responseSchema = null;
        surface.streamItemSchema = { kind: 'string' };
      }
    },
    {
      name: 'non-HTTP protocol',
      mutate: (fixture: Fixture) => {
        protocol(fixture.deployments[0]!, 'rawUnary').kind = 'webSocket';
      }
    },
    {
      name: 'adapter kind mismatch',
      mutate: (fixture: Fixture) => {
        gatewayEntry(fixture.deployments[0]!, 'rawUnary').adapterPlan.kind =
          'typedJson';
      }
    },
    {
      name: 'unary stream schema',
      mutate: (fixture: Fixture) => {
        httpSurface(fixture.deployments[0]!, 'rawUnary').streamItemSchema = {
          kind: 'string'
        };
      }
    },
    {
      name: 'retired deployment policy',
      mutate: (fixture: Fixture) => {
        fixture.deployments[1]!.policy = {
          resources: { cpuMillis: 100, memoryBytes: 1_048_576 },
          principal: 'service:skiff.run/echo'
        };
      }
    }
  ])('rejects invalid deployment mode or retired policy: $name', async ({ mutate }) => {
    const root = await fixtureRoot();
    const fixture = canonicalFixture();
    mutate(fixture);
    await writeFixture(root, fixture);

    await expect(
      loader(root).load({ assemblyIdentity: ASSEMBLY_IDENTITY })
    ).rejects.toThrow();
  });
});

interface Fixture {
  assembly: Record<string, any>;
  deployments: Array<Record<string, any>>;
}

function canonicalFixture(): Fixture {
  const deployments = [
    deployment('raw-unary', 'rawUnary', GATEWAY_IDENTITIES[0], {
      adapterKind: 'rawHttp',
      operationMode: 'unary',
      path: '/raw'
    }),
    deployment('raw-stream', 'rawStream', GATEWAY_IDENTITIES[1], {
      adapterKind: 'rawHttp',
      operationMode: 'serverStream',
      path: '/stream'
    }),
    deployment('typed-unary', 'typedUnary', GATEWAY_IDENTITIES[2], {
      adapterKind: 'typedJson',
      operationMode: 'unary',
      path: '/typed'
    })
  ];
  const references = deployments.map((value) => ({
    serviceId: value.contract.serviceId,
    contractVersion: value.contract.contractVersion,
    deploymentRevision: value.deploymentRevision,
    deploymentArtifactIdentity: value.deploymentArtifactIdentity
  }));
  return {
    assembly: {
      schemaVersion: 'skiff-runtime-assembly-v3',
      assemblyIdentity: ASSEMBLY_IDENTITY,
      roots: references,
      resolvedDeployments: references,
      resolvedContracts: deployments.map((value) => ({
        serviceId: value.contract.serviceId,
        contractVersion: value.contract.contractVersion,
        serviceProtocolIdentity: value.contract.serviceProtocolIdentity
      })),
      resolvedPackages: [],
      packageLinkPlan: { codeSlots: [], packageLinks: [] },
      serviceBindingTemplates: [],
      activationTemplates: [],
      gatewayIngress: deployments.map((value, index) => ({
        selector: structuredClone(value.ingress[0].selector),
        deployment: references[index],
        gatewayEntryKey: value.ingress[0].gatewayEntryKey,
        gatewayEntryIdentity:
          value.gatewayEntries[value.ingress[0].gatewayEntryKey].gatewayEntryIdentity
      }))
    },
    deployments
  };
}

function websocketFixture(handler: string | null): Fixture {
  const fixture = canonicalFixture();
  const deployment = fixture.deployments[0]!;
  const entry = deployment.gatewayEntries.rawUnary;
  entry.gatewayEntryIdentity = WEBSOCKET_GATEWAY_IDENTITY;
  entry.protocolSurface = {
    protocol: {
      kind: 'websocketConnect',
      surface: {
        connectRequestShape: 'v1',
        connectResultShape: 'v1',
        connectionPolicyShape: 'v1',
        externalSources: [
          { kind: 'websocket.connectRequest' },
          { kind: 'websocket.connectionId' }
        ],
        downlinkFrames: ['binary', 'text'],
        rpcProfiles: ['jsonrpc-2.0-text']
      }
    },
    externalErrorProjection: { kind: 'fixed', version: 'v1' }
  };
  entry.handler = handler;
  entry.pre = null;
  entry.guard = null;
  entry.adapterPlan = {
    kind: 'websocketConnect',
    args:
      handler === null
        ? []
        : [
            {
              param: 'request',
              source: { kind: 'websocket.connectRequest' }
            },
            {
              param: 'connectionId',
              source: { kind: 'websocket.connectionId' }
            }
          ]
  };
  deployment.gatewayEntries = { websocket: entry };
  deployment.ingress = [{
    selector: {
      protocol: 'webSocket',
      method: null,
      path: '/chat'
    },
    gatewayEntryKey: 'websocket'
  }];
  deployment.deploymentArtifactIdentity =
    deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(deployment);
  fixture.deployments = [deployment];
  const reference = {
    serviceId: deployment.contract.serviceId,
    contractVersion: deployment.contract.contractVersion,
    deploymentRevision: deployment.deploymentRevision,
    deploymentArtifactIdentity: deployment.deploymentArtifactIdentity
  };
  fixture.assembly.roots = [reference];
  fixture.assembly.resolvedDeployments = [reference];
  fixture.assembly.gatewayIngress = [{
    selector: structuredClone(deployment.ingress[0].selector),
    deployment: reference,
    gatewayEntryKey: 'websocket',
    gatewayEntryIdentity: entry.gatewayEntryIdentity
  }];
  return fixture;
}

function websocketRpcFixture(): Fixture {
  const fixture = websocketFixture(null);
  const deployment = fixture.deployments[0]!;
  const selector = {
    protocol: 'webSocket',
    method: 'status',
    path: '/chat'
  };
  deployment.gatewayEntries.status = {
    gatewayEntryIdentity: WEBSOCKET_METHOD_IDENTITY,
    protocolSurface: {
      protocol: {
        kind: 'websocketJsonRpc',
        surface: {
          profile: 'jsonrpc-2.0-text',
          dispatchMode: 'unary',
          externalSources: [
            { kind: 'websocket.connectionId' },
            { kind: 'websocket.jsonRpcParams' }
          ],
          paramsSchema: {
            kind: 'record',
            fields: { id: { kind: 'string' } },
            required: ['id']
          },
          resultSchema: {
            kind: 'record',
            fields: { value: { kind: 'string' } },
            required: ['value']
          }
        }
      },
      externalErrorProjection: { kind: 'fixed', version: 'v1' }
    },
    handler: 'pkg-callable:skiff.run/echo:top-level:main.status',
    pre: null,
    guard: null,
    adapterPlan: {
      kind: 'websocketJsonRpc',
      args: [
        {
          param: 'connectionId',
          source: { kind: 'websocket.connectionId' }
        },
        {
          param: 'params',
          source: { kind: 'websocket.jsonRpcParams' }
        }
      ]
    }
  };
  deployment.ingress.push({
    selector,
    gatewayEntryKey: 'status'
  });
  fixture.assembly.gatewayIngress.push({
    selector: structuredClone(selector),
    deployment: fixture.assembly.resolvedDeployments[0],
    gatewayEntryKey: 'status',
    gatewayEntryIdentity: WEBSOCKET_METHOD_IDENTITY
  });
  refreshFixtureDeploymentIdentity(fixture, 0);
  return fixture;
}

function deployment(
  revision: string,
  gatewayEntryKey: string,
  gatewayEntryIdentity: string,
  options: {
    adapterKind: 'rawHttp' | 'typedJson';
    operationMode: 'unary' | 'serverStream';
    path: string;
  }
): Record<string, any> {
  const typed = options.adapterKind === 'typedJson';
  const stream = options.operationMode === 'serverStream';
  const record = {
    schemaVersion: 'skiff-service-deployment-v4',
    contract: {
      serviceId: 'skiff.run/echo',
      contractVersion:
        gatewayEntryKey === 'rawUnary'
          ? '1.0.0'
          : gatewayEntryKey === 'rawStream'
            ? '1.1.0'
            : '1.2.0',
      serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY
    },
    deploymentRevision: revision,
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v4:sha256:${(
        gatewayEntryKey === 'rawUnary'
          ? '4'
          : gatewayEntryKey === 'rawStream'
            ? '5'
            : '6'
      ).repeat(64)}`,
    implementation: {
      packageId: 'skiff.run/echo',
      packageVersion: '1.0.0',
      packageBuildId: `skiff-package-build-v10:sha256:${'d'.repeat(64)}`,
      packageLocalAbiIdentity:
        `skiff-package-local-abi-v7:sha256:${'e'.repeat(64)}`
    },
    operationBindings: [],
    packageBindings: [],
    serviceSelectors: [],
    gatewayEntries: {
      [gatewayEntryKey]: {
        gatewayEntryIdentity,
        protocolSurface: {
          protocol: {
            kind: 'http',
            surface: {
              adapterKind: options.adapterKind,
              dispatchMode: options.operationMode,
              externalSources: [{
                kind: typed ? 'http.body' : 'http.request'
              }],
              requestBodySchema: typed ? { kind: 'string' } : null,
              responseSchema: typed ? { kind: 'string' } : null,
              streamItemSchema: stream ? { kind: 'string' } : null
            }
          },
          externalErrorProjection: { kind: 'fixed', version: 'v1' }
        },
        handler: `skiff-package-callable-v1:sha256:${'f'.repeat(64)}`,
        pre: null,
        guard: null,
        adapterPlan: {
          kind: options.adapterKind,
          args: [{
            param: typed ? 'body' : 'request',
            source: { kind: typed ? 'http.body' : 'http.request' }
          }]
        }
      }
    },
    ingress: [{
      selector: {
        protocol: 'http',
        method: 'POST',
        path: options.path
      },
      gatewayEntryKey
    }],
    resourceBindings: [],
    runtimeCapabilityBindings: [],
    diagnosticText: { displayName: gatewayEntryKey, notes: {} }
  };
  record.deploymentArtifactIdentity =
    deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(record);
  return record;
}

function gatewayEntry(
  deploymentRecord: Record<string, any>,
  key: string
): Record<string, any> {
  return deploymentRecord.gatewayEntries[key];
}

function protocol(
  deploymentRecord: Record<string, any>,
  key: string
): Record<string, any> {
  return gatewayEntry(deploymentRecord, key).protocolSurface.protocol;
}

function httpSurface(
  deploymentRecord: Record<string, any>,
  key: string
): Record<string, any> {
  return protocol(deploymentRecord, key).surface;
}

function loader(root: string): FilesystemRuntimeAssemblySnapshotLoader {
  return new FilesystemRuntimeAssemblySnapshotLoader(root);
}

async function fixtureRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'skiff-router-snapshot-'));
  roots.push(root);
  return root;
}

async function writeFixture(root: string, fixture: Fixture): Promise<void> {
  await writeJson(root, assemblyPath(), fixture.assembly);
  for (const [index, deploymentRecord] of fixture.deployments.entries()) {
    await writeJson(
      root,
      deploymentPath(deploymentRef(fixture.assembly, index)),
      deploymentRecord
    );
  }
}

function deploymentRef(
  assembly: Record<string, any>,
  index: number
): DeploymentRefFixture {
  return assembly.resolvedDeployments[index] as DeploymentRefFixture;
}

function refreshFixtureDeploymentIdentity(
  fixture: Fixture,
  index: number
): void {
  const deploymentRecord = fixture.deployments[index]!;
  const identity =
    deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(deploymentRecord);
  deploymentRecord.deploymentArtifactIdentity = identity;
  fixture.assembly.resolvedDeployments[index].deploymentArtifactIdentity =
    identity;
}

function assemblyPath(): string {
  return `records/runtime-assemblies/${identityHash(ASSEMBLY_IDENTITY)}.json`;
}

function deploymentPath(reference: DeploymentRefFixture): string {
  return [
    'records/service-deployments',
    reference.serviceId.replaceAll('.', '~d').replaceAll('/', '~s'),
    reference.contractVersion,
    reference.deploymentRevision,
    `${identityHash(reference.deploymentArtifactIdentity)}.json`
  ].join('/');
}

interface DeploymentRefFixture {
  serviceId: string;
  contractVersion: string;
  deploymentRevision: string;
  deploymentArtifactIdentity: string;
}

interface PackageRefFixture {
  packageId: string;
  packageVersion: string;
  packageBuildId: string;
  packageLocalAbiIdentity: string;
}

function packagePath(reference: PackageRefFixture): string {
  return [
    'records/package-artifacts',
    reference.packageId.replaceAll('.', '~d').replaceAll('/', '~s'),
    reference.packageVersion,
    identityHash(reference.packageBuildId),
    'package.json'
  ].join('/');
}

async function writeJson(root: string, path: string, value: unknown): Promise<void> {
  await writeText(root, path, JSON.stringify(value));
}

async function writeText(root: string, path: string, value: string): Promise<void> {
  const target = join(root, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, value);
}

function identityHash(value: string): string {
  return value.slice(value.lastIndexOf(':') + 1);
}
