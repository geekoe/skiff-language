import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';
import { ProductionActorMethodRouter } from '../src/router/productionActorMethodRouter.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  ACTOR_ARGUMENTS_ENCODING_V1,
  ACTOR_RETURN_ENCODING_V1,
  decodeActorMethodFrame,
  encodeActorMethodFrame,
  type ActorMethodInvokeFrameHeader,
} from '../src/protocol/actorMethodProtocol.js';
import {
  decodeActorOwnerInvokeFrame,
} from '../src/protocol/actorOwnerProtocol.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type SpawnSubmitRequestFrameHeader,
} from '../src/protocol/envelope.js';

const sockets: WebSocket[] = [];
const endpoints: RuntimeEndpoint[] = [];

afterEach(async () => {
  for (const socket of sockets.splice(0)) socket.close();
  await Promise.all(endpoints.splice(0).map((endpoint) => endpoint.close()));
});

const SERVICE_ID = 'example.com/actor';
const DECLARATION_OWNER = {
  unit: { kind: 'service' as const },
  file: { kind: 'loadedFileIndex' as const, value: 0 },
  actorSymbol: 'example.Counter',
};
const ACTOR_ABI = identity('skiff-actor-abi-v1:sha256', 'a');
const ACTOR_IMPLEMENTATION = identity(
  'skiff-actor-implementation-v1:sha256',
  'b'
);
const METHOD = identity('skiff-actor-method-v1:sha256', 'e');
const BUILD = identity('skiff-service-build-v1:sha256', 'c');
const SERVICE_PROTOCOL = identity('skiff-service-protocol-v5:sha256', 'd');
const ASSEMBLY = 'skiff-runtime-assembly-v3:sha256:' + 'f'.repeat(64);
const ARTIFACT = 'skiff-service-artifact-v1:sha256:' + 'g'.repeat(64);

describe('actor method spawn submit', () => {
  it('dispatches a spawned actor method to its owner without forwarding the result', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'not-needed-for-direct-submit',
      actor,
    });
    const result = await actorMethods.submitSpawn(submit, Buffer.from('[1]'));

    const ownerFrame = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(ownerFrame.header.invoke.invocationId).toBe(result.requestId);
    expect(ownerFrame.header.invoke.actorRef.epoch).toBe(actor.epoch);
    expect(ownerFrame.header.invoke.methodIdentity).toBe(METHOD);
    expect(ownerFrame.header.invoke.traceId).toBeUndefined();
    expect(Array.from(ownerFrame.payloadBytes)).toEqual([91, 49, 93]);

    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: result.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () => {
      const invocation = await registry
        .actorManager()
        .registryStore()
        .actorInvocation(result.requestId);
      return invocation?.state === 'completed';
    });
    await expect(
      registry.actorManager().registryStore().actorInvocation(result.requestId)
    ).resolves.not.toBeUndefined();
  });

  it('forwards the submit traceId into the dispatched actor method invoke', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'trace-parent-invoke-1',
      actor,
      traceId: 'trace:spawn-submit-1',
    });
    const result = await actorMethods.submitSpawn(submit, Buffer.from('[4]'));

    const ownerFrame = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(ownerFrame.header.invoke.invocationId).toBe(result.requestId);
    expect(ownerFrame.header.invoke.traceId).toBe('trace:spawn-submit-1');
  });

  it('activates a not-live actor from its saved entry before queuing the spawn', async () => {
    const { actorMethods, registry, left } = await spawnHarness();
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    const evicted = await registry
      .actorManager()
      .registryStore()
      .evictIdleActor(actorKeyOf(actor));
    expect(evicted).toBe(true);

    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'not-needed-for-direct-submit',
      actor,
    });
    const result = await actorMethods.submitSpawn(submit, Buffer.from('[2]'));
    const ownerFrame = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(ownerFrame.header.invoke.invocationId).toBe(result.requestId);
    expect(ownerFrame.header.activationBootstrap).toMatchObject({
      encodingVersion: 'skiff-canonical-v1',
    });
  });

  it('accepts an actor-method parent and returns submitted before the spawned call finishes', async () => {
    const { registry, actorMethods, dispatcher, left } = await spawnHarness({
      dispatcher: true,
    });
    if (dispatcher === undefined) throw new Error('dispatcher harness missing');
    const actor = await registry
      .actorManager()
      .getOrCreate(actorBootstrap());
    left.send(encodeActorMethodFrame(invocation(actor, 'parent-invoke-1')));
    const parentOwner = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(parentOwner.header.invoke.invocationId).toBe('parent-invoke-1');
    const serverWs = registry.runtimeConnection('runtime-a')!.ws;
    await waitFor(() =>
      actorMethods.hasActiveActorInvocation({
        invocationId: 'parent-invoke-1',
        ws: serverWs,
        serviceId: SERVICE_ID,
        serviceProtocolIdentity: ACTOR_ABI,
      })
    );

    const submit = spawnSubmit({
      runtimeId: 'runtime-a',
      callerRequestId: 'parent-invoke-1',
      actor,
    });
    const response = await dispatcher.handleSpawnSubmit(
      serverWs,
      submit,
      Buffer.from('[3]')
    );
    const header = response.header;
    expect(header.type).toBe('spawn.submit.response');
    if (header.type !== 'spawn.submit.response') return;
    expect(header.status).toBe('submitted');
    const spawnOwner = decodeActorOwnerInvokeFrame(
      await nextBinary(left)
    );
    expect(spawnOwner.header.invoke.invocationId).toBe(
      header.requestId
    );
    expect(spawnOwner.header.invoke.invocationId).not.toBe('parent-invoke-1');
    left.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: header.requestId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await waitForAsync(async () => {
      const invocation = await registry
        .actorManager()
        .registryStore()
        .actorInvocation(header.requestId);
      return invocation?.state === 'completed';
    });
  });
});

async function spawnHarness({
  dispatcher = false,
}: { dispatcher?: boolean } = {}) {
  const registry = new RuntimeRegistry();
  const disconnect = new ActorRuntimeDisconnectController(registry.actorManager());
  const endpoint = new RuntimeEndpoint({
    registry,
    actorRuntimeDisconnect: disconnect,
  });
  endpoints.push(endpoint);
  const actorMethods = new ProductionActorMethodRouter({
    registry,
    disconnectController: disconnect,
    catalog: {
      hasMethod: () => true,
      declarationOwnerFor: () => DECLARATION_OWNER,
    },
    send: (ws, bytes) => ws.send(bytes),
    id: () => 'spawn-id',
  });
  endpoint.setActorMethods(actorMethods);
  const listening = await endpoint.listen({ port: 0 });
  const left = await runtime(listening.url, 'runtime-a');
  let dispatcherInstance: RuntimeDispatcher | undefined;
  if (dispatcher) {
    const runtimeIdByWs = new Map<WebSocket, string>([
      [registry.runtimeConnection('runtime-a')!.ws, 'runtime-a'],
    ]);
    dispatcherInstance = new RuntimeDispatcher({
      registry: {
        setInFlightCounter: () => {},
        pickDispatchConnection: () => null,
        refreshAllRuntimeStates: () => {},
        refreshRuntimeStatesForRequest: () => {},
        spawnSubmitParentAuthority: (ws) => {
          const runtimeId = runtimeIdByWs.get(ws);
          if (runtimeId === undefined) return undefined;
          return {
            runtimeId,
            buildId: BUILD,
            serviceProtocolIdentity: SERVICE_PROTOCOL,
            assemblyIdentity: ASSEMBLY,
            assemblyGeneration: 1,
            deployment: {
              serviceId: SERVICE_ID,
              contractVersion: '1.0.0',
              deploymentRevision: 'rev-1',
              deploymentArtifactIdentity: ARTIFACT,
            },
          };
        },
      },
      frameSender: endpoint,
      maxConcurrency: 256,
      actorMethodSpawn: actorMethods,
    });
    endpoint.setDispatcher(dispatcherInstance);
  }
  return {
    registry,
    actorMethods,
    dispatcher: dispatcherInstance,
    left,
    ownerSocket: left,
  };
}

function spawnSubmit({
  runtimeId,
  callerRequestId,
  actor,
  traceId,
}: {
  runtimeId: string;
  callerRequestId: string;
  actor: Awaited<
    ReturnType<ReturnType<RuntimeRegistry['actorManager']>['getOrCreate']>
  >;
  traceId?: string;
}): SpawnSubmitRequestFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.request',
    rpcId: 'spawn-rpc-1',
    runtimeId,
    activationIdentity: {
      assemblyIdentity: ASSEMBLY,
      generation: 1,
      runtimeReplicaId: runtimeId,
      deploymentRevision: 'rev-1',
    },
    targetKind: 'actorMethod',
    serviceId: SERVICE_ID,
    serviceVersion: '1.0.0',
    serviceProtocolIdentity: ACTOR_ABI,
    target: `actorMethod:example.Counter:${METHOD}`,
    spawnId: 'spawn-1',
    buildId: BUILD,
    callerRequestId,
    ...(traceId === undefined ? {} : { traceId }),
    actorMethod: {
      actorRef: {
        serviceId: actor.serviceId,
        actorTypeIdentity: actor.actorTypeIdentity,
        actorIdTypeIdentity: actor.actorIdTypeIdentity,
        actorIdEncodingVersion: actor.actorIdEncodingVersion,
        canonicalActorIdKeyBytesBase64: Buffer.from(
          actor.canonicalActorIdKeyBytes
        ).toString('base64'),
        actorIdHash: actor.actorIdHash,
        epoch: actor.epoch!,
      },
      declarationOwner: DECLARATION_OWNER,
      actorAbiIdentity: ACTOR_ABI,
      actorImplementationIdentity: ACTOR_IMPLEMENTATION,
      methodIdentity: METHOD,
    },
  };
}

function actorBootstrap() {
  return {
    actorKey: {
      serviceId: SERVICE_ID,
      actorTypeIdentity: 'actor.example.Counter',
      actorIdTypeIdentity: 'type.example.CounterId',
      actorIdEncodingVersion: 'skiff-canonical-v1',
      canonicalActorIdKeyBytes: new Uint8Array([1]),
    },
    actorAbiIdentity: ACTOR_ABI,
    actorImplementationIdentity: ACTOR_IMPLEMENTATION,
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    encodedBootstrapBytes: Buffer.from('{}'),
  };
}

function actorKeyOf(
  actor: Awaited<
    ReturnType<ReturnType<RuntimeRegistry['actorManager']>['getOrCreate']>
  >
) {
  return {
    serviceId: actor.serviceId,
    actorTypeIdentity: actor.actorTypeIdentity,
    actorIdTypeIdentity: actor.actorIdTypeIdentity,
    actorIdEncodingVersion: actor.actorIdEncodingVersion,
    canonicalActorIdKeyBytes: actor.canonicalActorIdKeyBytes,
    actorIdHash: actor.actorIdHash,
  };
}

function invocation(
  actor: Awaited<
    ReturnType<ReturnType<RuntimeRegistry['actorManager']>['getOrCreate']>
  >,
  invocationId: string
): ActorMethodInvokeFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.method.invoke',
    invocationId,
    actorRef: {
      serviceId: actor.serviceId,
      actorTypeIdentity: actor.actorTypeIdentity,
      actorIdTypeIdentity: actor.actorIdTypeIdentity,
      actorIdEncodingVersion: actor.actorIdEncodingVersion,
      canonicalActorIdKeyBytesBase64: Buffer.from(
        actor.canonicalActorIdKeyBytes
      ).toString('base64'),
      actorIdHash: actor.actorIdHash,
      epoch: actor.epoch!,
    },
    declarationOwner: DECLARATION_OWNER,
    actorAbiIdentity: ACTOR_ABI,
    actorImplementationIdentity: ACTOR_IMPLEMENTATION,
    methodIdentity: METHOD,
    argumentsEncodingVersion: ACTOR_ARGUMENTS_ENCODING_V1,
    deadline: {
      timeoutMs: 60_000,
      expiresAt: new Date(Date.now() + 60_000).toISOString(),
    },
    cancellationCorrelation: `cancel-${invocationId}`,
  };
}

async function runtime(url: string, runtimeId: string): Promise<WebSocket> {
  const socket = new WebSocket(url);
  sockets.push(socket);
  await new Promise<void>((resolve, reject) => {
    socket.once('open', resolve);
    socket.once('error', reject);
  });
  socket.send(encodeRuntimeFrame({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.capabilities',
    runtimeId,
    capabilities: { runtimeProgram: true },
  }));
  socket.send(encodeRuntimeFrame({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.register',
    runtimeId,
    serviceId: SERVICE_ID,
    revisionId: 'a'.repeat(64),
    buildId: BUILD,
    serviceProtocolIdentity: SERVICE_PROTOCOL,
    targets: ['actor.example.Counter.increment'],
  }));
  const response = decodeBinaryFrame(await nextBinary(socket));
  expect(response.header.type).toBe('runtime.registered');
  return socket;
}

function nextBinary(socket: WebSocket): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    socket.once('message', (data, binary) => {
      if (!binary) reject(new Error('expected binary frame'));
      else resolve(Buffer.isBuffer(data) ? data : Buffer.from(data as ArrayBuffer));
    });
    socket.once('error', reject);
  });
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const started = Date.now();
  while (!predicate()) {
    if (Date.now() - started > 1_000) {
      throw new Error('timed out waiting for predicate');
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

async function waitForAsync(predicate: () => Promise<boolean>): Promise<void> {
  const started = Date.now();
  while (!(await predicate())) {
    if (Date.now() - started > 1_000) {
      throw new Error('timed out waiting for async predicate');
    }
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

function identity(prefix: string, digit: string): string {
  return `${prefix}:${digit.repeat(64)}`;
}
