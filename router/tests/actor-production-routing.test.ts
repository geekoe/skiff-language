import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { makeActorKey } from '../src/actor/index.js';
import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';
import type { ActorOwnerTransport } from '../src/router/actorMethodDispatcher.js';
import { ProductionActorMethodRouter } from '../src/router/productionActorMethodRouter.js';
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
  decodeActorOwnerFailureFrame,
  decodeActorOwnerInvokeFrame,
  encodeActorOwnerFailureFrame,
} from '../src/protocol/actorOwnerProtocol.js';
import {
  decodeBinaryFrame,
  encodeBinaryFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
} from '../src/protocol/envelope.js';

const sockets: WebSocket[] = [];
const endpoints: RuntimeEndpoint[] = [];

afterEach(async () => {
  for (const socket of sockets.splice(0)) socket.close();
  await Promise.all(endpoints.splice(0).map((endpoint) => endpoint.close()));
});

describe('production Actor WebSocket routing', () => {
  it('selects one of two Runtimes, sticks to its owner and correlates the return', async () => {
    const registry = new RuntimeRegistry();
    const disconnect = new ActorRuntimeDisconnectController(registry.actorManager());
    const endpoint = new RuntimeEndpoint({
      registry,
      actorRuntimeDisconnect: disconnect,
    });
    endpoints.push(endpoint);
    const declarationOwner = {
      unit: { kind: 'service' as const },
      file: { kind: 'loadedFileIndex' as const, value: 0 },
      actorSymbol: 'example.Counter',
    };
    const actorMethods = new ProductionActorMethodRouter({
      registry,
      actorOwnerRouteAuthority: ({ serviceId }) =>
        serviceId === 'example.com/actor'
          ? {
              assemblyIdentity:
                'skiff-runtime-assembly-v3:sha256:' + 'a'.repeat(64),
              assemblyGeneration: 1,
            }
          : undefined,
      disconnectController: disconnect,
      catalog: {
        hasMethod: () => true,
        declarationOwnerFor: () => declarationOwner,
      },
      send: (ws, bytes) => ws.send(bytes),
      id: () => 'lease',
    });
    endpoint.setActorMethods(actorMethods);
    const listening = await endpoint.listen({ port: 0 });
    const left = await runtime(listening.url, 'runtime-a');
    const right = await runtime(listening.url, 'runtime-b');
    const actorKey = {
      serviceId: 'example.com/actor',
      actorTypeIdentity: 'actor.example.Counter',
      actorIdTypeIdentity: 'type.example.CounterId',
      actorIdEncodingVersion: 'skiff-canonical-v1',
      canonicalActorIdKeyBytes: new Uint8Array([1]),
    };
    const actor = await registry.actorManager().getOrCreate({
      actorKey,
      actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', 'a'),
      actorImplementationIdentity: identity(
        'skiff-actor-implementation-v1:sha256',
        'b'
      ),
      bootstrapEncodingVersion: 'skiff-canonical-v1',
      encodedBootstrapBytes: Buffer.from('{}'),
    });
    const invoke = invocation(actor, 'invoke-1');
    left.send(encodeActorMethodFrame(invoke, new Uint8Array([7])));

    const selected = await Promise.race([
      nextBinary(left).then((bytes) => ({ socket: left, bytes })),
      nextBinary(right).then((bytes) => ({ socket: right, bytes })),
    ]);
    const owner = decodeActorOwnerInvokeFrame(selected.bytes);
    expect(owner.header.invoke.invocationId).toBe('invoke-1');
    expect(Array.from(owner.payloadBytes)).toEqual([7]);
    selected.socket.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: 'invoke-1',
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }, new Uint8Array([9])));
    const returned = decodeActorMethodFrame(await nextBinary(left));
    expect(returned.header.type).toBe('actor.method.return');
    expect(Array.from(returned.payloadBytes)).toEqual([9]);

    left.send(encodeActorMethodFrame(invocation(actor, 'invoke-2')));
    const again = decodeActorOwnerInvokeFrame(await nextBinary(selected.socket));
    expect(again.header.targetRuntimeId).toBe(owner.header.targetRuntimeId);
    selected.socket.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: 'invoke-2',
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await nextBinary(left);

    left.send(encodeActorMethodFrame({
      ...invocation(actor, 'invoke-3'),
      actorImplementationIdentity: identity(
        'skiff-actor-implementation-v1:sha256',
        'c'
      ),
    }));
    for (const operation of ['markUpgrading', 'discard', 'activate'] as const) {
      const control = decodeBinaryFrame(await nextBinary(selected.socket));
      expect(control.header).toMatchObject({
        type: 'actor.owner.control',
        operation,
        targetRuntimeId: owner.header.targetRuntimeId,
      });
      selected.socket.send(encodeBinaryFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.owner.control.ack',
        runtimeId: owner.header.targetRuntimeId,
        requestId: control.header.requestId,
        operation,
        accepted: true,
      }, new Uint8Array()));
    }
    const upgraded = decodeActorOwnerInvokeFrame(await nextBinary(selected.socket));
    expect(upgraded.header.invoke.actorImplementationIdentity).toBe(
      identity('skiff-actor-implementation-v1:sha256', 'c')
    );
    expect(upgraded.header.invoke.actorRef.epoch).toBe(actor.epoch! + 1);
    selected.socket.send(encodeActorMethodFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId: 'invoke-3',
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    }));
    await nextBinary(left);
    const live = await registry.actorManager().entry(actorKey);
    if (
      live?.ownerRuntimeId === undefined ||
      live.ownerLeaseId === undefined ||
      live.ownerLeaseExpiresAt === undefined
    ) {
      throw new Error('upgraded Actor owner is missing');
    }
    const eviction = actorMethods.evictIdleOwner({
      actorKey: live.actorKey,
      epoch: live.epoch,
      implementationIdentity: live.actorImplementationIdentity,
      ownerRuntimeId: live.ownerRuntimeId,
      ownerLeaseId: live.ownerLeaseId,
      ownerLeaseExpiresAt: live.ownerLeaseExpiresAt,
      evictionRequestId: 'evict-1',
    });
    const idle = decodeBinaryFrame(await nextBinary(selected.socket));
    expect(idle.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'idleEvict',
      fence: { evictionRequestId: 'evict-1' },
    });
    selected.socket.send(encodeBinaryFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.control.ack',
      runtimeId: live.ownerRuntimeId,
      requestId: idle.header.requestId,
      operation: 'idleEvict',
      accepted: true,
    }, new Uint8Array()));
    await eviction;

    left.send(encodeActorMethodFrame({
      ...invocation(actor, 'invoke-4'),
      actorRef: {
        ...invocation(actor, 'invoke-4').actorRef,
        epoch: live.epoch,
      },
      actorImplementationIdentity: live.actorImplementationIdentity,
    }));
    const failing = decodeActorOwnerInvokeFrame(await nextBinary(selected.socket));
    selected.socket.send(encodeActorOwnerFailureFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.failure',
      invocationId: 'invoke-4',
      ownerRuntimeId: failing.header.ownerFence.ownerRuntimeId,
      ownerLeaseId: failing.header.ownerFence.ownerLeaseId,
      epoch: failing.header.ownerFence.epoch,
      actorImplementationIdentity:
        failing.header.ownerFence.actorImplementationIdentity,
      reason: {
        code: 'ExecutionFailed',
        message: 'ordinary executor failure',
      },
    }));
    const failure = decodeActorOwnerFailureFrame(await nextBinary(left));
    expect(failure.reason.code).toBe('ExecutionFailed');
    await expect(
      registry.actorManager().registryStore().actorInvocation('invoke-4')
    ).resolves.toMatchObject({
      state: 'failed',
      terminalReason: 'ExecutionFailed: ordinary executor failure',
    });
  });

  it('rejects an activate acknowledgement from W1 after the Runtime mapping moves to W2', async () => {
    const registry = new RuntimeRegistry();
    const disconnect = new ActorRuntimeDisconnectController(registry.actorManager());
    const endpoint = new RuntimeEndpoint({
      registry,
      actorRuntimeDisconnect: disconnect,
    });
    endpoints.push(endpoint);
    const actorMethods = new ProductionActorMethodRouter({
      registry,
      actorOwnerRouteAuthority: ({ serviceId }) =>
        serviceId === 'example.com/actor'
          ? {
              assemblyIdentity:
                'skiff-runtime-assembly-v3:sha256:' + 'a'.repeat(64),
              assemblyGeneration: 1,
            }
          : undefined,
      disconnectController: disconnect,
      catalog: {
        hasMethod: () => true,
        declarationOwnerFor: () => ({
          unit: { kind: 'service' },
          file: { kind: 'loadedFileIndex', value: 0 },
          actorSymbol: 'example.Counter',
        }),
      },
      send: (ws, bytes) => ws.send(bytes),
      id: () => 'session-fenced-activate',
    });
    endpoint.setActorMethods(actorMethods);
    const listening = await endpoint.listen({ port: 0 });
    const first = await runtime(listening.url, 'runtime-a');
    const firstServerConnection = registry.runtimeConnection('runtime-a')!.ws;
    const actorKey = makeActorKey({
      serviceId: 'example.com/actor',
      actorTypeIdentity: 'actor.example.Counter',
      actorIdTypeIdentity: 'type.example.CounterId',
      actorIdEncodingVersion: 'skiff-canonical-v1',
      canonicalActorIdKeyBytes: new Uint8Array([9]),
    });
    const actor = await registry.actorManager().getOrCreate({
      actorKey,
      actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', 'a'),
      actorImplementationIdentity: identity(
        'skiff-actor-implementation-v1:sha256',
        'b'
      ),
      bootstrapEncodingVersion: 'skiff-canonical-v1',
      encodedBootstrapBytes: Buffer.from('{}'),
    });
    const transport = (
      actorMethods as unknown as { transport(): ActorOwnerTransport }
    ).transport();
    if (transport.activateTarget === undefined) {
      throw new Error('production activateTarget transport is missing');
    }

    const firstControlMessage = nextBinary(first);
    const activation = Promise.resolve(transport.activateTarget({
      transition: {
        actorKey,
        oldEpoch: actor.epoch!,
        newEpoch: actor.epoch! + 1,
        actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', 'a'),
        targetImplementationIdentity: identity(
          'skiff-actor-implementation-v1:sha256',
          'c'
        ),
        bootstrapEncodingVersion: 'skiff-canonical-v1',
        encodedBootstrapBytes: Buffer.from('{}'),
      },
      header: invocation(actor, 'activate-session-fence'),
    }));
    const control = decodeBinaryFrame(await firstControlMessage);
    expect(control.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'activate',
      targetRuntimeId: 'runtime-a',
    });
    if (control.header.type !== 'actor.owner.control') {
      throw new Error('expected Actor owner activate control');
    }
    const requestId = control.header.requestId;
    if (typeof requestId !== 'string') {
      throw new Error('expected Actor owner activate requestId');
    }

    void activation.catch(() => undefined);
    registry.removeRuntimeConnection(firstServerConnection);
    const second = await runtime(listening.url, 'runtime-a');
    expect(registry.runtimeConnection('runtime-a')?.ws).not.toBe(
      firstServerConnection
    );
    await expectNoBinary(second);

    expect(() => actorMethods.handleOwnerControlAck(firstServerConnection, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.control.ack',
      runtimeId: 'runtime-a',
      requestId,
      operation: 'activate',
      accepted: true,
    })).toThrow('Actor owner control acknowledgement is not correlated');
    await expect(activation).rejects.toThrow('Actor owner rejected activate');
    await expectNoBinary(second);
  });
});

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
    serviceId: 'example.com/actor',
    revisionId: 'a'.repeat(64),
    buildId: identity('skiff-service-build-v1:sha256', 'c'),
    serviceProtocolIdentity: identity(
      'skiff-service-protocol-v5:sha256',
      'd'
    ),
    targets: ['actor.example.Counter.increment'],
  }));
  await nextBinary(socket);
  return socket;
}

function invocation(
  actor: Awaited<ReturnType<ReturnType<RuntimeRegistry['actorManager']>['getOrCreate']>>,
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
    declarationOwner: {
      unit: { kind: 'service' },
      file: { kind: 'loadedFileIndex', value: 0 },
      actorSymbol: 'example.Counter',
    },
    actorAbiIdentity: identity('skiff-actor-abi-v1:sha256', 'a'),
    actorImplementationIdentity: identity(
      'skiff-actor-implementation-v1:sha256',
      'b'
    ),
    methodIdentity: identity('skiff-actor-method-v1:sha256', 'e'),
    argumentsEncodingVersion: ACTOR_ARGUMENTS_ENCODING_V1,
    deadline: {
      timeoutMs: 60_000,
      expiresAt: new Date(Date.now() + 60_000).toISOString(),
    },
    cancellationCorrelation: `cancel-${invocationId}`,
  };
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

async function expectNoBinary(socket: WebSocket): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.off('message', onMessage);
      socket.off('error', onError);
      resolve();
    }, 25);
    const onMessage = (_data: WebSocket.RawData, binary: boolean) => {
      clearTimeout(timer);
      socket.off('error', onError);
      reject(new Error(binary ? 'unexpected binary frame' : 'unexpected text frame'));
    };
    const onError = (error: Error) => {
      clearTimeout(timer);
      socket.off('message', onMessage);
      reject(error);
    };
    socket.once('message', onMessage);
    socket.once('error', onError);
  });
}

function identity(prefix: string, digit: string): string {
  return `${prefix}:${digit.repeat(64)}`;
}
