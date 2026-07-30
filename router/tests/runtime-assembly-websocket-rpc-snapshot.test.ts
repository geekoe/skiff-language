import { describe, expect, it } from 'vitest';

import {
  deriveCurrentRuntimeAssemblyServiceDeploymentIdentity,
  joinRuntimeAssemblyDeployments
} from '../src/router/runtimeAssemblyDeploymentSnapshot.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type DecodedRuntimeAssemblyRecord
} from '../src/router/runtimeAssemblySnapshot.js';
import { deriveWebSocketEntryId } from '../src/router/runtimeAssemblyWebSocketSnapshot.js';

const DEPLOYMENT_IDENTITY =
  `skiff-deployment-artifact-v4:sha256:${'a'.repeat(64)}`;
const PHYSICAL_IDENTITY =
  'skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d';
const METHOD_IDENTITY =
  'skiff-gateway-entry-v2:sha256:76fd205e35d35474a2082dd58b914b25b653eeecbfd8b6c96c52d3d070eae331';

describe('RuntimeAssembly WebSocket RPC snapshot', () => {
  it('joins a current physical entry and method-bearing JSON-RPC entry', () => {
    const fixture = currentWebSocketFixture();

    const loaded = joinRuntimeAssemblyDeployments(
      fixture.assembly as unknown as DecodedRuntimeAssemblyRecord,
      [fixture.deployment]
    );

    expect(loaded.gatewayIngress).toHaveLength(1);
    expect(loaded.gatewayIngress[0]).toMatchObject({
      gatewayEntryIdentity: PHYSICAL_IDENTITY,
      adapterKind: 'websocketConnect',
      handler: 'pkg-callable:example.com/chat_implementation:top-level:main.connect'
    });
    const binding = loaded.gatewayIngress[0]!;
    expect(binding.websocketEntryId).toBe(
      deriveWebSocketEntryId('example.com/chat', 'websocket')
    );
    expect(binding.websocketRpcProfiles).toEqual(['jsonrpc-2.0-text']);
    expect(Array.from(binding.websocketMethods!.capture())).toEqual([
      [
        'status',
        expect.objectContaining({
          method: 'status',
          profile: 'jsonrpc-2.0-text',
          handler:
            'pkg-callable:example.com/chat_implementation:top-level:main.status',
          gatewayEntryIdentity: METHOD_IDENTITY,
          websocketEntryId: binding.websocketEntryId
        })
      ]
    ]);
  });

  it('keeps methods when the physical connect entry has no handler', () => {
    const fixture = currentWebSocketFixture();
    fixture.deployment.gatewayEntries.websocket.handler = null;
    fixture.deployment.gatewayEntries.websocket.adapterPlan.args = [];
    refreshDeploymentIdentity(fixture);

    const binding = joinFixture(fixture).gatewayIngress[0]!;

    expect(binding).not.toHaveProperty('handler');
    expect(binding.websocketMethods?.size).toBe(1);
    expect(binding.websocketMethods?.capture().get('status')?.handler).toBe(
      'pkg-callable:example.com/chat_implementation:top-level:main.status'
    );
  });

  it('keeps a pure path-only physical entry with an empty method table', () => {
    const fixture = currentWebSocketFixture();
    removeMethod(fixture, 'status');

    const loaded = joinFixture(fixture);

    expect(loaded.gatewayIngress).toHaveLength(1);
    expect(loaded.gatewayIngress[0]?.websocketMethods?.size).toBe(0);
  });

  it('captures multiple methods under the exact physical profile', () => {
    const fixture = currentWebSocketFixture();
    addMethod(fixture, 'acknowledge', 'acknowledge');

    const binding = joinFixture(fixture).gatewayIngress[0]!;
    const methods = binding.websocketMethods!.capture();

    expect(Array.from(methods.keys())).toEqual(['status', 'acknowledge']);
    expect(
      Array.from(methods.values()).map(({ profile }) => profile)
    ).toEqual(['jsonrpc-2.0-text', 'jsonrpc-2.0-text']);
    expect(
      Array.from(methods.values()).every(
        ({ websocketEntryId }) =>
          websocketEntryId === binding.websocketEntryId
      )
    ).toBe(true);
  });

  it.each([
    {
      name: 'duplicate method',
      mutate: (fixture: Fixture) => {
        addMethod(fixture, 'duplicateStatus', 'status');
      }
    },
    {
      name: 'orphan method without physical entry',
      mutate: (fixture: Fixture) => {
        delete fixture.deployment.gatewayEntries.websocket;
        fixture.deployment.ingress.splice(0, 1);
        fixture.assembly.gatewayIngress.splice(0, 1);
      }
    },
    {
      name: 'method on another path',
      mutate: (fixture: Fixture) => {
        fixture.deployment.ingress[1].selector.path = '/other';
        fixture.assembly.gatewayIngress[1].selector.path = '/other';
      }
    },
    {
      name: 'cross-deployment declaration owner',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[1].deployment = {
          ...fixture.assembly.gatewayIngress[1].deployment,
          serviceId: 'example.com/other'
        };
      }
    },
    {
      name: 'foreign-package method handler',
      mutate: (fixture: Fixture) => {
        fixture.deployment.gatewayEntries.status.handler =
          'pkg-callable:example.com/other:top-level:main.status';
      }
    },
    {
      name: 'foreign-package physical handler',
      mutate: (fixture: Fixture) => {
        fixture.deployment.gatewayEntries.websocket.handler =
          'pkg-callable:example.com/other:top-level:main.connect';
      }
    },
    {
      name: 'unsupported physical profile',
      mutate: (fixture: Fixture) => {
        fixture.deployment.gatewayEntries.websocket.protocolSurface.protocol.surface.rpcProfiles =
          ['future-profile'];
      }
    },
    {
      name: 'missing method handler',
      mutate: (fixture: Fixture) => {
        fixture.deployment.gatewayEntries.status.handler = null;
      }
    },
    {
      name: 'wrong method adapter',
      mutate: (fixture: Fixture) => {
        fixture.deployment.gatewayEntries.status.adapterPlan.kind =
          'websocketConnect';
      }
    },
    {
      name: 'ambiguous physical selectors',
      mutate: (fixture: Fixture) => {
        const ingress = structuredClone(fixture.deployment.ingress[0]);
        ingress.selector.path = '/other';
        fixture.deployment.ingress.push(ingress);
        const declaration = structuredClone(fixture.assembly.gatewayIngress[0]);
        declaration.selector.path = '/other';
        fixture.assembly.gatewayIngress.push(declaration);
      }
    },
    {
      name: 'method identity drift',
      mutate: (fixture: Fixture) => {
        fixture.assembly.gatewayIngress[1].gatewayEntryIdentity =
          `skiff-gateway-entry-v2:sha256:${'9'.repeat(64)}`;
      }
    },
    {
      name: 'method surface drift behind its declared identity',
      mutate: (fixture: Fixture) => {
        fixture.deployment.gatewayEntries.status.protocolSurface.protocol.surface.resultSchema =
          { kind: 'null' };
      }
    }
  ])('fails closed for $name', ({ mutate }) => {
    const fixture = currentWebSocketFixture();
    mutate(fixture);
    refreshDeploymentIdentity(fixture);
    expect(() => joinFixture(fixture)).toThrow();
  });

  it.each([
    {
      name: 'ServiceDeployment v3',
      mutate: (fixture: Fixture) => {
        fixture.deployment.schemaVersion = 'skiff-service-deployment-v3';
      }
    },
    {
      name: 'DeploymentArtifact v2',
      mutate: (fixture: Fixture) => {
        const legacy =
          `skiff-deployment-artifact-v2:sha256:${'a'.repeat(64)}`;
        fixture.deployment.deploymentArtifactIdentity = legacy;
        fixture.assembly.resolvedDeployments[0].deploymentArtifactIdentity =
          legacy;
      }
    },
    {
      name: 'GatewayEntry v1',
      mutate: (fixture: Fixture) => {
        const legacy =
          `skiff-gateway-entry-v1:sha256:${PHYSICAL_IDENTITY.slice(PHYSICAL_IDENTITY.lastIndexOf(':') + 1)}`;
        fixture.deployment.gatewayEntries.websocket.gatewayEntryIdentity =
          legacy;
        fixture.assembly.gatewayIngress[0].gatewayEntryIdentity = legacy;
      }
    }
  ])('strictly rejects legacy $name records', ({ mutate }) => {
    const fixture = currentWebSocketFixture();
    mutate(fixture);
    expect(() => joinFixture(fixture)).toThrow();
  });

  it('strictly rejects the removed deployment activation policy', () => {
    const fixture = currentWebSocketFixture();
    fixture.deployment.policy.activation = { idleTimeoutMs: null };

    expect(() => joinFixture(fixture)).toThrow(
      /policy fields must contain principal,resources and only optional timeoutMs/
    );
  });

  it.each(['configLiterals', 'secretRefs', 'stateBindings'])(
    'strictly rejects the removed ServiceDeployment field %s',
    (field) => {
      const fixture = currentWebSocketFixture();
      fixture.deployment[field] = [];

      expect(() => joinFixture(fixture)).toThrow(
        /RouterSnapshot\.serviceDeployments\[0\] fields must be exactly/
      );
    }
  );

  it('rejects DeploymentArtifact preimage drift under a current v3 prefix', () => {
    const fixture = currentWebSocketFixture();
    fixture.deployment.policy.principal = 'service:example.com/other';

    expect(() => joinFixture(fixture)).toThrow(/current preimage/);
  });

  it('returns copy-on-capture method maps that survive active snapshot replacement', () => {
    const firstFixture = currentWebSocketFixture();
    const firstLoaded = joinFixture(firstFixture);
    const firstBinding = firstLoaded.gatewayIngress[0]!;
    const captured = firstBinding.websocketMethods!.capture();
    const store = new RouterActiveAssemblySnapshotStore();
    store.replace({
      environment: 'test',
      generation: 1,
      assembly: {
        assemblyIdentity: firstFixture.assembly.assemblyIdentity
      },
      ingress: new RuntimeAssemblyIngressIndex(firstLoaded.gatewayIngress)
    });

    const secondFixture = currentWebSocketFixture();
    renameMethod(secondFixture, 'status', 'acknowledge');
    const secondLoaded = joinFixture(secondFixture);
    store.replace({
      environment: 'test',
      generation: 2,
      assembly: {
        assemblyIdentity: secondFixture.assembly.assemblyIdentity
      },
      ingress: new RuntimeAssemblyIngressIndex(secondLoaded.gatewayIngress)
    });

    (captured as Map<string, unknown>).set('caller-local', {});
    const currentBinding = store.get().ingress.values()[0]!;
    expect(Array.from(captured.keys())).toEqual(['status', 'caller-local']);
    expect(
      Array.from(firstBinding.websocketMethods!.capture().keys())
    ).toEqual(['status']);
    expect(
      Array.from(currentBinding.websocketMethods!.capture().keys())
    ).toEqual(['acknowledge']);
  });
});

interface Fixture {
  assembly: Record<string, any>;
  deployment: Record<string, any>;
}

function currentWebSocketFixture(): Fixture {
  const deploymentRef = {
    serviceId: 'example.com/chat',
    contractVersion: '1.0.0',
    deploymentRevision: 'chat-current',
    deploymentArtifactIdentity: DEPLOYMENT_IDENTITY
  };
  const physicalSelector = {
    protocol: 'webSocket',
    method: null,
    path: '/chat'
  };
  const methodSelector = {
    protocol: 'webSocket',
    method: 'status',
    path: '/chat'
  };
  const physicalEntry = {
    gatewayEntryIdentity: PHYSICAL_IDENTITY,
    protocolSurface: {
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
    },
    handler: 'pkg-callable:example.com/chat_implementation:top-level:main.connect',
    pre: null,
    guard: null,
    adapterPlan: {
      kind: 'websocketConnect',
      args: [
        {
          param: 'request',
          source: { kind: 'websocket.connectRequest' }
        },
        {
          param: 'connectionId',
          source: { kind: 'websocket.connectionId' }
        }
      ]
    }
  };
  const methodEntry = {
    gatewayEntryIdentity: METHOD_IDENTITY,
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
    handler: 'pkg-callable:example.com/chat_implementation:top-level:main.status',
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
  const deployment = {
    schemaVersion: 'skiff-service-deployment-v4',
    contract: {
      serviceId: deploymentRef.serviceId,
      contractVersion: deploymentRef.contractVersion,
      serviceProtocolIdentity:
        `skiff-service-protocol-v5:sha256:${'b'.repeat(64)}`
    },
    deploymentRevision: deploymentRef.deploymentRevision,
    deploymentArtifactIdentity: DEPLOYMENT_IDENTITY,
    implementation: {
      packageId: 'example.com/chat_implementation',
      packageVersion: '1.0.0',
      packageBuildId: `skiff-package-build-v10:sha256:${'c'.repeat(64)}`,
      packageLocalAbiIdentity:
        `skiff-package-local-abi-v7:sha256:${'d'.repeat(64)}`
    },
    operationBindings: [],
    packageBindings: [],
    serviceSelectors: [],
    gatewayEntries: {
      websocket: physicalEntry,
      status: methodEntry
    },
    ingress: [
      { selector: physicalSelector, gatewayEntryKey: 'websocket' },
      { selector: methodSelector, gatewayEntryKey: 'status' }
    ],
    resourceBindings: [],
    runtimeCapabilityBindings: [],
    policy: {
      timeoutMs: 5_000,
      resources: { cpuMillis: 100, memoryBytes: 1_048_576 },
      principal: 'service:example.com/chat'
    },
    diagnosticText: { displayName: 'chat-current', notes: {} }
  };
  const currentDeploymentIdentity =
    deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(deployment);
  deployment.deploymentArtifactIdentity = currentDeploymentIdentity;
  deploymentRef.deploymentArtifactIdentity = currentDeploymentIdentity;
  return {
    assembly: {
      schemaVersion: 'skiff-runtime-assembly-v3',
      assemblyIdentity:
        `skiff-runtime-assembly-v3:sha256:${'e'.repeat(64)}`,
      resolvedDeployments: [deploymentRef],
      resolvedContracts: [{
        serviceId: deploymentRef.serviceId,
        contractVersion: deploymentRef.contractVersion,
        serviceProtocolIdentity:
          `skiff-service-protocol-v5:sha256:${'b'.repeat(64)}`
      }],
      gatewayIngress: [
        {
          selector: physicalSelector,
          deployment: deploymentRef,
          gatewayEntryKey: 'websocket',
          gatewayEntryIdentity: PHYSICAL_IDENTITY
        },
        {
          selector: methodSelector,
          deployment: deploymentRef,
          gatewayEntryKey: 'status',
          gatewayEntryIdentity: METHOD_IDENTITY
        }
      ]
    },
    deployment
  };
}

function joinFixture(fixture: Fixture) {
  return joinRuntimeAssemblyDeployments(
    fixture.assembly as unknown as DecodedRuntimeAssemblyRecord,
    [fixture.deployment]
  );
}

function addMethod(
  fixture: Fixture,
  gatewayEntryKey: string,
  externalMethod: string
): void {
  const entry = structuredClone(fixture.deployment.gatewayEntries.status);
  entry.handler =
    `pkg-callable:example.com/chat_implementation:top-level:main.${gatewayEntryKey}`;
  fixture.deployment.gatewayEntries[gatewayEntryKey] = entry;
  const selector = {
    protocol: 'webSocket',
    method: externalMethod,
    path: '/chat'
  };
  fixture.deployment.ingress.push({
    selector,
    gatewayEntryKey
  });
  fixture.assembly.gatewayIngress.push({
    selector,
    deployment: fixture.assembly.resolvedDeployments[0],
    gatewayEntryKey,
    gatewayEntryIdentity: METHOD_IDENTITY
  });
  refreshDeploymentIdentity(fixture);
}

function removeMethod(fixture: Fixture, gatewayEntryKey: string): void {
  delete fixture.deployment.gatewayEntries[gatewayEntryKey];
  fixture.deployment.ingress = fixture.deployment.ingress.filter(
    (binding: Record<string, any>) =>
      binding.gatewayEntryKey !== gatewayEntryKey
  );
  fixture.assembly.gatewayIngress = fixture.assembly.gatewayIngress.filter(
    (binding: Record<string, any>) =>
      binding.gatewayEntryKey !== gatewayEntryKey
  );
  refreshDeploymentIdentity(fixture);
}

function renameMethod(
  fixture: Fixture,
  gatewayEntryKey: string,
  nextMethod: string
): void {
  const deploymentBinding = fixture.deployment.ingress.find(
    (binding: Record<string, any>) =>
      binding.gatewayEntryKey === gatewayEntryKey
  );
  const assemblyBinding = fixture.assembly.gatewayIngress.find(
    (binding: Record<string, any>) =>
      binding.gatewayEntryKey === gatewayEntryKey
  );
  if (deploymentBinding === undefined || assemblyBinding === undefined) {
    throw new Error(`missing method fixture ${gatewayEntryKey}`);
  }
  deploymentBinding.selector.method = nextMethod;
  assemblyBinding.selector.method = nextMethod;
  refreshDeploymentIdentity(fixture);
}

function refreshDeploymentIdentity(fixture: Fixture): void {
  const identity =
    deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(fixture.deployment);
  fixture.deployment.deploymentArtifactIdentity = identity;
  fixture.assembly.resolvedDeployments[0].deploymentArtifactIdentity =
    identity;
}
