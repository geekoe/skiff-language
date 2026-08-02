import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RouterToRuntimeFrameHeader,
  type SpawnSubmitRequestFrameHeader,
} from '../src/protocol/envelope.js';
import type {
  RuntimeAssemblyRequestStartFrameHeader,
  RuntimeAssemblyRequestStartFrameWireHeader,
} from '../src/protocol/runtimeAssemblyRequest.js';
import {
  type ActiveActorInvocationParent,
  type ActorMethodSpawnControl,
  RuntimeDispatcher,
  type RuntimeFrameSender,
  type RuntimeSelfIngressTestCorrelation,
  type RuntimeSpawnParentAuthority,
} from '../src/router/runtimeDispatcher.js';
import type {
  RuntimeDispatchConnection,
  RuntimeDispatchRuntimeIdentity,
} from '../src/router/runtimeRegistry.js';

const RUNTIME_ID = 'runtime-self-ingress-a';
const SERVICE_ID = 'example.com/self-ingress';
const OTHER_SERVICE_ID = 'example.com/other-service';
const CAPABILITY = 'test-case:self_ingress.actor-parent';
const BUILD_ID = identity('skiff-service-build-v1:sha256', 'b');
const SERVICE_PROTOCOL_IDENTITY = identity(
  'skiff-service-protocol-v5:sha256',
  'c'
);
const ASSEMBLY_IDENTITY = identity('skiff-runtime-assembly-v3:sha256', 'd');
const DEPLOYMENT_ARTIFACT_IDENTITY = identity(
  'skiff-deployment-artifact-v4:sha256',
  'e'
);
const GATEWAY_ENTRY_IDENTITY = identity(
  'skiff-gateway-entry-v2:sha256',
  'f'
);

describe('RuntimeDispatcher actor-parent self-ingress capability', () => {
  it('pins a valid actor parent to its exact Runtime connection', async () => {
    const w1 = openSocket('w1');
    const decoy = openSocket('decoy');
    const authority = rootAuthority();
    const parent = actorParent(w1, authority);
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => parent,
      ordinaryConnection: { runtimeId: 'runtime-decoy', ws: decoy },
    });
    const request = testRequest(
      'nested-valid',
      CAPABILITY,
      SERVICE_ID,
      'actor-parent-valid'
    );

    const result = harness.dispatcher.dispatchPinnedTestBinary(
      { header: request, payloadBytes: new Uint8Array([1, 2, 3]) },
      1_000,
      correlation('actor-parent-valid')
    );

    expect(harness.actorParentResolver).toHaveBeenCalledWith({
      invocationId: 'actor-parent-valid',
      testCaseCapability: CAPABILITY,
      serviceId: SERVICE_ID,
    });
    expect(harness.pickDispatchConnection).not.toHaveBeenCalled();
    expect(harness.sent).toHaveLength(1);
    expect(harness.sent[0]).toMatchObject({
      ws: w1,
      header: { requestId: request.requestId, testCaseCapability: CAPABILITY },
      payloadBytes: new Uint8Array([1, 2, 3]),
    });

    harness.dispatcher.resolveRequest(w1, response(request.requestId));
    await expect(result).resolves.toMatchObject({
      header: { requestId: request.requestId },
    });
    expect(harness.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0,
    });
  });

  it.each([
    {
      name: 'wrong capability',
      request: testRequest(
        'nested-wrong-capability',
        'test-case:wrong',
        SERVICE_ID,
        'actor-parent-wrong-capability'
      ),
      correlation: correlation('actor-parent-wrong-capability', {
        testCaseCapability: 'test-case:wrong',
      }),
    },
    {
      name: 'cross-service request',
      request: testRequest(
        'nested-cross-service',
        CAPABILITY,
        OTHER_SERVICE_ID,
        'actor-parent-cross-service'
      ),
      correlation: correlation('actor-parent-cross-service'),
    },
  ])('rejects $name even when the actor resolver returns a parent', async ({
    request,
    correlation: testCorrelation,
  }) => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => actorParent(w1, authority),
    });

    await expect(harness.dispatcher.dispatchPinnedTestBinary(
      { header: request, payloadBytes: new Uint8Array() },
      1_000,
      testCorrelation
    )).rejects.toMatchObject({
      statusCode: 403,
      code: 'TestCaseCapabilityRejected',
    });
    expect(harness.sent).toHaveLength(0);
    expect(harness.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0,
    });
  });

  it('rejects an ID that simultaneously names an active request and actor parent', async () => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const ambiguousId = 'ambiguous-parent-id';
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: ({ invocationId }) =>
        invocationId === ambiguousId ? actorParent(w1, authority) : undefined,
      assemblyTestConnection: runtimeConnection(w1),
    });
    const root = harness.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRequest(ambiguousId, CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      1_000
    );
    expect(harness.sent).toHaveLength(1);

    await expect(harness.dispatcher.dispatchPinnedTestBinary(
      {
        header: testRequest(
          'nested-ambiguous',
          CAPABILITY,
          SERVICE_ID,
          ambiguousId
        ),
        payloadBytes: new Uint8Array(),
      },
      1_000,
      correlation(ambiguousId)
    )).rejects.toMatchObject({
      statusCode: 403,
      code: 'TestCaseCapabilityRejected',
    });
    expect(harness.sent).toHaveLength(1);

    harness.dispatcher.resolveRequest(w1, response(ambiguousId));
    await expect(root).resolves.toMatchObject({
      header: { requestId: ambiguousId },
    });
  });

  it('rejects a direct capability-parent lookup when request and actor IDs collide', async () => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const ambiguousId = 'direct-ambiguous-parent-id';
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => undefined,
      directActorParentResolver: ({ invocationId }) =>
        invocationId === ambiguousId ? actorParent(w1, authority) : undefined,
      assemblyTestConnection: runtimeConnection(w1),
    });
    const root = harness.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRequest(ambiguousId, CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      1_000
    );

    expect(harness.dispatcher.activeTestCaseParent({
      parentRequestId: ambiguousId,
      testCaseCapability: CAPABILITY,
      serviceId: SERVICE_ID,
      serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY,
      ws: w1,
    })).toBeUndefined();

    harness.dispatcher.resolveRequest(w1, response(ambiguousId));
    await root;
  });

  it('fails closed when the Runtime ID reconnects from W1 to W2', async () => {
    const w1 = openSocket('w1');
    const w2 = openSocket('w2');
    const authority = rootAuthority();
    const harness = createHarness({
      connections: [[RUNTIME_ID, w2]],
      actorParentResolver: () => actorParent(w1, authority),
    });

    await expect(harness.dispatcher.dispatchPinnedTestBinary(
      {
        header: testRequest(
          'nested-after-reconnect',
          CAPABILITY,
          SERVICE_ID,
          'actor-parent-before-reconnect'
        ),
        payloadBytes: new Uint8Array(),
      },
      1_000,
      correlation('actor-parent-before-reconnect')
    )).rejects.toMatchObject({
      statusCode: 403,
      code: 'TestCaseCapabilityRejected',
    });
    expect(harness.sent).toHaveLength(0);
  });

  it('fails closed when current HTTP routing advances beyond the parent generation', async () => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => actorParent(w1, authority),
    });

    await expect(harness.dispatcher.dispatchPinnedTestBinary(
      {
        header: testRequest(
          'nested-after-generation-advance',
          CAPABILITY,
          SERVICE_ID,
          'actor-parent-old-generation',
          2
        ),
        payloadBytes: new Uint8Array(),
      },
      1_000,
      correlation('actor-parent-old-generation')
    )).rejects.toMatchObject({
      statusCode: 403,
      code: 'TestCaseCapabilityRejected',
    });
    expect(harness.sent).toHaveLength(0);
  });

  it('rejects a stale actor parent that is no longer active', async () => {
    const w1 = openSocket('w1');
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => undefined,
    });

    await expect(harness.dispatcher.dispatchPinnedTestBinary(
      {
        header: testRequest(
          'nested-stale-parent',
          CAPABILITY,
          SERVICE_ID,
          'finished-actor-parent'
        ),
        payloadBytes: new Uint8Array(),
      },
      1_000,
      correlation('finished-actor-parent')
    )).rejects.toMatchObject({
      statusCode: 403,
      code: 'TestCaseCapabilityRejected',
    });
    expect(harness.sent).toHaveLength(0);
  });
});

describe('RuntimeDispatcher spawn parent collision', () => {
  it('selects the exact request parent when the same id also names an actor invocation', async () => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const sharedId = 'shared-request-actor-id';
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => undefined,
      directActorParentResolver: ({ invocationId }) =>
        invocationId === sharedId ? actorParent(w1, authority) : undefined,
      assemblyTestConnection: runtimeConnection(w1),
    });
    const root = harness.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRequest(sharedId, CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      1_000
    );
    expect(harness.sent).toHaveLength(1);

    const result = await harness.dispatcher.handleSpawnSubmit(
      w1,
      spawnSubmit(sharedId, 'function', 'request'),
      new Uint8Array()
    );
    expect(result.header).toMatchObject({
      type: 'spawn.submit.response',
      status: 'submitted',
    });
    expect(harness.directActorParentResolver).not.toHaveBeenCalled();
    expect(harness.submitSpawn).not.toHaveBeenCalled();
    expect(harness.sent).toHaveLength(2);

    harness.dispatcher.resolveRequest(w1, response(sharedId));
    await root;
  });

  it('selects the exact actor invocation parent when the same id also names a request', async () => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const sharedId = 'shared-request-actor-id';
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => undefined,
      directActorParentResolver: ({ invocationId }) =>
        invocationId === sharedId ? actorParent(w1, authority) : undefined,
      assemblyTestConnection: runtimeConnection(w1),
    });
    const root = harness.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRequest(sharedId, CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      1_000
    );
    expect(harness.sent).toHaveLength(1);

    const result = await harness.dispatcher.handleSpawnSubmit(
      w1,
      spawnSubmit(sharedId, 'actorMethod', 'actorInvocation'),
      new Uint8Array()
    );
    expect(result.header).toMatchObject({
      type: 'spawn.submit.response',
      status: 'submitted',
    });
    expect(harness.directActorParentResolver).toHaveBeenCalledWith({
      invocationId: sharedId,
      ws: w1,
      serviceId: SERVICE_ID,
      serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY,
    });
    expect(harness.submitSpawn).toHaveBeenCalledTimes(1);
    expect(harness.sent).toHaveLength(1);

    harness.dispatcher.resolveRequest(w1, response(sharedId));
    await root;
  });

  it('rejects a function target whose exact actorInvocation parent cannot produce a request parent', async () => {
    const w1 = openSocket('w1');
    const authority = rootAuthority();
    const sharedId = 'shared-request-actor-id';
    const harness = createHarness({
      connections: [[RUNTIME_ID, w1]],
      actorParentResolver: () => undefined,
      directActorParentResolver: ({ invocationId }) =>
        invocationId === sharedId ? actorParent(w1, authority) : undefined,
      assemblyTestConnection: runtimeConnection(w1),
    });
    const root = harness.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testRequest(sharedId, CAPABILITY),
        payloadBytes: new Uint8Array(),
      },
      1_000
    );
    expect(harness.sent).toHaveLength(1);

    const result = await harness.dispatcher.handleSpawnSubmit(
      w1,
      spawnSubmit(sharedId, 'function', 'actorInvocation'),
      new Uint8Array()
    );
    expect(result.header).toMatchObject({
      type: 'spawn.submit.error',
      error: {
        message: 'function spawn requires a runtime assembly request parent',
      },
    });
    expect(harness.submitSpawn).not.toHaveBeenCalled();
    expect(harness.sent).toHaveLength(1);

    harness.dispatcher.resolveRequest(w1, response(sharedId));
    await root;
  });
});

function createHarness(options: {
  connections: ReadonlyArray<readonly [string, WebSocket]>;
  actorParentResolver(
    input: Parameters<ActorMethodSpawnControl['activeTestCaseActorInvocationParent']>[0]
  ): ActiveActorInvocationParent | undefined;
  directActorParentResolver?(
    input: Parameters<ActorMethodSpawnControl['activeActorInvocationParent']>[0]
  ): ActiveActorInvocationParent | undefined;
  ordinaryConnection?: RuntimeDispatchConnection;
  assemblyTestConnection?: RuntimeDispatchConnection;
}) {
  const forward = new Map(options.connections);
  const reverse = new Map<WebSocket, string>(
    options.connections.map(([runtimeId, ws]) => [ws, runtimeId])
  );
  const sent: Array<{
    ws: WebSocket;
    header: RouterToRuntimeFrameHeader | RuntimeAssemblyRequestStartFrameWireHeader;
    payloadBytes: Uint8Array;
  }> = [];
  const actorParentResolver = vi.fn(options.actorParentResolver);
  const directActorParentResolver = vi.fn(
    options.directActorParentResolver ?? (() => undefined)
  );
  const submitSpawn = vi.fn(async () => ({
    spawnId: 'unused',
    requestId: 'unused',
  }));
  const pickDispatchConnection = vi.fn(() => options.ordinaryConnection ?? null);
  const frameSender: RuntimeFrameSender = {
    sendFrame: (ws, header, payloadBytes = new Uint8Array(), callback) => {
      sent.push({ ws, header, payloadBytes });
      callback?.();
    },
  };
  const actorMethodSpawn: ActorMethodSpawnControl = {
    activeActorInvocationParent: directActorParentResolver,
    activeTestCaseActorInvocationParent: actorParentResolver,
    submitSpawn,
  };
  const dispatcher = new RuntimeDispatcher({
    frameSender,
    maxConcurrency: 8,
    actorMethodSpawn,
    registry: {
      setInFlightCounter: () => {},
      pickDispatchConnection,
      pickAssemblyTestDispatchConnection: () =>
        options.assemblyTestConnection ?? null,
      refreshAllRuntimeStates: () => {},
      refreshRuntimeStatesForRequest: () => {},
      runtimeConnection: (runtimeId): RuntimeDispatchRuntimeIdentity | undefined => {
        const ws = forward.get(runtimeId);
        return ws === undefined ? undefined : { runtimeId, ws };
      },
      runtimeCapabilityIdentityForConnection: (ws) => reverse.get(ws),
    },
  });
  return {
    actorParentResolver,
    directActorParentResolver,
    dispatcher,
    pickDispatchConnection,
    sent,
    submitSpawn,
  };
}

function spawnSubmit(
  callerRequestId: string,
  targetKind: 'actorMethod' | 'function',
  callerKind: 'request' | 'actorInvocation' = 'request'
): SpawnSubmitRequestFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.request',
    rpcId: `spawn-rpc-${targetKind}`,
    runtimeId: RUNTIME_ID,
    callerKind,
    activationIdentity: {
      assemblyIdentity: ASSEMBLY_IDENTITY,
      generation: 1,
      runtimeReplicaId: RUNTIME_ID,
      deploymentRevision: 'revision-1',
    },
    targetKind,
    serviceId: SERVICE_ID,
    serviceVersion: '1.0.0',
    serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY,
    target: targetKind === 'actorMethod'
      ? 'actorMethod:example.Counter:increment'
      : 'function:example.nested',
    buildId: BUILD_ID,
    callerRequestId,
    ...(targetKind === 'actorMethod'
      ? {
          actorMethod: {
            actorRef: {
              serviceId: SERVICE_ID,
              actorTypeIdentity: 'actor.example.Counter',
              actorIdTypeIdentity: 'type.example.CounterId',
              actorIdEncodingVersion: 'json-v1',
              canonicalActorIdKeyBytesBase64: Buffer.from('"counter-1"').toString(
                'base64'
              ),
              actorIdHash: `sha256:${'1'.repeat(64)}`,
              epoch: 1,
            },
            declarationOwner: {
              unit: { kind: 'service' },
              file: { kind: 'loadedFileIndex', value: 0 },
              actorSymbol: 'Counter',
            },
            actorAbiIdentity: `skiff-actor-abi-v1:sha256:${'2'.repeat(64)}`,
            actorImplementationIdentity:
              `skiff-actor-implementation-v1:sha256:${'3'.repeat(64)}`,
            methodIdentity: `skiff-actor-method-v1:sha256:${'4'.repeat(64)}`,
          },
        }
      : {}),
  };
}

function actorParent(
  ws: WebSocket,
  authority: RuntimeSpawnParentAuthority
): ActiveActorInvocationParent {
  return Object.freeze({
    originRuntimeId: RUNTIME_ID,
    originRuntimeConnection: ws,
    testCaseCapability: CAPABILITY,
    authority,
  });
}

function rootAuthority(): RuntimeSpawnParentAuthority {
  return Object.freeze({
    runtimeId: RUNTIME_ID,
    buildId: BUILD_ID,
    serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY,
    assemblyIdentity: ASSEMBLY_IDENTITY,
    assemblyGeneration: 1,
    testCaseCapability: CAPABILITY,
    deployment: Object.freeze({
      serviceId: SERVICE_ID,
      contractVersion: '1.0.0',
      deploymentRevision: 'revision-1',
      deploymentArtifactIdentity: DEPLOYMENT_ARTIFACT_IDENTITY,
    }),
  });
}

function runtimeConnection(ws: WebSocket): RuntimeDispatchConnection {
  return {
    runtimeId: RUNTIME_ID,
    ws,
    runtimeAssemblyAuthority: {
      assemblyIdentity: ASSEMBLY_IDENTITY,
      assemblyGeneration: 1,
      deployment: {
        serviceId: SERVICE_ID,
        contractVersion: '1.0.0',
        deploymentRevision: 'revision-1',
        deploymentArtifactIdentity: DEPLOYMENT_ARTIFACT_IDENTITY,
      },
      buildId: BUILD_ID,
      serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY,
    },
  };
}

function correlation(
  parentRequestId: string,
  override: Partial<RuntimeSelfIngressTestCorrelation> = {}
): RuntimeSelfIngressTestCorrelation {
  return {
    parentRequestId,
    testCaseCapability: CAPABILITY,
    buildId: BUILD_ID,
    serviceProtocolIdentity: SERVICE_PROTOCOL_IDENTITY,
    ...override,
  };
}

function testRequest(
  requestId: string,
  testCaseCapability: string,
  serviceId = SERVICE_ID,
  testCaseParentRequestId?: string,
  assemblyGeneration = 1
): RuntimeAssemblyRequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId,
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY_IDENTITY,
      assemblyGeneration,
      deployment: {
        serviceId,
        contractVersion: '1.0.0',
        deploymentRevision: 'revision-1',
        deploymentArtifactIdentity: DEPLOYMENT_ARTIFACT_IDENTITY,
      },
      gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
      ingress: { protocol: 'http', method: 'POST', path: '/test' },
    },
    deadline: {
      timeoutMs: 1_000,
      expiresAt: new Date(Date.now() + 1_000).toISOString(),
    },
    trace: { traceId: 'trace:self-ingress', spanId: `span:${requestId}` },
    httpRequest: {
      method: 'POST',
      url: 'http://example.local/test',
      path: '/test',
      query: [],
      headers: [],
    },
    testEffectsEnabled: true,
    testCaseCapability,
    ...(testCaseParentRequestId === undefined
      ? {}
      : { testCaseParentRequestId }),
  };
}

function response(requestId: string) {
  return {
    header: {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end' as const,
      requestId,
      payloadPresent: false,
      httpResponse: { status: 204, headers: [] },
    },
    payloadBytes: new Uint8Array(),
  };
}

function openSocket(label: string): WebSocket {
  return {
    readyState: WebSocket.OPEN,
    label,
  } as unknown as WebSocket;
}

function identity(prefix: string, digit: string): string {
  return `${prefix}:${digit.repeat(64)}`;
}
