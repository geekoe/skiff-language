import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import {
  decodeAssemblyActivationFrame,
  encodeAssemblyActivationFrame
} from '../src/protocol/assemblyActivationFrame.js';
import type { AssemblyActivationControl } from '../src/protocol/assemblyActivationProtocol.js';
import {
  decodeBinaryFrame,
  decodeRuntimeFrame,
  encodeBinaryFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ResponseEndFrameHeader,
  type RuntimeBinaryFrame
} from '../src/protocol/envelope.js';
import {
  runtimeFrameHeaderFixtures,
  validateRuntimeAssemblyRequestStartFrameHeader,
  validateRuntimeAssemblyRequestStartFrameWireHeader
} from '../src/protocol/runtimeProtocol.js';
import { AssemblyActivationCoordinator } from '../src/router/assemblyActivationCoordinator.js';
import { ActorGetCreateActivationCoordinator } from '../src/router/actorGetCreateActivationCoordinator.js';
import { ActorSpawnRuntimeControl } from '../src/router/actorSpawnRuntimeControl.js';
import { ActorManager } from '../src/actor/index.js';
import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';
import {
  initialActivationState,
  MemoryAssemblyActivationStateStore
} from '../src/router/assemblyActivationStateStore.js';
import { AssemblyControlPlane } from '../src/router/assemblyControlPlane.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  MemoryRuntimeAssemblySnapshotLoader,
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type LoadedRuntimeAssembly
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY_A = identity('a');
const ASSEMBLY_B = identity('b');
const ASSEMBLY_C = identity('c');
const EMPTY_ASSEMBLY =
  'skiff-runtime-assembly-v3:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f';
const RUNTIME_ID = 'runtime-assembly-a';
const SERVICE_ID = 'example.com/actors';
const SECOND_SERVICE_ID = 'example.com/actors-case-two';
const SERVICE_VERSION = '1.0.0';
const SERVICE_PROTOCOL =
  `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`;
const SECOND_SERVICE_PROTOCOL =
  `skiff-service-protocol-v5:sha256:${'b'.repeat(64)}`;
const BUILD_ID = `skiff-service-build-v1:sha256:${'d'.repeat(64)}`;
const PACKAGE_BUILD_ID = `skiff-package-build-v10:sha256:${'d'.repeat(64)}`;
const TARGET = 'function:service.example~actors.ActorApi.spawn';
const CURRENT_TEST_GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'9'.repeat(64)}`;
const SECOND_TEST_GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'8'.repeat(64)}`;
const TEST_HOST = 'case-0.package-test.skiff.localhost';
const TEST_PATH = '/__skiff/package-test/0';
const SECOND_TEST_PATH = '/__skiff/package-test/1';
const CONFIG_SNAPSHOT = {
  snapshotId: 'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
};
const fixtures: CompositeEndpointFixture[] = [];

describe('unified RuntimeEndpoint assembly bootstrap', () => {
  afterEach(async () => {
    while (fixtures.length > 0) {
      await fixtures.pop()?.close();
    }
  });

  it('keeps one socket through capabilities, all six activation controls, health, and connection.send', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    await until(() => fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 1);
    expect(fixture.assemblyRegistry.healthyParticipantReplicaIds()).toEqual([]);

    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const prepareB = nextActivation(ws, 'prepare');
    const activationB = fixture.coordinator.activate(activationRequest('activation-b', 1, ASSEMBLY_B));
    expect(await prepareB).toEqual(transition('prepare', 'activation-b', 1, ASSEMBLY_B));
    const commitBFrame = nextActivation(ws, 'commit');
    sendActivation(ws, transition('prepared', 'activation-b', 1, ASSEMBLY_B));
    const commitB = await commitBFrame;
    await expect(activationB).resolves.toMatchObject({
      committed: {
        generation: 2,
        assembly: { assemblyIdentity: ASSEMBLY_B },
        configSnapshot: CONFIG_SNAPSHOT
      }
    });
    expect(commitB).toEqual(transition('commit', 'activation-b', 1, ASSEMBLY_B));

    sendActivation(ws, registration(2, ASSEMBLY_B));
    await until(() => fixture.assemblyRegistry.snapshot().some(
      (replica) => replica.generation === 2 && replica.state === 'healthy'
    ));
    const prepareC = nextActivation(ws, 'prepare');
    const activationC = fixture.coordinator.activate(activationRequest('activation-c', 2, ASSEMBLY_C));
    const activationCRejected = expect(activationC).rejects.toThrow(
      /rejected activation during admission/
    );
    expect(await prepareC).toEqual(transition('prepare', 'activation-c', 2, ASSEMBLY_C));
    const abortCFrame = nextActivation(ws, 'abort');
    sendActivation(ws, transition('reject', 'activation-c', 2, ASSEMBLY_C));
    const abortC = await abortCFrame;
    await activationCRejected;
    expect(abortC).toEqual(transition('abort', 'activation-c', 2, ASSEMBLY_C));

    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['runtime.health'],
      runtimeId: RUNTIME_ID
    }));
    await until(() => fixture.assemblyRegistry.snapshot()[0]?.lastHealthAt !== undefined);

    const connectionSend = new Promise<unknown>((resolve) => {
      fixture.endpoint.onConnectionSend(resolve);
    });
    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['connection.send']));
    await expect(connectionSend).resolves.toMatchObject({ type: 'connection.send' });
    expect(ws.readyState).toBe(WebSocket.OPEN);

    const runtimeConnection = fixture.assemblyRegistry.connectionForReplica(RUNTIME_ID);
    expect(runtimeConnection).toBeDefined();
    fixture.assemblyRegistry.setConnectionPinCounter({
      connectionPinCount: () => 0,
      connectionReleaseAckCount: (candidate) =>
        candidate === runtimeConnection ? 1 : 0
    });
    const health = await fetch(`${fixture.controlUrl}/__router/health`).then(async (response) => {
      expect(response.ok).toBe(true);
      return await response.json() as {
        activeAssembly: {
          configSnapshotId: string;
        };
        capabilityConnections: unknown[];
        replicas: unknown[];
      };
    });
    expect(health.activeAssembly.configSnapshotId).toBe(
      CONFIG_SNAPSHOT.snapshotId
    );
    expect(JSON.stringify(health)).not.toContain('resolvedConfig');
    expect(JSON.stringify(health)).not.toContain('configSnapshotPath');
    expect(health.capabilityConnections).toEqual([
      expect.objectContaining({ runtimeId: RUNTIME_ID, connected: true })
    ]);
    expect(health.replicas).toEqual([
      expect.objectContaining({
        replicaId: RUNTIME_ID,
        generation: 2,
        assemblyIdentity: ASSEMBLY_B,
        configSnapshotId: CONFIG_SNAPSHOT.snapshotId,
        state: 'healthy',
        connected: true,
        connectionReleaseAckCount: 1
      })
    ]);
  });

  it('dispatches exact kind:test control through the isolated test-effects seam', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const body = testDispatchBody();
    const responsePromise = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      body
    );
    const requestFrame = await nextRuntimeFrame(ws, 'request.start');
    const validation = validateRuntimeAssemblyRequestStartFrameHeader(
      requestFrame.header
    );
    expect(validation).toMatchObject({ ok: true });
    if (!validation.ok) throw new Error(validation.error);
    expect(validation.envelope).toMatchObject({
      mode: body.mode,
      routing: body.routing,
      httpRequest: body.httpRequest,
      testEffectsEnabled: true
    });
    expect(Buffer.from(requestFrame.payloadBytes)).toEqual(
      Buffer.from('null', 'utf8')
    );

    const responseHeader: ResponseEndFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: validation.envelope.requestId,
      payloadPresent: true,
      httpResponse: {
        status: 200,
        headers: [
          {
            name: 'content-type',
            value: 'application/json; charset=utf-8'
          }
        ]
      }
    };
    ws.send(
      encodeRuntimeFrame(responseHeader, Buffer.from('null', 'utf8'))
    );

    const response = await responsePromise;
    expect(response.status).toBe(200);
    expect(response.body).toEqual({
      ok: true,
      header: responseHeader,
      payloadBase64: Buffer.from('null', 'utf8').toString('base64')
    });
  });

  it('dispatches recursive spawn as detached requests and inherits one test case capability', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);
    const capability = rootValidation.envelope.testCaseCapability;
    expect(capability).toEqual(expect.any(String));

    const [child, childReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-child',
      rootValidation.envelope.requestId
    );
    const childValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(child.header);
    if (!childValidation.ok || !('invocation' in childValidation.envelope)) {
      throw new Error(
        childValidation.ok ? 'expected derived spawn invocation' : childValidation.error
      );
    }
    expect(childValidation.envelope).toMatchObject({
      caller: { kind: 'service' },
      routing: {
        assemblyIdentity: ASSEMBLY_A,
        assemblyGeneration: 1,
        deployment: deploymentRef(deploymentRevision(ASSEMBLY_A))
      },
      invocation: {
        kind: 'spawn',
        targetKind: 'function',
        target: TARGET
      },
      testEffectsEnabled: true,
      testCaseCapability: capability
    });
    expect([...child.payloadBytes]).toEqual([7, 8]);
    expect(childReceipt.header).toMatchObject({
      type: 'spawn.submit.response',
      rpcId: 'spawn-rpc-child',
      requestId: childValidation.envelope.requestId,
      status: 'submitted'
    });

    const [grandchild, grandchildReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-grandchild',
      childValidation.envelope.requestId
    );
    const grandchildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(grandchild.header);
    if (
      !grandchildValidation.ok ||
      !('invocation' in grandchildValidation.envelope)
    ) {
      throw new Error(
        grandchildValidation.ok
          ? 'expected recursive derived spawn invocation'
          : grandchildValidation.error
      );
    }
    expect(grandchildValidation.envelope.testCaseCapability).toBe(capability);
    expect(grandchildReceipt.header.type).toBe('spawn.submit.response');

    sendEmptyResponseEnd(ws, grandchildValidation.envelope.requestId);
    sendEmptyResponseEnd(ws, childValidation.envelope.requestId);
    sendRootResponseEnd(ws, rootValidation.envelope.requestId);
    await expect(rootResponse).resolves.toMatchObject({ status: 200 });
    await until(
      () => fixture.dispatcher.pendingLifecycleCounters().pendingUnary === 0
    );
  });

  it('authenticates root capability actor getOrCreate and forwards it to initial creation', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);
    const capability = rootValidation.envelope.testCaseCapability;
    if (capability === undefined) throw new Error('test root capability is required');

    const activationMessage = nextBinaryMessage(ws);
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-capability-create',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: actorKey(),
      testCaseCapability: capability,
      testCaseParentRequestId: rootValidation.envelope.requestId
    }, new Uint8Array([1, 2, 3])));
    const initialActivation = decodeBinaryFrame(await activationMessage);
    expect(initialActivation.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'activateInitial',
      targetRuntimeId: RUNTIME_ID,
      testCaseCapability: capability,
      testCaseParentRequestId: rootValidation.envelope.requestId
    });
    expect(initialActivation.header.routeAuthority).toEqual({
      assemblyIdentity: rootValidation.envelope.routing.assemblyIdentity,
      assemblyGeneration: rootValidation.envelope.routing.assemblyGeneration
    });
    const created = nextRuntimeFrame(ws, 'actor.getOrCreate.response');
    ws.send(encodeBinaryFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.control.ack',
      runtimeId: RUNTIME_ID,
      requestId: initialActivation.header.requestId,
      operation: 'activateInitial',
      accepted: true
    }, new Uint8Array()));
    await expect(created).resolves.toMatchObject({
      header: {
        rpcId: 'actor-capability-create',
        actorRef: { serviceId: SERVICE_ID, epoch: 1 }
      }
    });

    sendRootResponseEnd(ws, rootValidation.envelope.requestId);
    await expect(rootResponse).resolves.toMatchObject({ status: 200 });
  });

  it('cleans up a pending ActivateInitial create on Runtime disconnect and retries the same fence', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true,
      activationTimeoutMs: 60
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(
      () => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1
    );

    const activationMessage = nextBinaryMessage(ws);
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-disconnect-create',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: actorKey(),
    }, new Uint8Array([1, 2, 3])));
    const initialActivation = decodeBinaryFrame(await activationMessage);
    expect(initialActivation.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'activateInitial',
      targetRuntimeId: RUNTIME_ID,
    });
    expect(initialActivation.header.routeAuthority).toEqual({
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
    });

    const entryKey = {
      serviceId: actorKey().serviceId,
      actorTypeIdentity: actorKey().actorTypeIdentity,
      actorIdTypeIdentity: actorKey().actorIdTypeIdentity,
      actorIdEncodingVersion: actorKey().actorIdEncodingVersion,
      canonicalActorIdKeyBytes: Buffer.from(
        actorKey().canonicalActorIdKeyBytesBase64,
        'base64'
      ),
    };
    type ActorEntry = Awaited<ReturnType<ActorManager['entry']>>;
    async function untilEntry(
      predicate: (entry: ActorEntry) => boolean
    ): Promise<void> {
      for (let attempt = 0; attempt < 100; attempt += 1) {
        const entry = await fixture.runtimeRegistry.actorManager().entry(entryKey);
        if (predicate(entry)) return;
        await nextTurn();
      }
      throw new Error('actor entry condition was not reached');
    }
    await untilEntry((entry) => entry?.lifecycleState === 'activating');

    ws.close();
    await untilEntry(
      (entry) =>
        entry?.status === 'present' &&
        entry.lifecycleState === 'inactive' &&
        entry.ownerLeaseId === undefined
    );
    const retained = await fixture.runtimeRegistry.actorManager().entry(entryKey);
    expect(retained).toMatchObject({
      status: 'present',
      epoch: 1,
      lifecycleState: 'inactive',
      encodedBootstrapBytes: new Uint8Array([1, 2, 3]),
    });
    // The disconnect releases the lease immediately, but the first getOrCreate
    // claim stays pending until its activation deadline. Let it settle so the
    // retry does not join the failed claim.
    await new Promise((resolve) => setTimeout(resolve, 200));

    const second = await openSocket(fixture.url);
    sendCapabilities(second, RUNTIME_ID);
    sendActivation(second, registration(1, ASSEMBLY_A));
    await until(
      () => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1
    );
    const retryResponse = nextRuntimeFrame(second, 'actor.getOrCreate.response');
    second.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-disconnect-retry',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: actorKey(),
    }, new Uint8Array([1, 2, 3])));
    await expect(retryResponse).resolves.toMatchObject({
      header: {
        rpcId: 'actor-disconnect-retry',
        actorRef: { serviceId: SERVICE_ID, epoch: 1 },
      },
    });
    second.close();
  });

  it('keeps other socket frames flowing while an ActivateInitial admission awaits its ack', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true,
      activationTimeoutMs: 1_000,
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(
      () => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1
    );

    const firstControlMessage = nextBinaryMessage(ws);
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'pending-create-first',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: actorKey(),
    }, new Uint8Array([1])));
    const firstControl = decodeBinaryFrame(await firstControlMessage);
    expect(firstControl.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'activateInitial',
    });

    // The pending create admission must not serialize the whole socket:
    // health and a second independent create are still processed.
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['runtime.health'],
      runtimeId: RUNTIME_ID,
    }));
    await until(
      () => fixture.assemblyRegistry.snapshot()[0]?.lastHealthAt !== undefined
    );

    const secondKey = {
      ...actorKey(),
      canonicalActorIdKeyBytesBase64:
        Buffer.from('"thread-2"').toString('base64'),
    };
    const secondControlMessage = nextBinaryMessage(ws);
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'pending-create-second',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: secondKey,
    }, new Uint8Array([2])));
    const secondControl = decodeBinaryFrame(await secondControlMessage);
    expect(secondControl.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'activateInitial',
    });

    const responses = nextBinaryMessages(ws, 2);
    for (const requestId of [
      firstControl.header.requestId,
      secondControl.header.requestId,
    ]) {
      ws.send(encodeBinaryFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'actor.owner.control.ack',
        runtimeId: RUNTIME_ID,
        requestId,
        operation: 'activateInitial',
        accepted: true,
      }, new Uint8Array()));
    }
    const settled = await responses;
    expect(
      settled
        .map((frame) => decodeRuntimeFrame(frame).header.rpcId)
        .sort()
    ).toEqual(['pending-create-first', 'pending-create-second']);
    ws.close();
  });

  it('rejects a self-authorized getOrCreate capability before owner activation', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rejected = nextRuntimeFrame(ws, 'actor.getOrCreate.error');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-capability-forged',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: actorKey(),
      testCaseCapability: 'case:forged',
      testCaseParentRequestId: 'missing-parent'
    }, new Uint8Array([1])));
    await expect(rejected).resolves.toMatchObject({
      header: {
        rpcId: 'actor-capability-forged',
        error: { code: 'TestCapabilityParentRejected', status: 403 }
      }
    });
    await expect(
      fixture.runtimeRegistry.actorManager().entry({
        serviceId: actorKey().serviceId,
        actorTypeIdentity: actorKey().actorTypeIdentity,
        actorIdTypeIdentity: actorKey().actorIdTypeIdentity,
        actorIdEncodingVersion: actorKey().actorIdEncodingVersion,
        canonicalActorIdKeyBytes: Buffer.from(
          actorKey().canonicalActorIdKeyBytesBase64,
          'base64'
        )
      })
    ).resolves.toBeUndefined();
  });

  it('inherits spawn authority from a pinned parent after the current registry advances', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);

    const prepare = nextActivation(ws, 'prepare');
    const activationPromise = fixture.coordinator.activate(
      activationRequest('activation-parent-pin', 1, ASSEMBLY_B)
    );
    expect(await prepare).toEqual(
      transition('prepare', 'activation-parent-pin', 1, ASSEMBLY_B)
    );
    const commit = nextActivation(ws, 'commit');
    sendActivation(
      ws,
      transition('prepared', 'activation-parent-pin', 1, ASSEMBLY_B)
    );
    await expect(activationPromise).resolves.toMatchObject({
      committed: {
        generation: 2,
        assembly: { assemblyIdentity: ASSEMBLY_B },
        configSnapshot: CONFIG_SNAPSHOT
      }
    });
    await commit;
    sendActivation(ws, registration(2, ASSEMBLY_B));
    await until(
      () =>
        fixture.assemblyRegistry.snapshot().some(
          (replica) =>
            replica.generation === 2 &&
            replica.assemblyIdentity === ASSEMBLY_B &&
            replica.state === 'healthy'
        )
    );

    const [child, childReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-pinned-parent',
      rootValidation.envelope.requestId
    );
    const childValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(child.header);
    if (!childValidation.ok || !('invocation' in childValidation.envelope)) {
      throw new Error('expected pinned-parent derived spawn invocation');
    }
    expect(childReceipt.header.type).toBe('spawn.submit.response');
    expect(childValidation.envelope.routing).toMatchObject({
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
      deployment: deploymentRef(deploymentRevision(ASSEMBLY_A))
    });

    const [grandchild, grandchildReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-pinned-parent-recursive',
      childValidation.envelope.requestId
    );
    const grandchildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(grandchild.header);
    if (
      !grandchildValidation.ok ||
      !('invocation' in grandchildValidation.envelope)
    ) {
      throw new Error('expected recursive pinned-parent spawn invocation');
    }
    expect(grandchildReceipt.header.type).toBe('spawn.submit.response');
    expect(grandchildValidation.envelope.routing).toMatchObject({
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1
    });

    sendEmptyResponseEnd(ws, grandchildValidation.envelope.requestId);
    sendEmptyResponseEnd(ws, childValidation.envelope.requestId);
    sendRootResponseEnd(ws, rootValidation.envelope.requestId);
    await expect(rootResponse).resolves.toMatchObject({ status: 200 });
  });

  it('does not require a current registry source while the authenticated parent is pending', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);

    const registeredConnection =
      fixture.assemblyRegistry.connectionForReplica(RUNTIME_ID);
    if (registeredConnection === undefined) {
      throw new Error('expected registered RuntimeAssembly connection');
    }
    expect(
      fixture.assemblyRegistry.removeRuntimeConnection(registeredConnection)
    ).toBe(RUNTIME_ID);
    expect(fixture.assemblyRegistry.actorSpawnRuntimeControlSource(
      registeredConnection,
      {
        ...runtimeFrameHeaderFixtures['spawn.submit.request'],
        runtimeId: RUNTIME_ID,
        activationIdentity: activation(ASSEMBLY_A, 1),
        serviceId: SERVICE_ID,
        serviceVersion: SERVICE_VERSION,
        serviceProtocolIdentity: SERVICE_PROTOCOL,
        callerRequestId: rootValidation.envelope.requestId
      }
    )).toBeUndefined();

    const [child, receipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-no-current-source',
      rootValidation.envelope.requestId
    );
    const childValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(child.header);
    if (!childValidation.ok || !('invocation' in childValidation.envelope)) {
      throw new Error('expected derived spawn without current registry source');
    }
    expect(receipt.header.type).toBe('spawn.submit.response');

    sendEmptyResponseEnd(ws, childValidation.envelope.requestId);
    sendRootResponseEnd(ws, rootValidation.envelope.requestId);
    await expect(rootResponse).resolves.toMatchObject({ status: 200 });
  });

  it('keeps an accepted spawn alive after its parent is cancelled', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);
    const [child, childReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-detached',
      rootValidation.envelope.requestId
    );
    const childValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(child.header);
    if (!childValidation.ok || !('invocation' in childValidation.envelope)) {
      throw new Error('expected derived spawn invocation');
    }
    expect(childReceipt.header.type).toBe('spawn.submit.response');

    ws.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.cancel',
      requestId: rootValidation.envelope.requestId,
      reason: 'caller_cancel'
    }));
    await until(
      () => fixture.dispatcher.pendingLifecycleCounters().pendingUnary === 1
    );
    sendEmptyResponseEnd(ws, childValidation.envelope.requestId);
    await until(
      () => fixture.dispatcher.pendingLifecycleCounters().pendingUnary === 0
    );
    await expect(rootResponse).resolves.toMatchObject({ status: 503 });
  });

  it('bounds a detached spawn by the parent platform deadline and cleans up', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      mutateTestDispatchBody((body) => {
        body.timeoutMs = 250;
      })
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);

    const [child, childReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-timeout',
      rootValidation.envelope.requestId
    );
    const childValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(child.header);
    if (!childValidation.ok || !('invocation' in childValidation.envelope)) {
      throw new Error('expected derived spawn invocation');
    }
    expect(childReceipt.header.type).toBe('spawn.submit.response');
    if (
      rootValidation.envelope.deadline === undefined ||
      childValidation.envelope.deadline === undefined
    ) {
      throw new Error('expected parent and derived spawn deadlines');
    }
    expect(
      Date.parse(childValidation.envelope.deadline.expiresAt)
    ).toBeLessThanOrEqual(
      Date.parse(rootValidation.envelope.deadline.expiresAt)
    );
    expect(childValidation.envelope.deadline.expiresAt).toBe(
      rootValidation.envelope.deadline.expiresAt
    );

    const childCancel = nextRuntimeFrame(ws, 'request.cancel');
    sendRootResponseEnd(ws, rootValidation.envelope.requestId);
    await expect(rootResponse).resolves.toMatchObject({ status: 200 });
    await until(
      () => fixture.dispatcher.pendingLifecycleCounters().pendingUnary === 1
    );

    const cancel = await childCancel;
    expect(cancel.header.reason).toBe('timeout');
    expect(cancel.header.requestId).toBe(childValidation.envelope.requestId);
    await until(
      () => fixture.dispatcher.pendingLifecycleCounters().pendingUnary === 0
    );
  });

  it('cleans up accepted detached spawns when their runtime connection closes', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);
    const [child, childReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-disconnect',
      rootValidation.envelope.requestId
    );
    expect(child.header.type).toBe('request.start');
    expect(childReceipt.header.type).toBe('spawn.submit.response');
    expect(fixture.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(2);

    ws.close();
    await until(
      () => fixture.dispatcher.pendingLifecycleCounters().pendingUnary === 0
    );
    await expect(rootResponse).resolves.toMatchObject({ status: 503 });
  });

  it('keeps concurrent test case capabilities isolated across derived requests', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const firstResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const firstRoot = await nextRuntimeFrame(ws, 'request.start');
    const secondResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const secondRoot = await nextRuntimeFrame(ws, 'request.start');
    const firstValidation =
      validateRuntimeAssemblyRequestStartFrameHeader(firstRoot.header);
    const secondValidation =
      validateRuntimeAssemblyRequestStartFrameHeader(secondRoot.header);
    if (!firstValidation.ok || !secondValidation.ok) {
      throw new Error('expected two valid test root requests');
    }
    expect(firstValidation.envelope.testCaseCapability).not.toBe(
      secondValidation.envelope.testCaseCapability
    );

    const [firstChild, firstChildReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-case-1',
      firstValidation.envelope.requestId
    );
    expect(firstChildReceipt.header.type).toBe('spawn.submit.response');
    const [secondChild, secondChildReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-case-2',
      secondValidation.envelope.requestId
    );
    expect(secondChildReceipt.header.type).toBe('spawn.submit.response');
    const firstChildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(firstChild.header);
    const secondChildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(secondChild.header);
    if (
      !firstChildValidation.ok ||
      !secondChildValidation.ok ||
      !('invocation' in firstChildValidation.envelope) ||
      !('invocation' in secondChildValidation.envelope)
    ) {
      throw new Error('expected two valid derived spawn requests');
    }
    expect(firstChildValidation.envelope.testCaseCapability).toBe(
      firstValidation.envelope.testCaseCapability
    );
    expect(secondChildValidation.envelope.testCaseCapability).toBe(
      secondValidation.envelope.testCaseCapability
    );

    sendEmptyResponseEnd(ws, firstChildValidation.envelope.requestId);
    sendEmptyResponseEnd(ws, secondChildValidation.envelope.requestId);
    sendRootResponseEnd(ws, firstValidation.envelope.requestId);
    sendRootResponseEnd(ws, secondValidation.envelope.requestId);
    await Promise.all([firstResponse, secondResponse]);
  });

  it('isolates test capabilities and recursive spawn routing across deployments in one assembly generation', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true,
      sharedTestDeployments: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const firstBody = testDispatchBody();
    const secondBody = secondTestDispatchBody();
    const firstResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      firstBody
    );
    const firstRoot = await nextRuntimeFrame(ws, 'request.start');
    const secondResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      secondBody
    );
    const secondRoot = await nextRuntimeFrame(ws, 'request.start');
    const firstValidation =
      validateRuntimeAssemblyRequestStartFrameHeader(firstRoot.header);
    const secondValidation =
      validateRuntimeAssemblyRequestStartFrameHeader(secondRoot.header);
    if (!firstValidation.ok || !secondValidation.ok) {
      throw new Error('expected two valid shared-assembly test roots');
    }

    expect(firstValidation.envelope.routing).toEqual(firstBody.routing);
    expect(secondValidation.envelope.routing).toEqual(secondBody.routing);
    expect(firstValidation.envelope.routing.deployment).toEqual(
      deploymentRef(deploymentRevision(ASSEMBLY_A))
    );
    expect(secondValidation.envelope.routing.deployment).toEqual(
      secondDeploymentRef(ASSEMBLY_A)
    );
    expect(firstValidation.envelope.testCaseCapability).toEqual(expect.any(String));
    expect(secondValidation.envelope.testCaseCapability).toEqual(expect.any(String));
    expect(firstValidation.envelope.testCaseCapability).not.toBe(
      secondValidation.envelope.testCaseCapability
    );

    const [firstChild, firstChildReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-shared-case-1',
      firstValidation.envelope.requestId
    );
    expect(firstChildReceipt.header.type).toBe('spawn.submit.response');
    const [secondChild, secondChildReceipt] = await sendSpawnAndReceive(
      ws,
      'spawn-rpc-shared-case-2',
      secondValidation.envelope.requestId,
      {
        activationIdentity: activationForDeployment(
          ASSEMBLY_A,
          1,
          secondDeploymentRef(ASSEMBLY_A).deploymentRevision
        ),
        serviceId: SECOND_SERVICE_ID,
        serviceProtocolIdentity: SECOND_SERVICE_PROTOCOL
      }
    );
    expect(secondChildReceipt.header.type).toBe('spawn.submit.response');
    const firstChildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(firstChild.header);
    const secondChildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(secondChild.header);
    if (
      !firstChildValidation.ok ||
      !secondChildValidation.ok ||
      !('invocation' in firstChildValidation.envelope) ||
      !('invocation' in secondChildValidation.envelope)
    ) {
      throw new Error('expected exact derived spawn requests for both deployments');
    }
    expect(firstChildValidation.envelope.routing.deployment).toEqual(
      firstValidation.envelope.routing.deployment
    );
    expect(secondChildValidation.envelope.routing.deployment).toEqual(
      secondValidation.envelope.routing.deployment
    );
    expect(firstChildValidation.envelope.testCaseCapability).toBe(
      firstValidation.envelope.testCaseCapability
    );
    expect(secondChildValidation.envelope.testCaseCapability).toBe(
      secondValidation.envelope.testCaseCapability
    );

    const [secondGrandchild, secondGrandchildReceipt] =
      await sendSpawnAndReceive(
        ws,
        'spawn-rpc-shared-case-2-recursive',
        secondChildValidation.envelope.requestId,
        {
          activationIdentity: activationForDeployment(
            ASSEMBLY_A,
            1,
            secondDeploymentRef(ASSEMBLY_A).deploymentRevision
          ),
          serviceId: SECOND_SERVICE_ID,
          serviceProtocolIdentity: SECOND_SERVICE_PROTOCOL
        }
      );
    expect(secondGrandchildReceipt.header.type).toBe('spawn.submit.response');
    const secondGrandchildValidation =
      validateRuntimeAssemblyRequestStartFrameWireHeader(secondGrandchild.header);
    if (
      !secondGrandchildValidation.ok ||
      !('invocation' in secondGrandchildValidation.envelope)
    ) {
      throw new Error('expected recursive spawn for the second deployment');
    }
    expect(secondGrandchildValidation.envelope.routing.deployment).toEqual(
      secondValidation.envelope.routing.deployment
    );
    expect(secondGrandchildValidation.envelope.testCaseCapability).toBe(
      secondValidation.envelope.testCaseCapability
    );

    const crossedEntrypoint = structuredClone(secondBody);
    crossedEntrypoint.routing.gatewayEntryIdentity =
      CURRENT_TEST_GATEWAY_ENTRY_IDENTITY;
    const rejected = await postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      crossedEntrypoint
    );
    expect(rejected.status).toBe(409);

    sendEmptyResponseEnd(
      ws,
      secondGrandchildValidation.envelope.requestId
    );
    sendEmptyResponseEnd(ws, firstChildValidation.envelope.requestId);
    sendEmptyResponseEnd(ws, secondChildValidation.envelope.requestId);
    sendRootResponseEnd(ws, firstValidation.envelope.requestId);
    sendRootResponseEnd(ws, secondValidation.envelope.requestId);
    await Promise.all([firstResponse, secondResponse]);
  });

  it('fails closed for missing parents, mismatched owner facts, and exhausted capacity', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true,
      maxConcurrency: 1
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    sendSpawnSubmit(ws, 'spawn-rpc-missing-parent', 'missing-request');
    await expect(
      nextRuntimeFrame(ws, 'spawn.submit.error')
    ).resolves.toMatchObject({
      header: {
        error: { code: 'SpawnSubmitRejected', status: 502 }
      }
    });

    const rootResponse = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const root = await nextRuntimeFrame(ws, 'request.start');
    const rootValidation = validateRuntimeAssemblyRequestStartFrameHeader(root.header);
    if (!rootValidation.ok) throw new Error(rootValidation.error);

    const otherWs = await openSocket(fixture.url);
    sendCapabilities(otherWs, 'runtime-other');
    sendActivation(otherWs, {
      type: 'register',
      environment: 'test',
      generation: 1,
      assembly: { assemblyIdentity: ASSEMBLY_A },
      configSnapshot: CONFIG_SNAPSHOT,
      replicaId: 'runtime-other'
    });
    await until(
      () => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 2
    );
    sendSpawnSubmit(
      otherWs,
      'spawn-rpc-wrong-socket',
      rootValidation.envelope.requestId,
      {
        runtimeId: 'runtime-other',
        activationIdentity: {
          ...activation(ASSEMBLY_A, 1),
          runtimeReplicaId: 'runtime-other'
        }
      }
    );
    await expect(
      nextRuntimeFrame(otherWs, 'spawn.submit.error')
    ).resolves.toMatchObject({
      header: {
        error: { code: 'SpawnSubmitRejected', status: 502 }
      }
    });

    const ownerFactMismatches = [
      {
        rpcId: 'spawn-rpc-service-mismatch',
        overrides: { serviceId: 'example.com/other' }
      },
      {
        rpcId: 'spawn-rpc-version-mismatch',
        overrides: { serviceVersion: '2.0.0' }
      },
      {
        rpcId: 'spawn-rpc-protocol-mismatch',
        overrides: {
          serviceProtocolIdentity:
            `skiff-service-protocol-v5:sha256:${'e'.repeat(64)}`
        }
      },
      {
        rpcId: 'spawn-rpc-activation-mismatch',
        overrides: {
          activationIdentity: activation(ASSEMBLY_B, 1)
        }
      }
    ] as const;
    for (const mismatch of ownerFactMismatches) {
      sendSpawnSubmit(
        ws,
        mismatch.rpcId,
        rootValidation.envelope.requestId,
        mismatch.overrides
      );
      await expect(
        nextRuntimeFrame(ws, 'spawn.submit.error')
      ).resolves.toMatchObject({
        header: {
          error: { code: 'SpawnSubmitRejected', status: 502 }
        }
      });
    }

    sendSpawnSubmit(
      ws,
      'spawn-rpc-owner-mismatch',
      rootValidation.envelope.requestId,
      { buildId: `skiff-package-build-v10:sha256:${'e'.repeat(64)}` }
    );
    await expect(
      nextRuntimeFrame(ws, 'spawn.submit.error')
    ).resolves.toMatchObject({
      header: {
        error: { code: 'SpawnSubmitRejected', status: 502 }
      }
    });

    sendSpawnSubmit(
      ws,
      'spawn-rpc-missing-build',
      rootValidation.envelope.requestId,
      { buildId: null }
    );
    await expect(
      nextRuntimeFrame(ws, 'spawn.submit.error')
    ).resolves.toMatchObject({
      header: {
        error: { code: 'SpawnSubmitRejected', status: 502 }
      }
    });

    const actorFind = nextRuntimeFrame(ws, 'actor.find.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.find.request'],
      rpcId: 'actor-find-while-request-capacity-full',
      runtimeId: RUNTIME_ID,
      activationIdentity: activation(ASSEMBLY_A, 1),
      actorKey: actorKey()
    }));
    await expect(actorFind).resolves.toMatchObject({
      header: {
        type: 'actor.find.response',
        rpcId: 'actor-find-while-request-capacity-full',
        found: false
      }
    });
    expect(fixture.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(1);

    sendSpawnSubmit(
      ws,
      'spawn-rpc-capacity',
      rootValidation.envelope.requestId
    );
    await expect(
      nextRuntimeFrame(ws, 'spawn.submit.error')
    ).resolves.toMatchObject({
      header: {
        error: { code: 'SpawnSubmitRejected', status: 503 }
      }
    });
    expect(fixture.dispatcher.pendingLifecycleCounters().pendingUnary).toBe(1);

    sendRootResponseEnd(ws, rootValidation.envelope.requestId);
    await expect(rootResponse).resolves.toMatchObject({ status: 200 });
  });

  it('rejects non-exact test control fields and facts before runtime dispatch', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);
    let runtimeRequests = 0;
    const countRuntimeRequests = (data: WebSocket.RawData, isBinary: boolean) => {
      if (
        isBinary &&
        decodeRuntimeFrame(rawDataBuffer(data)).header.type === 'request.start'
      ) {
        runtimeRequests += 1;
      }
    };
    ws.on('message', countRuntimeRequests);

    const invalidBodies = [
      mutateTestDispatchBody((body) => {
        body.contractOperationId =
          `skiff-contract-operation-v1:sha256:${'f'.repeat(64)}`;
      }),
      mutateTestDispatchBody((body) => {
        body.deployment = { serviceId: SERVICE_ID };
      }),
      mutateTestDispatchBody((body) => {
        body.gatewayEntryKey = 'run';
      }),
      mutateTestDispatchBody((body) => {
        body.testEffectDoubles = {};
      }),
      mutateTestDispatchBody((body) => {
        body.testEffectsEnabled = true;
      }),
      mutateTestDispatchBody((body) => {
        body.unknown = true;
      }),
      mutateTestDispatchBody((body) => {
        body.routing.unknown = true;
      }),
      mutateTestDispatchBody((body) => {
        body.routing.ingress.unknown = true;
      }),
      mutateTestDispatchBody((body) => {
        body.httpRequest.unknown = true;
      }),
      mutateTestDispatchBody((body) => {
        body.httpRequest.headers[0].unknown = true;
      }),
      mutateTestDispatchBody((body) => {
        body.kind = 'runtimeAssembly';
      }),
      mutateTestDispatchBody((body) => {
        delete body.kind;
      }),
      mutateTestDispatchBody((body) => {
        body.routing.assemblyIdentity =
          `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
      }),
      mutateTestDispatchBody((body) => {
        body.routing.assemblyGeneration += 1;
      }),
      mutateTestDispatchBody((body) => {
        body.routing.gatewayEntryIdentity =
          `skiff-gateway-entry-v2:sha256:${'f'.repeat(64)}`;
      }),
      mutateTestDispatchBody((body) => {
        body.mode = 'serverStream';
      }),
      mutateTestDispatchBody((body) => {
        body.routing.ingress.path = '/wrong';
      }),
      mutateTestDispatchBody((body) => {
        body.routing.ingress.host = TEST_HOST.toUpperCase();
      }),
      mutateTestDispatchBody((body) => {
        body.routing.ingress.method = 'post';
      }),
      mutateTestDispatchBody((body) => {
        body.httpRequest.url = `http://${TEST_HOST}/wrong`;
      }),
      mutateTestDispatchBody((body) => {
        body.httpRequest.path = '/wrong';
      }),
      mutateTestDispatchBody((body) => {
        body.payloadBase64 = 'bnVsbA';
      }),
      mutateTestDispatchBody((body) => {
        body.timeoutMs = 0;
      }),
      mutateTestDispatchBody((body) => {
        body.timeoutMs = Number.MAX_SAFE_INTEGER + 1;
      })
    ];

    for (const body of invalidBodies) {
      const response = await postControlJson(
        `${fixture.controlUrl}/__skiff/test-dispatch`,
        body
      );
      expect(response.status).toBeGreaterThanOrEqual(400);
    }
    await nextTurn();
    ws.off('message', countRuntimeRequests);
    expect(runtimeRequests).toBe(0);
  });

  it('authorizes active actor/spawn control and preserves the current service protocol identity', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);
    const activationIdentity = activation(ASSEMBLY_A, 1);

    const initialControl = nextBinaryMessage(ws);
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-active-put',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      actorKey: actorKey()
    }, new Uint8Array([1, 2, 3])));
    const initialActivation = decodeBinaryFrame(await initialControl);
    expect(initialActivation.header).toMatchObject({
      type: 'actor.owner.control',
      operation: 'activateInitial',
      targetRuntimeId: RUNTIME_ID,
      bootstrap: {
        encodingVersion: 'skiff-canonical-v1',
        payloadBase64: Buffer.from([1, 2, 3]).toString('base64')
      }
    });
    ws.send(encodeBinaryFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.owner.control.ack',
      runtimeId: RUNTIME_ID,
      requestId: initialActivation.header.requestId,
      operation: 'activateInitial',
      accepted: true
    }, new Uint8Array()));
    const created = await nextRuntimeFrame(ws, 'actor.getOrCreate.response');
    expect(created).toMatchObject({
      header: {
        type: 'actor.getOrCreate.response',
        rpcId: 'actor-active-put',
        actorRef: { serviceId: SERVICE_ID }
      }
    });

    const actorGetAgain = nextRuntimeFrame(ws, 'actor.getOrCreate.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-active-get-again',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      actorKey: actorKey()
    }, new Uint8Array([9])));
    const existing = await actorGetAgain;
    expect(existing.header).toMatchObject({
      type: 'actor.getOrCreate.response',
      actorRef: { epoch: 1 }
    });

    const actorReplace = nextRuntimeFrame(ws, 'actor.replace.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.replace.request'],
      rpcId: 'actor-active-replace',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      actorKey: actorKey()
    }, new Uint8Array([4, 5, 6])));
    const replaced = await actorReplace;
    expect(replaced.header).toMatchObject({
      type: 'actor.replace.response',
      actorRef: { epoch: 2 }
    });
    expect(created.header).toMatchObject({ actorRef: { epoch: 1 } });

  });

  it('rejects every mismatched activation tuple field on the exact assembly sender', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const mismatches = [
      { ...activation(ASSEMBLY_A, 1), assemblyIdentity: ASSEMBLY_B },
      { ...activation(ASSEMBLY_A, 1), generation: 2 },
      { ...activation(ASSEMBLY_A, 1), runtimeReplicaId: 'runtime-other' },
      { ...activation(ASSEMBLY_A, 1), deploymentRevision: 'revision-other' }
    ];
    for (const [index, activationIdentity] of mismatches.entries()) {
      const rpcId = `actor-mismatch-${index}`;
      const response = nextRuntimeFrame(ws, 'actor.find.error');
      ws.send(encodeRuntimeFrame({
        ...runtimeFrameHeaderFixtures['actor.find.request'],
        rpcId,
        runtimeId: RUNTIME_ID,
        activationIdentity,
        actorKey: actorKey()
      }));
      await expect(response).resolves.toMatchObject({
        header: {
          type: 'actor.find.error',
          rpcId,
          error: { code: 'RuntimeActivationMismatch', status: 403 }
        }
      });
    }
  });

  it('allows a pinned draining activation and rejects it after the pin drains', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);
    const oldActivation = activation(ASSEMBLY_A, 1);

    fixture.snapshots.replace({
      environment: 'test',
      generation: 2,
      assembly: { assemblyIdentity: ASSEMBLY_B },
      configSnapshot: CONFIG_SNAPSHOT,
      ingress: new RuntimeAssemblyIngressIndex(assembly(ASSEMBLY_B).gatewayIngress)
    });
    fixture.assemblyRegistry.activate(fixture.snapshots.get());
    fixture.assemblyRegistry.setConnectionPinCounter({
      connectionPinCount: () => 1,
      connectionReleaseAckCount: () => 0
    });

    const pinned = nextRuntimeFrame(ws, 'actor.find.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.find.request'],
      rpcId: 'actor-draining-pinned',
      runtimeId: RUNTIME_ID,
      activationIdentity: oldActivation,
      actorKey: actorKey()
    }));
    await expect(pinned).resolves.toMatchObject({
      header: { type: 'actor.find.response', rpcId: 'actor-draining-pinned' }
    });

    fixture.assemblyRegistry.setConnectionPinCounter({
      connectionPinCount: () => 0,
      connectionReleaseAckCount: () => 0
    });
    const drained = nextRuntimeFrame(ws, 'actor.find.error');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.find.request'],
      rpcId: 'actor-draining-finished',
      runtimeId: RUNTIME_ID,
      activationIdentity: oldActivation,
      actorKey: actorKey()
    }));
    await expect(drained).resolves.toMatchObject({
      header: {
        type: 'actor.find.error',
        rpcId: 'actor-draining-finished',
        error: { code: 'RuntimeActivationMismatch', status: 403 }
      }
    });
  });

  it('uses capability participants across the initial empty and later old registrations', async () => {
    const fixture = await createFixture({
      generation: 0,
      assemblyIdentity: EMPTY_ASSEMBLY
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(0, EMPTY_ASSEMBLY));
    await until(() =>
      fixture.runtimeRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID) &&
      fixture.assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID)
    );

    const firstPrepare = nextActivation(ws, 'prepare');
    const firstActivation = fixture.coordinator.activate(
      activationRequest('activation-first', 0, ASSEMBLY_A)
    );
    expect(await firstPrepare).toEqual(
      transition('prepare', 'activation-first', 0, ASSEMBLY_A)
    );
    const firstCommit = nextActivation(ws, 'commit');
    sendActivation(ws, transition('prepared', 'activation-first', 0, ASSEMBLY_A));
    await expect(firstActivation).resolves.toMatchObject({
      committed: {
        generation: 1,
        assembly: { assemblyIdentity: ASSEMBLY_A },
        configSnapshot: CONFIG_SNAPSHOT
      }
    });
    await firstCommit;
    expect(fixture.assemblyRegistry.snapshot()).toEqual([
      expect.objectContaining({
        generation: 0,
        assemblyIdentity: EMPTY_ASSEMBLY,
        state: 'draining'
      })
    ]);

    const secondPrepare = nextActivation(ws, 'prepare');
    const secondActivation = fixture.coordinator.activate(
      activationRequest('activation-second', 1, ASSEMBLY_B)
    );
    expect(await secondPrepare).toEqual(
      transition('prepare', 'activation-second', 1, ASSEMBLY_B)
    );
    const secondCommit = nextActivation(ws, 'commit');
    sendActivation(ws, transition('prepared', 'activation-second', 1, ASSEMBLY_B));
    await expect(secondActivation).resolves.toMatchObject({
      committed: {
        generation: 2,
        assembly: { assemblyIdentity: ASSEMBLY_B },
        configSnapshot: CONFIG_SNAPSHOT
      }
    });
    await secondCommit;
    expect(ws.readyState).toBe(WebSocket.OPEN);
  });

  it('keeps the complete generic runtime switch on the composite endpoint', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    const runtimeId = runtimeFrameHeaderFixtures['runtime.register'].runtimeId;
    sendCapabilities(ws, runtimeId);
    const registered = nextRuntimeRegisteredAfterInitialBootstrap(ws);
    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['runtime.register']));
    await expect(registered).resolves.toMatchObject({
      header: { type: 'runtime.registered', runtimeId }
    });

    const actorResponse = nextRuntimeFrame(ws, 'actor.find.error');
    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['actor.find.request']));
    await expect(actorResponse).resolves.toMatchObject({
      header: {
        type: 'actor.find.error',
        error: { code: 'RuntimeActivationMismatch', status: 403 }
      }
    });

    const spawnResponse = nextRuntimeFrame(ws, 'spawn.submit.error');
    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['spawn.submit.request']));
    await expect(spawnResponse).resolves.toMatchObject({
      header: { type: 'spawn.submit.error' }
    });

    const serviceRequestResponse = nextRuntimeFrame(ws, 'response.error');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['request.start'],
      caller: {
        kind: 'service',
        target: runtimeFrameHeaderFixtures['request.start'].caller.target
      }
    }));
    const serviceRequestError = await serviceRequestResponse;
    expect(serviceRequestError.header).toMatchObject({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      errorKind: 'control',
      error: { code: 'InProcessServiceCallRequired' }
    });
    expect(serviceRequestError.payloadBytes).toHaveLength(0);

    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['runtime.health']));
    await until(() => fixture.runtimeRegistry.loopRiskRuntimeHealthSnapshot().length === 1);
    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['response.end']));
    ws.send(encodeRuntimeFrame(runtimeFrameHeaderFixtures['request.cancel']));
    await nextTurn();
    expect(ws.readyState).toBe(WebSocket.OPEN);
    expect(fixture.assemblyRegistry.snapshot()).toEqual([]);
  });

  it('keeps capability sessions separate from committed registrations and clears both on disconnect', async () => {
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    await until(() => fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 1);
    expect(fixture.assemblyRegistry.snapshot()).toEqual([]);
    expect(fixture.assemblyRegistry.healthyParticipantReplicaIds()).toEqual([]);

    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.snapshot().length === 1);
    ws.close();
    await waitForClose(ws);
    await until(() => fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 0);
    expect(fixture.assemblyRegistry.snapshot()).toEqual([
      expect.objectContaining({ replicaId: RUNTIME_ID, state: 'disconnected', connected: false })
    ]);
  });

  it('keeps the first capability session when a duplicate live runtime identity connects', async () => {
    const fixture = await createFixture();
    const owner = await openSocket(fixture.url);
    sendCapabilities(owner, RUNTIME_ID);
    await until(() =>
      fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 1
    );

    await expectPolicyClose(
      fixture.url,
      (duplicate) => sendCapabilities(duplicate, RUNTIME_ID)
    );
    expect(owner.readyState).toBe(WebSocket.OPEN);
    expect(fixture.runtimeRegistry.capabilityConnectionsSnapshot()).toEqual([
      expect.objectContaining({ runtimeId: RUNTIME_ID, connected: true })
    ]);
  });

  it('fails closed with 1008 before session mutation for invalid bootstrap frames', async () => {
    const fixture = await createFixture();
    await expectPolicyClose(fixture.url, (ws) => sendActivation(ws, registration(1, ASSEMBLY_A)));
    await expectPolicyClose(fixture.url, (ws) => {
      ws.send(encodeRuntimeFrame({
        ...runtimeFrameHeaderFixtures['runtime.capabilities'],
        runtimeId: RUNTIME_ID
      }, new Uint8Array([1])));
    });
    await expectPolicyClose(fixture.url, (ws) => {
      sendCapabilities(ws, RUNTIME_ID);
      sendActivation(ws, { ...registration(1, ASSEMBLY_A), replicaId: 'runtime-other' });
    });
    await expectPolicyClose(fixture.url, (ws) => {
      sendCapabilities(ws, RUNTIME_ID);
      sendCapabilities(ws, 'runtime-other');
    });
    await expectPolicyClose(fixture.url, (ws) => {
      sendCapabilities(ws, RUNTIME_ID);
      ws.send(encodeAssemblyActivationFrame(
        'routerToRuntime',
        transition('prepare', 'wrong-direction', 1, ASSEMBLY_B)
      ));
    });
    await expectPolicyClose(fixture.url, (ws) => {
      sendCapabilities(ws, RUNTIME_ID);
      ws.send(JSON.stringify(registration(1, ASSEMBLY_A)));
    });
    await expectPolicyClose(fixture.url, (ws) => {
      sendCapabilities(ws, RUNTIME_ID);
      ws.send(encodeBinaryFrame(registration(1, ASSEMBLY_A)));
    });
    await expectPolicyClose(fixture.url, (ws) => {
      sendCapabilities(ws, RUNTIME_ID);
      ws.send(encodeBinaryFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'assembly.activation',
        control: registration(1, ASSEMBLY_A)
      }, new Uint8Array([1])));
    });
    await until(() => fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 0);
    expect(fixture.runtimeRegistry.capabilityConnectionsSnapshot()).toEqual([]);
    expect(fixture.assemblyRegistry.snapshot()).toEqual([]);
  });

  it('admits response.error only through the strict v2 header and payload seam', async () => {
    const fixture = await createFixture();
    const validFixedPayload = Buffer.from(JSON.stringify({
      kind: 'internalError',
      payload: {
        message: 'Internal service error',
        traceId: 'trace-endpoint-fixed',
        errorId: 'error-endpoint-fixed'
      }
    }), 'utf8');
    const invalidFrames: Array<{
      name: string;
      header: Record<string, unknown>;
      payloadBytes: Uint8Array;
    }> = [
      {
        name: 'legacy v1 control',
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: 'legacy-v1',
          error: { code: 'LegacyError', message: 'legacy response.error' }
        },
        payloadBytes: new Uint8Array()
      },
      {
        name: 'mixed fixed and generic fields',
        header: {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: 'mixed-fixed',
          errorKind: 'fixedService',
          error: { code: 'MixedError', message: 'must not be admitted' }
        },
        payloadBytes: validFixedPayload
      },
      {
        name: 'fixed with empty payload',
        header: {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: 'fixed-empty',
          errorKind: 'fixedService'
        },
        payloadBytes: new Uint8Array()
      },
      {
        name: 'control with non-empty payload',
        header: {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: 'control-non-empty',
          errorKind: 'control',
          error: { code: 'ControlError', message: 'control payload must be empty' }
        },
        payloadBytes: new Uint8Array([1])
      },
      {
        name: 'fixed with malformed envelope',
        header: {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: 'fixed-malformed',
          errorKind: 'fixedService'
        },
        payloadBytes: Buffer.from('{', 'utf8')
      }
    ];

    for (const invalid of invalidFrames) {
      await expectPolicyClose(
        fixture.url,
        (ws) => {
          sendCapabilities(ws, RUNTIME_ID);
          ws.send(encodeBinaryFrame(invalid.header, invalid.payloadBytes));
        },
        invalid.name
      );
      await until(
        () => fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 0
      );
    }
    expect(fixture.assemblyRegistry.snapshot()).toEqual([]);
  });
});

interface CompositeEndpointFixture {
  assemblyRegistry: AssemblyRuntimeRegistry;
  controlUrl: string;
  coordinator: AssemblyActivationCoordinator;
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  runtimeRegistry: RuntimeRegistry;
  snapshots: RouterActiveAssemblySnapshotStore;
  url: string;
  close(): Promise<void>;
}

async function createFixture(
  initial: {
    generation: number;
    assemblyIdentity: string;
    testGateway?: boolean;
    sharedTestDeployments?: boolean;
    maxConcurrency?: number;
    activationTimeoutMs?: number;
  } = { generation: 1, assemblyIdentity: ASSEMBLY_A }
): Promise<CompositeEndpointFixture> {
  const testGateway = initial.testGateway ?? false;
  const snapshots = new RouterActiveAssemblySnapshotStore();
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const actorManager = new ActorManager();
  const actorSpawnControl = new ActorSpawnRuntimeControl({ actorManager });
  const actorDisconnect = new ActorRuntimeDisconnectController(actorManager);
  let runtimeRegistry!: RuntimeRegistry;
  const actorGetCreateControl = new ActorGetCreateActivationCoordinator({
    actorManager,
    runtimeDirectory: {
      actorRuntimeCandidates: (serviceId) =>
        assemblyRegistry.actorRuntimeCandidates(serviceId),
      runtimeConnection: (runtimeId) => {
        const ws = assemblyRegistry.connectionForReplica(runtimeId);
        return ws === undefined ? undefined : { runtimeId, ws };
      },
      runtimeIdForConnection: (ws) => assemblyRegistry.replicaIdForConnection(ws),
      runtimeConnectionFenceForConnection: (ws) =>
        runtimeRegistry.runtimeConnectionFenceForConnection(ws),
    },
    disconnectController: actorDisconnect,
    send: (ws, bytes) => ws.send(bytes),
    ...(initial.activationTimeoutMs === undefined
      ? {}
      : { activationTimeoutMs: initial.activationTimeoutMs }),
  });
  runtimeRegistry = new RuntimeRegistry({
    actorSpawnControl,
    actorGetCreateControl,
  });
  const endpoint = new RuntimeEndpoint({
    registry: runtimeRegistry,
    actorRuntimeDisconnect: actorDisconnect,
    actorGetCreateControl,
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
      http: { maxResponseBytes: 67108864 },
      activation: {
        environment: 'test',
        generation: initial.generation,
        assembly: { assemblyIdentity: initial.assemblyIdentity },
        configSnapshot: CONFIG_SNAPSHOT
      }
    }
  });
  const coordinator = new AssemblyActivationCoordinator({
    environment: 'test',
    stateStore: new MemoryAssemblyActivationStateStore(initialActivationState({
      environment: 'test',
      generation: initial.generation,
      assemblyIdentity: initial.assemblyIdentity, configSnapshotId: 'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    })),
    assemblyLoader: new MemoryRuntimeAssemblySnapshotLoader([
      assembly(EMPTY_ASSEMBLY),
      assembly(
        ASSEMBLY_A,
        testGateway,
        initial.sharedTestDeployments
      ),
      assembly(
        ASSEMBLY_B,
        testGateway,
        initial.sharedTestDeployments
      ),
      assembly(
        ASSEMBLY_C,
        testGateway,
        initial.sharedTestDeployments
      )
    ]),
    snapshots,
    registry: assemblyRegistry,
    participants: runtimeRegistry,
    controlSender: endpoint,
    prepareTimeoutMs: 1000
  });
  endpoint.setCoordinator(coordinator);
  await coordinator.initialize();
  const dispatcher = new RuntimeDispatcher({
    registry: assemblyRegistry,
    frameSender: endpoint,
    maxConcurrency: initial.maxConcurrency ?? 64
  });
  endpoint.setDispatcher(dispatcher);
  const controlPlane = new AssemblyControlPlane({
    coordinator,
    dispatcher,
    registry: assemblyRegistry,
    runtimeRegistry,
    snapshots
  });
  const listening = await endpoint.listen({ controlPlane, port: 0 });
  const fixture = {
    assemblyRegistry,
    controlUrl: `http://${listening.host}:${listening.port}`,
    coordinator,
    dispatcher,
    endpoint,
    runtimeRegistry,
    snapshots,
    url: listening.url,
    close: () => endpoint.close()
  };
  fixtures.push(fixture);
  return fixture;
}

function sendCapabilities(ws: WebSocket, runtimeId: string): void {
  ws.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId
  }));
}

function sendActivation(ws: WebSocket, control: AssemblyActivationControl): void {
  ws.send(encodeAssemblyActivationFrame('runtimeToRouter', control));
}

function registration(generation: number, assemblyIdentity: string): AssemblyActivationControl {
  return {
    type: 'register',
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    configSnapshot: CONFIG_SNAPSHOT,
    replicaId: RUNTIME_ID
  };
}

function activationRequest(activationId: string, expectedGeneration: number, assemblyIdentity: string) {
  return {
    schemaVersion: 'skiff-assembly-activation-request-v2' as const,
    environment: 'test',
    activationId,
    expectedGeneration,
    assembly: { assemblyIdentity },
    configSnapshot: CONFIG_SNAPSHOT
  };
}

function transition(
  type: 'prepare' | 'prepared' | 'reject' | 'commit' | 'abort',
  activationId: string,
  expectedGeneration: number,
  assemblyIdentity: string
): AssemblyActivationControl {
  const base = {
    environment: 'test',
    activationId,
    expectedGeneration,
    candidateGeneration: expectedGeneration + 1,
    assembly: { assemblyIdentity },
    configSnapshot: CONFIG_SNAPSHOT,
    replicaId: RUNTIME_ID
  };
  return type === 'reject'
    ? { ...base, type, reason: 'admission' }
    : { ...base, type };
}

function assembly(
  assemblyIdentity: string,
  includeTestGateway = false,
  includeSecondTestDeployment = false
): LoadedRuntimeAssembly {
  const revision = deploymentRevision(assemblyIdentity);
  const deployment = deploymentRef(revision);
  const secondDeployment = secondDeploymentRef(assemblyIdentity);
  const deployments = includeSecondTestDeployment
    ? [deployment, secondDeployment]
    : [deployment];
  return {
    schemaVersion: 'skiff-runtime-assembly-v3',
    assemblyIdentity,
    resolvedDeployments:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : deployments,
    resolvedContracts:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : [
            {
              serviceId: SERVICE_ID,
              contractVersion: SERVICE_VERSION,
              serviceProtocolIdentity: SERVICE_PROTOCOL
            },
            ...(includeSecondTestDeployment
              ? [{
                  serviceId: SECOND_SERVICE_ID,
                  contractVersion: SERVICE_VERSION,
                  serviceProtocolIdentity: SECOND_SERVICE_PROTOCOL
                }]
              : [])
          ],
    deploymentRuntimeBindings:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : deployments.map((current) => ({
            deployment: current,
            packageBuildId: PACKAGE_BUILD_ID
          })),
    gatewayIngress:
      assemblyIdentity === EMPTY_ASSEMBLY || !includeTestGateway
        ? []
        : [
            {
              selector: {
                protocol: 'http',
                method: 'POST',
                path: TEST_PATH
              },
              deployment,
              gatewayEntryKey: 'run',
              gatewayEntryIdentity: CURRENT_TEST_GATEWAY_ENTRY_IDENTITY,
              adapterKind: 'typedJson',
              operationMode: 'unary'
            },
            ...(includeSecondTestDeployment
              ? [{
                  selector: {
                    protocol: 'http' as const,
                    method: 'POST',
                    path: SECOND_TEST_PATH
                  },
                  deployment: secondDeployment,
                  gatewayEntryKey: 'run',
                  gatewayEntryIdentity: SECOND_TEST_GATEWAY_ENTRY_IDENTITY,
                  adapterKind: 'typedJson' as const,
                  operationMode: 'unary' as const
                }]
              : [])
          ]
  };
}

function deploymentRef(deploymentRevision: string) {
  return {
    serviceId: SERVICE_ID,
    contractVersion: SERVICE_VERSION,
    deploymentRevision,
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v4:sha256:${'e'.repeat(64)}`
  };
}

function secondDeploymentRef(assemblyIdentity: string) {
  return {
    serviceId: SECOND_SERVICE_ID,
    contractVersion: SERVICE_VERSION,
    deploymentRevision: `case-two-${deploymentRevision(assemblyIdentity)}`,
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v4:sha256:${'f'.repeat(64)}`
  };
}

function deploymentRevision(assemblyIdentity: string): string {
  return assemblyIdentity === ASSEMBLY_A ? 'revision-a' : 'revision-b';
}

function activation(
  assemblyIdentity: string,
  generation: number
) {
  return activationForDeployment(
    assemblyIdentity,
    generation,
    deploymentRevision(assemblyIdentity)
  );
}

function activationForDeployment(
  assemblyIdentity: string,
  generation: number,
  revision: string
) {
  return {
    assemblyIdentity,
    generation,
    runtimeReplicaId: RUNTIME_ID,
    deploymentRevision: revision
  };
}

function actorKey() {
  return {
    serviceId: SERVICE_ID,
    actorTypeIdentity: 'actor.example.ThreadActor',
    actorIdTypeIdentity: 'type.example.ThreadId',
    actorIdEncodingVersion: 'json-v1',
    canonicalActorIdKeyBytesBase64:
      Buffer.from('"thread-1"').toString('base64')
  };
}

function sendSpawnSubmit(
  ws: WebSocket,
  rpcId: string,
  callerRequestId: string,
  overrides: {
    buildId?: string | null;
    runtimeId?: string;
    activationIdentity?: ReturnType<typeof activation>;
    serviceId?: string;
    serviceVersion?: string;
    serviceProtocolIdentity?: string;
  } = {}
): void {
  const {
    buildId: _fixtureBuildId,
    ...fixture
  } = runtimeFrameHeaderFixtures['spawn.submit.request'];
  ws.send(encodeRuntimeFrame({
    ...fixture,
    rpcId,
    runtimeId: overrides.runtimeId ?? RUNTIME_ID,
    activationIdentity:
      overrides.activationIdentity ?? activation(ASSEMBLY_A, 1),
    serviceId: overrides.serviceId ?? SERVICE_ID,
    serviceVersion: overrides.serviceVersion ?? SERVICE_VERSION,
    serviceProtocolIdentity:
      overrides.serviceProtocolIdentity ?? SERVICE_PROTOCOL,
    target: TARGET,
    ...(overrides.buildId === null
      ? {}
      : { buildId: overrides.buildId ?? PACKAGE_BUILD_ID }),
    callerRequestId
  }, new Uint8Array([7, 8])));
}

async function sendSpawnAndReceive(
  ws: WebSocket,
  rpcId: string,
  callerRequestId: string,
  overrides: {
    buildId?: string | null;
    runtimeId?: string;
    activationIdentity?: ReturnType<typeof activation>;
    serviceId?: string;
    serviceVersion?: string;
    serviceProtocolIdentity?: string;
  } = {}
): Promise<[RuntimeBinaryFrame, RuntimeBinaryFrame]> {
  const received = nextBinaryMessages(ws, 2);
  sendSpawnSubmit(ws, rpcId, callerRequestId, overrides);
  const frames = (await received).map((data) => decodeRuntimeFrame(data));
  expect(frames[0]?.header.type).toBe('request.start');
  expect(frames[1]?.header.type).toBe('spawn.submit.response');
  return [frames[0]!, frames[1]!];
}

function sendEmptyResponseEnd(ws: WebSocket, requestId: string): void {
  ws.send(encodeRuntimeFrame({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId,
    payloadPresent: false
  }));
}

function sendRootResponseEnd(ws: WebSocket, requestId: string): void {
  ws.send(encodeRuntimeFrame({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId,
    payloadPresent: true,
    httpResponse: {
      status: 200,
      headers: [
        {
          name: 'content-type',
          value: 'application/json; charset=utf-8'
        }
      ]
    }
  }, Buffer.from('null', 'utf8')));
}

function identity(character: string): string {
  return `skiff-runtime-assembly-v3:sha256:${character.repeat(64)}`;
}

function testDispatchBody() {
  return {
    kind: 'test',
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
      deployment: deploymentRef(deploymentRevision(ASSEMBLY_A)),
      gatewayEntryIdentity: CURRENT_TEST_GATEWAY_ENTRY_IDENTITY,
      ingress: {
        protocol: 'http',
        method: 'POST',
        path: TEST_PATH
      }
    },
    mode: 'unary',
    httpRequest: {
      method: 'POST',
      url: `http://${TEST_HOST}${TEST_PATH}`,
      path: TEST_PATH,
      query: [],
      headers: [
        {
          name: 'content-type',
          value: 'application/json'
        }
      ]
    },
    payloadBase64: Buffer.from('null', 'utf8').toString('base64'),
    timeoutMs: 1_000
  };
}

function secondTestDispatchBody() {
  return {
    kind: 'test',
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY_A,
      assemblyGeneration: 1,
      deployment: secondDeploymentRef(ASSEMBLY_A),
      gatewayEntryIdentity: SECOND_TEST_GATEWAY_ENTRY_IDENTITY,
      ingress: {
        protocol: 'http',
        method: 'POST',
        path: SECOND_TEST_PATH
      }
    },
    mode: 'unary',
    httpRequest: {
      method: 'POST',
      url: `http://${TEST_HOST}${SECOND_TEST_PATH}`,
      path: SECOND_TEST_PATH,
      query: [],
      headers: [
        {
          name: 'content-type',
          value: 'application/json'
        }
      ]
    },
    payloadBase64: Buffer.from('null', 'utf8').toString('base64'),
    timeoutMs: 1_000
  };
}

function mutateTestDispatchBody(
  change: (body: Record<string, any>) => void
): Record<string, unknown> {
  const body = structuredClone(testDispatchBody()) as unknown as Record<
    string,
    any
  >;
  change(body);
  if (
    typeof body.timeoutMs === 'number' &&
    Number.isSafeInteger(body.timeoutMs) &&
    body.timeoutMs > 0
  ) {
    body.timeoutMs = Math.min(body.timeoutMs, 250);
  }
  return body;
}

async function postControlJson(
  url: string,
  body: unknown
): Promise<{ status: number; body: any }> {
  const response = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body)
  });
  return {
    status: response.status,
    body: await response.json()
  };
}

async function openSocket(url: string): Promise<WebSocket> {
  const ws = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
  return ws;
}

async function nextActivation(
  ws: WebSocket,
  type: AssemblyActivationControl['type']
): Promise<AssemblyActivationControl> {
  const data = await nextBinaryMessage(ws);
  const control = decodeAssemblyActivationFrame('routerToRuntime', data);
  expect(control.type).toBe(type);
  return control;
}

async function nextRuntimeFrame(ws: WebSocket, type: string): Promise<RuntimeBinaryFrame> {
  const data = await nextBinaryMessage(ws);
  const frame = decodeRuntimeFrame(data);
  expect(frame.header.type).toBe(type);
  return frame;
}

async function nextRuntimeRegisteredAfterInitialBootstrap(
  ws: WebSocket
): Promise<RuntimeBinaryFrame> {
  return await new Promise<RuntimeBinaryFrame>((resolve, reject) => {
    let skippedInitialBootstrap = false;
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error('timed out waiting for binary frame'));
    }, 1000);
    const onMessage = (data: WebSocket.RawData, isBinary: boolean) => {
      if (!isBinary) {
        cleanup();
        reject(new Error('expected binary runtime frame'));
        return;
      }
      try {
        const frame = decodeRuntimeFrame(rawDataBuffer(data));
        if (
          !skippedInitialBootstrap &&
          frame.header.type === 'router.bootstrap'
        ) {
          skippedInitialBootstrap = true;
          return;
        }
        expect(frame.header.type).toBe('runtime.registered');
        cleanup();
        resolve(frame);
      } catch (error) {
        cleanup();
        reject(error);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

async function nextBinaryMessage(ws: WebSocket): Promise<Buffer> {
  return await new Promise<Buffer>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for binary frame')), 1000);
    ws.once('message', (data, isBinary) => {
      clearTimeout(timeout);
      if (!isBinary) {
        reject(new Error('expected binary runtime frame'));
        return;
      }
      resolve(rawDataBuffer(data));
    });
  });
}

async function nextBinaryMessages(ws: WebSocket, count: number): Promise<Buffer[]> {
  return await new Promise<Buffer[]>((resolve, reject) => {
    const messages: Buffer[] = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error('timed out waiting for binary frames'));
    }, 1000);
    const onMessage = (data: WebSocket.RawData, isBinary: boolean) => {
      if (!isBinary) {
        cleanup();
        reject(new Error('expected binary runtime frame'));
        return;
      }
      messages.push(rawDataBuffer(data));
      if (messages.length === count) {
        cleanup();
        resolve(messages);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

async function expectPolicyClose(
  url: string,
  send: (ws: WebSocket) => void,
  label?: string
): Promise<void> {
  const ws = await openSocket(url);
  const closed = waitForClose(ws);
  send(ws);
  const [code] = await closed;
  expect(code, label).toBe(1008);
}

async function waitForClose(ws: WebSocket): Promise<[number, Buffer]> {
  return await new Promise<[number, Buffer]>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for socket close')), 1000);
    ws.once('close', (code, reason) => {
      clearTimeout(timeout);
      resolve([code, Buffer.from(reason)]);
    });
  });
}

function rawDataBuffer(data: WebSocket.RawData): Buffer {
  if (Array.isArray(data)) {
    return Buffer.concat(data);
  }
  if (data instanceof ArrayBuffer) {
    return Buffer.from(new Uint8Array(data));
  }
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
}

async function until(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) {
      return;
    }
    await nextTurn();
  }
  throw new Error('condition was not reached');
}

async function nextTurn(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}
