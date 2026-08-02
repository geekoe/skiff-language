import { describe, expect, it } from 'vitest';

import { ActorManager, makeActorKey, type ActorKeyInput } from '../src/actor/index.js';
import { ActorGetCreateActivationCoordinator } from '../src/router/actorGetCreateActivationCoordinator.js';
import { ActorMethodDispatcher, type ActorOwnerTransport } from '../src/router/actorMethodDispatcher.js';
import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';
import type { ActiveActorInvocationParent } from '../src/router/runtimeDispatcher.js';
import { decodeBinaryFrame } from '../src/protocol/envelope.js';
import {
  ACTOR_ARGUMENTS_ENCODING_V1,
  type ActorMethodInvokeFrameHeader,
} from '../src/protocol/actorMethodProtocol.js';
import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ActorGetOrCreateRequestFrameHeader,
} from '../src/protocol/envelope.js';
import type WebSocket from 'ws';
import {
  encodeActorOwnerControlFrame,
  type ActorOwnerControlFrameHeader,
} from '../src/protocol/actorOwnerProtocol.js';

const actorAbi = identity('skiff-actor-abi-v1:sha256', 'a');
const implementation = identity('skiff-actor-implementation-v1:sha256', 'b');
const methodIdentity = identity('skiff-actor-method-v1:sha256', 'c');
const DECLARATION_OWNER = {
  unit: { kind: 'service' as const },
  file: { kind: 'fileIrIdentity' as const, value: 'file:actor' },
  actorSymbol: 'Counter',
};

interface FakeSocket extends WebSocket {
  sent: Buffer[];
}

function fakeSocket(): FakeSocket {
  const sent: Buffer[] = [];
  return {
    sent,
    readyState: 1,
    send(bytes: Buffer) {
      sent.push(Buffer.from(bytes));
    },
  } as unknown as FakeSocket;
}

function coordinatorFor(
  manager: ActorManager,
  options: {
    sockets?: FakeSocket[];
    activationTimeoutMs?: number;
  } = {}
) {
  const sockets = options.sockets ?? [fakeSocket()];
  const disconnectController = new ActorRuntimeDisconnectController(manager);
  const coordinator = new ActorGetCreateActivationCoordinator({
    actorManager: manager,
    runtimeDirectory: {
      actorRuntimeCandidates: () =>
        sockets.map((ws, index) => ({ runtimeId: `runtime-${index}`, ws })),
      runtimeConnection: (runtimeId) => {
        const index = Number(runtimeId.slice('runtime-'.length));
        const ws = sockets[index];
        return ws === undefined ? undefined : { runtimeId, ws };
      },
      runtimeIdForConnection: (ws) => {
        const index = sockets.indexOf(ws as FakeSocket);
        return index < 0 ? undefined : `runtime-${index}`;
      },
      runtimeConnectionFenceForConnection: (ws) => {
        const index = sockets.indexOf(ws as FakeSocket);
        return index < 0
          ? undefined
          : { runtimeId: `runtime-${index}`, sessionId: `session-${index}` };
      },
    },
    disconnectController,
    send: (ws, bytes) => ws.send(bytes),
    activationTimeoutMs: options.activationTimeoutMs ?? 30_000,
    id: () => 'lease',
  });
  return { coordinator, sockets, disconnectController };
}

function activationFrame(socket: FakeSocket) {
  const frame = decodeBinaryFrame(socket.sent[0]!);
  expect(frame.header).toMatchObject({
    type: 'actor.owner.control',
    operation: 'activateInitial',
  });
  return frame.header as {
    requestId: string;
    bootstrap: { encodingVersion: string; payloadBase64: string };
    testCaseCapability?: string;
    testCaseParentRequestId?: string;
  };
}

function capabilityParent(
  socket: FakeSocket,
  runtimeId = 'runtime-0'
): ActiveActorInvocationParent {
  const testCaseCapability = 'case:capability_1';
  return Object.freeze({
    originRuntimeId: runtimeId,
    originRuntimeConnection: socket,
    testCaseCapability,
    authority: Object.freeze({
      runtimeId,
      buildId: 'skiff-service-build-v1:sha256:' + '1'.repeat(64),
      serviceProtocolIdentity:
        'skiff-service-protocol-v5:sha256:' + '2'.repeat(64),
      assemblyIdentity: 'skiff-runtime-assembly-v3:sha256:' + '3'.repeat(64),
      assemblyGeneration: 1,
      testCaseCapability,
      deployment: Object.freeze({
        serviceId: actorKeyInput().serviceId,
        contractVersion: '1.0.0',
        deploymentRevision: 'revision-1',
        deploymentArtifactIdentity:
          'skiff-deployment-artifact-v4:sha256:' + '4'.repeat(64),
      }),
    }),
  });
}

function ack(
  coordinator: ActorGetCreateActivationCoordinator,
  socket: FakeSocket,
  frame: { requestId: string },
  accepted: boolean,
  reason?: { code: string; message: string }
) {
  const claimed = coordinator.handleOwnerControlAck(socket, {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.owner.control.ack',
    runtimeId: 'runtime-0',
    requestId: frame.requestId,
    operation: 'activateInitial',
    accepted,
    ...(reason === undefined ? {} : { reason }),
  });
  expect(claimed).toBe(true);
}

function requestHeader(
  actorKey: ActorKeyInput,
  overrides: Record<string, unknown> = {}
): ActorGetOrCreateRequestFrameHeader {
  const canonical = makeActorKey(actorKey);
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.getOrCreate.request',
    rpcId: 'rpc-1',
    runtimeId: 'runtime-0',
    activationIdentity: {
      assemblyIdentity: 'skiff-runtime-assembly-v3:sha256:' + '1'.repeat(64),
      generation: 1,
      runtimeReplicaId: 'runtime-0',
      deploymentRevision: 'revision-1',
    },
    actorKey: {
      serviceId: canonical.serviceId,
      actorTypeIdentity: canonical.actorTypeIdentity,
      actorIdTypeIdentity: canonical.actorIdTypeIdentity,
      actorIdEncodingVersion: canonical.actorIdEncodingVersion,
      canonicalActorIdKeyBytesBase64: Buffer.from(
        canonical.canonicalActorIdKeyBytes
      ).toString('base64'),
    },
    actorAbiIdentity: actorAbi,
    actorImplementationIdentity: implementation,
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    declarationOwner: {
      unit: { kind: 'service' },
      file: { kind: 'fileIrIdentity', value: 'file:actor' },
      actorSymbol: 'Counter',
    },
    deadline: { timeoutMs: 30_000, expiresAt: '2099-01-01T00:00:00.000Z' },
    ...(overrides as object),
  } as ActorGetOrCreateRequestFrameHeader;
}

function actorKeyInput(): ActorKeyInput {
  return {
    serviceId: 'example.com/actor',
    actorTypeIdentity: 'actor.example.Counter',
    actorIdTypeIdentity: 'type.example.CounterId',
    actorIdEncodingVersion: 'json-v1',
    canonicalActorIdKeyBytes: new TextEncoder().encode('"counter-1"'),
  };
}

describe('Actor getOrCreate activation contract', () => {
  it('pins capability creation to the exact parent session and forwards its pair', async () => {
    const manager = new ActorManager();
    const sockets = [fakeSocket(), fakeSocket()];
    const { coordinator, disconnectController } = coordinatorFor(manager, { sockets });
    const parent = capabilityParent(sockets[1]!, 'runtime-1');
    const pending = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), {
        testCaseCapability: parent.testCaseCapability,
        testCaseParentRequestId: 'root-request-1',
      }),
      payloadBytes: new Uint8Array([1, 2, 3]),
      sourceRuntimeId: 'runtime-1',
      sourceConnection: sockets[1]!,
      capabilityParent: parent,
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(sockets[0]!.sent).toHaveLength(0);
    const frame = activationFrame(sockets[1]!);
    expect(frame).toMatchObject({
      testCaseCapability: 'case:capability_1',
      testCaseParentRequestId: 'root-request-1',
    });
    const entry = await manager.entry(actorKeyInput());
    expect(
      disconnectController.ownerFenceBoundToConnection(
        { runtimeId: 'runtime-1', sessionId: 'session-1' },
        {
          actorKey: makeActorKey(actorKeyInput()),
          epoch: entry!.epoch,
          implementationIdentity: implementation,
          declarationOwner: DECLARATION_OWNER,
          ownerRuntimeId: 'runtime-1',
          ownerLeaseId: entry!.ownerLeaseId!,
          ownerLeaseExpiresAt: entry!.ownerLeaseExpiresAt!,
        }
      )
    ).toBe(true);
    const claimed = coordinator.handleOwnerControlAck(sockets[1]!, {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.control.ack',
      runtimeId: 'runtime-1',
      requestId: frame.requestId,
      operation: 'activateInitial',
      accepted: true,
    });
    expect(claimed).toBe(true);
    await expect(pending).resolves.toMatchObject({
      header: { type: 'actor.getOrCreate.response' },
    });
  });

  it('rejects a forged capability before registry mutation or owner send', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager);
    const result = await coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), {
        testCaseCapability: 'case:forged',
        testCaseParentRequestId: 'missing-parent',
      }),
      payloadBytes: new Uint8Array([1]),
      sourceRuntimeId: 'runtime-0',
      sourceConnection: sockets[0]!,
    });
    expect(result.header).toMatchObject({
      type: 'actor.getOrCreate.error',
      error: { code: 'TestCapabilityParentRejected', status: 403 },
    });
    expect(sockets[0]!.sent).toHaveLength(0);
    await expect(manager.entry(actorKeyInput())).resolves.toBeUndefined();
  });

  it('keeps the owner-control capability pair strict and activateInitial-only', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager);
    const parent = capabilityParent(sockets[0]!);
    const pending = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), {
        testCaseCapability: parent.testCaseCapability,
        testCaseParentRequestId: 'root-owner-protocol',
      }),
      payloadBytes: new Uint8Array([1]),
      sourceRuntimeId: 'runtime-0',
      sourceConnection: sockets[0]!,
      capabilityParent: parent,
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    const raw = decodeBinaryFrame(sockets[0]!.sent[0]!).header as unknown as
      ActorOwnerControlFrameHeader;
    const {
      testCaseParentRequestId: _parentRequestId,
      ...halfPair
    } = raw;
    expect(() => encodeActorOwnerControlFrame(halfPair)).toThrow(
      'invalid actor owner control frame'
    );
    expect(() => encodeActorOwnerControlFrame({
      ...raw,
      testCaseCapability: 'invalid/capability',
    })).toThrow('invalid actor owner control frame');
    const { bootstrap: _bootstrap, deadline: _deadline, ...withoutActivation } = raw;
    expect(() => encodeActorOwnerControlFrame({
      ...withoutActivation,
      operation: 'discard',
    })).toThrow('invalid actor owner control frame');
    ack(coordinator, sockets[0]!, raw, true);
    await pending;
  });

  it('fails closed and rolls back when the origin Runtime reconnects before ack', async () => {
    const manager = new ActorManager();
    const original = fakeSocket();
    const sockets = [original];
    const { coordinator } = coordinatorFor(manager, { sockets });
    const parent = capabilityParent(original);
    const pending = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), {
        testCaseCapability: parent.testCaseCapability,
        testCaseParentRequestId: 'root-before-reconnect',
      }),
      payloadBytes: new Uint8Array([1]),
      sourceRuntimeId: 'runtime-0',
      sourceConnection: original,
      capabilityParent: parent,
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    const frame = activationFrame(original);
    sockets[0] = fakeSocket();
    ack(coordinator, original, frame, true);

    await expect(pending).resolves.toMatchObject({
      header: {
        type: 'actor.getOrCreate.error',
        error: { code: 'OwnerUnavailable', status: 503 },
      },
    });
    await expect(manager.entry(actorKeyInput())).resolves.toMatchObject({
      lifecycleState: 'inactive',
      ownerRuntimeId: undefined,
      ownerLeaseId: undefined,
    });
    expect(sockets[0]!.sent).toHaveLength(0);
  });

  it('waits for create to complete before returning the handle on a new entry', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager);
    const pending = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput()),
      payloadBytes: new Uint8Array([1, 2, 3]),
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(sockets[0]!.sent).toHaveLength(1);
    const frame = activationFrame(sockets[0]!);
    expect(frame.bootstrap).toEqual({
      encodingVersion: 'skiff-canonical-v1',
      payloadBase64: Buffer.from([1, 2, 3]).toString('base64'),
    });
    let settled = false;
    void pending.then(() => {
      settled = true;
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(settled).toBe(false);
    ack(coordinator, sockets[0]!, frame, true);
    const result = await pending;
    expect(result.header).toMatchObject({
      type: 'actor.getOrCreate.response',
      rpcId: 'rpc-1',
      actorRef: { epoch: 1 },
    });
    await expect(manager.entry(actorKeyInput())).resolves.toMatchObject({
      lifecycleState: 'live',
    });
  });

  it('surfaces create failure on get, retains the entry and allows a retry', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager);
    const first = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput()),
      payloadBytes: new Uint8Array([1]),
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    const frame = activationFrame(sockets[0]!);
    ack(coordinator, sockets[0]!, frame, false, {
      code: 'ActorCreateFailed',
      message: 'create boom',
    });
    const failed = await first;
    expect(failed.header).toMatchObject({
      type: 'actor.getOrCreate.error',
      error: { code: 'ActorCreateFailed', message: 'create boom', status: 500 },
    });
    await expect(manager.entry(actorKeyInput())).resolves.toMatchObject({
      status: 'present',
      lifecycleState: 'inactive',
      ownerRuntimeId: undefined,
    });
    // A retained entry is returned immediately by a retry get; creation is
    // re-attempted from the stored inputs by the first method call.
    const retry = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), { rpcId: 'rpc-2' }),
      payloadBytes: new Uint8Array([2]),
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    await expect(retry).resolves.toMatchObject({
      header: { type: 'actor.getOrCreate.response', rpcId: 'rpc-2', actorRef: { epoch: 1 } },
    });
    expect(sockets[0]!.sent).toHaveLength(1);
    await expect(manager.entry(actorKeyInput())).resolves.toMatchObject({
      status: 'present',
      lifecycleState: 'inactive',
      encodedBootstrapBytes: new Uint8Array([1]),
    });
  });

  it('returns immediately for an existing entry without touching stored creation inputs', async () => {
    const manager = new ActorManager();
    await manager.getOrCreate({
      actorKey: actorKeyInput(),
      actorAbiIdentity: actorAbi,
      actorImplementationIdentity: implementation,
      declarationOwner: DECLARATION_OWNER,
      bootstrapEncodingVersion: 'skiff-canonical-v1',
      encodedBootstrapBytes: new Uint8Array([9, 9]),
    });
    const { coordinator, sockets } = coordinatorFor(manager);
    const result = await coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), {
        bootstrapEncodingVersion: 'skiff-canonical-v1',
      }),
      payloadBytes: new Uint8Array([1, 2, 3]),
    });
    expect(result.header).toMatchObject({ type: 'actor.getOrCreate.response' });
    expect(sockets[0]!.sent).toHaveLength(0);
    await expect(manager.entry(actorKeyInput())).resolves.toMatchObject({
      encodedBootstrapBytes: new Uint8Array([9, 9]),
    });
  });

  it('deduplicates concurrent gets onto a single activation', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager);
    const first = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), { rpcId: 'rpc-1' }),
      payloadBytes: new Uint8Array([1]),
    });
    const second = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput(), { rpcId: 'rpc-2' }),
      payloadBytes: new Uint8Array([2]),
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(sockets[0]!.sent).toHaveLength(1);
    ack(coordinator, sockets[0]!, activationFrame(sockets[0]!), true);
    const [left, right] = await Promise.all([first, second]);
    expect(left.header).toMatchObject({ type: 'actor.getOrCreate.response', rpcId: 'rpc-1' });
    expect(right.header).toMatchObject({ type: 'actor.getOrCreate.response', rpcId: 'rpc-2' });
  });

  it('times out when the owner never acks, releases the lease and retains the entry', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager, {
      activationTimeoutMs: 40,
    });
    const started = Date.now();
    const result = await coordinator.getOrCreate({
      header: requestHeader(actorKeyInput()),
      payloadBytes: new Uint8Array([1]),
    });
    expect(Date.now() - started).toBeGreaterThanOrEqual(30);
    expect(result.header).toMatchObject({
      type: 'actor.getOrCreate.error',
      error: { code: 'ActorCreateTimeout', status: 504 },
    });
    await expect(manager.entry(actorKeyInput())).resolves.toMatchObject({
      status: 'present',
      lifecycleState: 'inactive',
      ownerRuntimeId: undefined,
      ownerLeaseId: undefined,
    });
  });

  it('keeps get waiting until the deadline when the owner disconnects', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager, {
      activationTimeoutMs: 50,
    });
    const pending = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput()),
      payloadBytes: new Uint8Array([1]),
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(sockets[0]!.sent).toHaveLength(1);
    coordinator.handleRuntimeDisconnect(sockets[0]!);
    let settled = false;
    void pending.then(() => {
      settled = true;
    });
    await new Promise((resolve) => setTimeout(resolve, 15));
    expect(settled).toBe(false);
    const result = await pending;
    expect(result.header).toMatchObject({
      type: 'actor.getOrCreate.error',
      error: { code: 'ActorCreateTimeout' },
    });
  });

  it('admits no method while the entry is activating', async () => {
    const manager = new ActorManager();
    const { coordinator, sockets } = coordinatorFor(manager);
    const pending = coordinator.getOrCreate({
      header: requestHeader(actorKeyInput()),
      payloadBytes: new Uint8Array([1]),
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    const entry = await manager.entry(actorKeyInput());
    expect(entry?.lifecycleState).toBe('activating');
    const admitted = await manager.registryStore().admitActorMethod({
      invocationId: 'invoke-during-create',
      actorKey: makeActorKey(actorKeyInput()),
      expectedEpoch: entry!.epoch,
      actorAbiIdentity: actorAbi,
      requestedImplementationIdentity: implementation,
      methodIdentity,
      methodKnown: true,
    });
    expect(admitted).toMatchObject({ ok: false, rejection: { reason: 'OwnerUnavailable' } });
    ack(coordinator, sockets[0]!, activationFrame(sockets[0]!), true);
    await pending;
  });

  it('replays stored creation inputs after eviction through the method path', async () => {
    const manager = new ActorManager();
    await manager.getOrCreate({
      actorKey: actorKeyInput(),
      actorAbiIdentity: actorAbi,
      actorImplementationIdentity: implementation,
      declarationOwner: DECLARATION_OWNER,
      bootstrapEncodingVersion: 'skiff-canonical-v1',
      encodedBootstrapBytes: new Uint8Array([7, 7]),
    });
    await manager.evictIdle(actorKeyInput());
    const { coordinator, sockets } = coordinatorFor(manager);
    const result = await coordinator.getOrCreate({
      header: requestHeader(actorKeyInput()),
      payloadBytes: new Uint8Array([1, 2, 3]),
    });
    expect(result.header).toMatchObject({ type: 'actor.getOrCreate.response' });
    expect(sockets[0]!.sent).toHaveLength(0);

    const delivered: Array<{ payloadBase64?: string }> = [];
    const ownerConnections = new Map<string, WebSocket>();
    const transport: ActorOwnerTransport = {
      activateInitial({ header: invoke }) {
        return {
          ownerRuntimeId: 'runtime-0',
          ownerLeaseId: 'lease-1',
          ownerLeaseExpiresAt: new Date(Date.now() + 60_000),
          ownerConnection: sockets[0]!,
        };
      },
      bindOwnerConnection({ ownerFence, requiredOwnerConnection }) {
        const key = `${ownerFence.ownerRuntimeId}:${ownerFence.ownerLeaseId}`;
        ownerConnections.set(key, requiredOwnerConnection);
        return {
          unbind() {
            if (ownerConnections.get(key) === requiredOwnerConnection) {
              ownerConnections.delete(key);
            }
          },
        };
      },
      ownerConnectionMatches({ ownerFence, requiredOwnerConnection }) {
        return (
          ownerConnections.get(
            `${ownerFence.ownerRuntimeId}:${ownerFence.ownerLeaseId}`
          ) === requiredOwnerConnection
        );
      },
      dispatchToOwner({ ownerFence, header, payloadBytes }) {
        void ownerFence;
        void header;
        void payloadBytes;
        delivered.push({});
      },
    };
    const dispatcher = new ActorMethodDispatcher(
      manager,
      { hasMethod: () => true },
      transport,
      () => new Date()
    );
    await dispatcher.dispatch(invokeFrame(actorKeyInput(), 1), new Uint8Array());
    const entry = await manager.entry(actorKeyInput());
    expect(entry?.encodedBootstrapBytes).toEqual(new Uint8Array([7, 7]));
    expect(delivered).toHaveLength(1);
  });
});

function invokeFrame(
  actorKey: ActorKeyInput,
  epoch: number
): ActorMethodInvokeFrameHeader {
  const canonical = makeActorKey(actorKey);
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.method.invoke',
    invocationId: 'invoke-1',
    actorRef: {
      serviceId: canonical.serviceId,
      actorTypeIdentity: canonical.actorTypeIdentity,
      actorIdTypeIdentity: canonical.actorIdTypeIdentity,
      actorIdEncodingVersion: canonical.actorIdEncodingVersion,
      canonicalActorIdKeyBytesBase64: Buffer.from(
        canonical.canonicalActorIdKeyBytes
      ).toString('base64'),
      actorIdHash: canonical.actorIdHash,
      epoch,
    },
    declarationOwner: {
      unit: { kind: 'service' },
      file: { kind: 'fileIrIdentity', value: 'file:actor' },
      actorSymbol: 'Counter',
    },
    actorAbiIdentity: actorAbi,
    actorImplementationIdentity: implementation,
    methodIdentity,
    argumentsEncodingVersion: ACTOR_ARGUMENTS_ENCODING_V1,
    deadline: { timeoutMs: 1_000, expiresAt: new Date(Date.now() + 1_000).toISOString() },
    cancellationCorrelation: 'cancel-1',
  };
}

function identity(prefix: string, character: string): string {
  return `${prefix}:${character.repeat(64)}`;
}
