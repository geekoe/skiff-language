import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import {
  decodeAssemblyActivationFrame,
  encodeAssemblyActivationFrame
} from '../src/protocol/assemblyActivationFrame.js';
import type { AssemblyActivationControl } from '../src/protocol/assemblyActivationProtocol.js';
import {
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
  validateRuntimeAssemblyRequestStartFrameHeader
} from '../src/protocol/runtimeProtocol.js';
import { AssemblyActivationCoordinator } from '../src/router/assemblyActivationCoordinator.js';
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
const SERVICE_VERSION = '1.0.0';
const SERVICE_PROTOCOL =
  `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`;
const BUILD_ID = `skiff-service-build-v1:sha256:${'d'.repeat(64)}`;
const TARGET = 'function:service.example~actors.ActorApi.spawn';
const SPAWN_COMPATIBILITY = `${SERVICE_VERSION}:${SERVICE_PROTOCOL}:${TARGET}`;
const CURRENT_TEST_GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'9'.repeat(64)}`;
const TEST_HOST = 'case-0.package-test.skiff.localhost';
const TEST_PATH = '/__skiff/package-test/0';
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
      committed: { generation: 2, assembly: { assemblyIdentity: ASSEMBLY_B } }
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
        capabilityConnections: unknown[];
        replicas: unknown[];
      };
    });
    expect(health.capabilityConnections).toEqual([
      expect.objectContaining({ runtimeId: RUNTIME_ID, connected: true })
    ]);
    expect(health.replicas).toEqual([
      expect.objectContaining({
        replicaId: RUNTIME_ID,
        generation: 2,
        assemblyIdentity: ASSEMBLY_B,
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

    const actorGetOrCreate = nextRuntimeFrame(ws, 'actor.getOrCreate.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
      rpcId: 'actor-active-put',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      actorKey: actorKey()
    }, new Uint8Array([1, 2, 3])));
    const created = await actorGetOrCreate;
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

    const submit = nextRuntimeFrame(ws, 'spawn.submit.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.submit.request'],
      rpcId: 'spawn-active-submit',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      serviceId: SERVICE_ID,
      serviceVersion: SERVICE_VERSION,
      serviceProtocolIdentity: SERVICE_PROTOCOL,
      target: TARGET,
      buildId: BUILD_ID,
      spawnId: 'spawn-active-1'
    }, new Uint8Array([7, 8])));
    await expect(submit).resolves.toMatchObject({
      header: { type: 'spawn.submit.response', spawnId: 'spawn-active-1' }
    });

    const claim = nextRuntimeFrame(ws, 'spawn.claim.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.claim.request'],
      rpcId: 'spawn-active-claim',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      serviceId: SERVICE_ID,
      serviceVersion: SERVICE_VERSION,
      serviceProtocolIdentity: SERVICE_PROTOCOL,
      supportedTargets: [TARGET],
      supportedSpawnCompatibilityKeys: [SPAWN_COMPATIBILITY],
      buildId: BUILD_ID
    }));
    const claimed = await claim;
    expect(claimed).toMatchObject({
      header: {
        type: 'spawn.claim.response',
        claimed: true,
        item: {
          serviceId: SERVICE_ID,
          buildId: BUILD_ID,
          serviceProtocolIdentity: SERVICE_PROTOCOL,
          activationIdentity
        }
      }
    });
    expect([...claimed.payloadBytes]).toEqual([7, 8]);
    if (
      claimed.header.type !== 'spawn.claim.response' ||
      claimed.header.item === undefined
    ) {
      throw new Error('expected a claimed spawn item');
    }

    const renew = nextRuntimeFrame(ws, 'spawn.renew.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.renew.request'],
      rpcId: 'spawn-active-renew',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      itemId: claimed.header.item.itemId,
      leaseId: claimed.header.item.leaseId
    }));
    await expect(renew).resolves.toMatchObject({
      header: {
        type: 'spawn.renew.response',
        rpcId: 'spawn-active-renew',
        renewed: true
      }
    });

    const wrongComplete = nextRuntimeFrame(ws, 'spawn.complete.error');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.complete.request'],
      rpcId: 'spawn-wrong-complete',
      runtimeId: RUNTIME_ID,
      activationIdentity: {
        ...activationIdentity,
        deploymentRevision: 'revision-other'
      },
      itemId: claimed.header.item.itemId,
      leaseId: claimed.header.item.leaseId
    }));
    await expect(wrongComplete).resolves.toMatchObject({
      header: {
        type: 'spawn.complete.error',
        rpcId: 'spawn-wrong-complete',
        error: { code: 'RuntimeActivationMismatch', status: 403 }
      }
    });

    const complete = nextRuntimeFrame(ws, 'spawn.complete.response');
    ws.send(encodeRuntimeFrame({
      ...runtimeFrameHeaderFixtures['spawn.complete.request'],
      rpcId: 'spawn-active-complete',
      runtimeId: RUNTIME_ID,
      activationIdentity,
      itemId: claimed.header.item.itemId,
      leaseId: claimed.header.item.leaseId
    }));
    await expect(complete).resolves.toMatchObject({
      header: {
        type: 'spawn.complete.response',
        rpcId: 'spawn-active-complete',
        status: 'completed'
      }
    });
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
      committed: { generation: 1, assembly: { assemblyIdentity: ASSEMBLY_A } }
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
      committed: { generation: 2, assembly: { assemblyIdentity: ASSEMBLY_B } }
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
  } = { generation: 1, assemblyIdentity: ASSEMBLY_A }
): Promise<CompositeEndpointFixture> {
  const testGateway = initial.testGateway ?? false;
  const snapshots = new RouterActiveAssemblySnapshotStore();
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const runtimeRegistry = new RuntimeRegistry();
  const endpoint = new RuntimeEndpoint({
    registry: runtimeRegistry,
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
      http: { maxResponseBytes: 67108864 }
    }
  });
  const coordinator = new AssemblyActivationCoordinator({
    environment: 'test',
    stateStore: new MemoryAssemblyActivationStateStore(initialActivationState({
      environment: 'test',
      generation: initial.generation,
      assemblyIdentity: initial.assemblyIdentity
    })),
    assemblyLoader: new MemoryRuntimeAssemblySnapshotLoader([
      assembly(EMPTY_ASSEMBLY),
      assembly(ASSEMBLY_A, testGateway),
      assembly(ASSEMBLY_B, testGateway),
      assembly(ASSEMBLY_C, testGateway)
    ]),
    snapshots,
    registry: assemblyRegistry,
    participants: runtimeRegistry,
    controlSender: endpoint,
    prepareTimeoutMs: 1000
  });
  endpoint.setCoordinator(coordinator);
  await coordinator.initialize();
  const dispatcher = new RuntimeDispatcher({ registry: assemblyRegistry, frameSender: endpoint });
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
    replicaId: RUNTIME_ID
  };
}

function activationRequest(activationId: string, expectedGeneration: number, assemblyIdentity: string) {
  return {
    schemaVersion: 'skiff-assembly-activation-request-v1' as const,
    environment: 'test',
    activationId,
    expectedGeneration,
    assembly: { assemblyIdentity }
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
    replicaId: RUNTIME_ID
  };
  return type === 'reject'
    ? { ...base, type, reason: 'admission' }
    : { ...base, type };
}

function assembly(
  assemblyIdentity: string,
  includeTestGateway = false
): LoadedRuntimeAssembly {
  const revision = deploymentRevision(assemblyIdentity);
  const deployment = deploymentRef(revision);
  return {
    schemaVersion: 'skiff-runtime-assembly-v3',
    assemblyIdentity,
    resolvedDeployments:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : [deployment],
    resolvedContracts:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : [{
          serviceId: SERVICE_ID,
          contractVersion: SERVICE_VERSION,
          serviceProtocolIdentity: SERVICE_PROTOCOL
        }],
    gatewayIngress:
      assemblyIdentity === EMPTY_ASSEMBLY || !includeTestGateway
        ? []
        : [{
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
          }]
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

function deploymentRevision(assemblyIdentity: string): string {
  return assemblyIdentity === ASSEMBLY_A ? 'revision-a' : 'revision-b';
}

function activation(
  assemblyIdentity: string,
  generation: number
) {
  return {
    assemblyIdentity,
    generation,
    runtimeReplicaId: RUNTIME_ID,
    deploymentRevision: deploymentRevision(assemblyIdentity)
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
    body.timeoutMs = Math.min(body.timeoutMs, 25);
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
