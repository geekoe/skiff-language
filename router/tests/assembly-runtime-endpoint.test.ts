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
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RuntimeBinaryFrame
} from '../src/protocol/envelope.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
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
  type LoadedRuntimeAssembly,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY_A = identity('a');
const ASSEMBLY_B = identity('b');
const ASSEMBLY_C = identity('c');
const EMPTY_ASSEMBLY =
  'skiff-runtime-assembly-v1:sha256:4176e39122928fcf47db987c34884f2f7ab4a1833c502a33bb6fd0c861a5acf6';
const RUNTIME_ID = 'runtime-assembly-a';
const SERVICE_ID = 'example.com/actors';
const SERVICE_VERSION = '1.0.0';
const SERVICE_PROTOCOL =
  `skiff-service-protocol-v3:sha256:${'c'.repeat(64)}`;
const BUILD_ID = `skiff-service-build-v1:sha256:${'d'.repeat(64)}`;
const TARGET = 'function:service.example~actors.ActorApi.spawn';
const SPAWN_COMPATIBILITY = `${SERVICE_VERSION}:${SERVICE_PROTOCOL}:${TARGET}`;
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

  it('authorizes active actor/spawn control and round-trips structured activation identity', async () => {
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
      ingress: new RuntimeAssemblyIngressIndex(assembly(ASSEMBLY_B).globalIngress)
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
    const registered = nextRuntimeFrame(ws, 'runtime.registered');
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
    await expect(serviceRequestResponse).resolves.toMatchObject({
      header: {
        type: 'response.error',
        error: { code: 'InProcessServiceCallRequired' }
      }
    });

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
  initial = { generation: 1, assemblyIdentity: ASSEMBLY_A }
): Promise<CompositeEndpointFixture> {
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
      assembly(ASSEMBLY_A),
      assembly(ASSEMBLY_B),
      assembly(ASSEMBLY_C)
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

function assembly(assemblyIdentity: string): LoadedRuntimeAssembly {
  const revision = deploymentRevision(assemblyIdentity);
  return {
    schemaVersion: 'skiff-runtime-assembly-v1',
    assemblyIdentity,
    resolvedDeployments:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : [ingressBinding(revision).deployment],
    resolvedContracts:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : [ingressBinding(revision).contract],
    globalIngress:
      assemblyIdentity === EMPTY_ASSEMBLY
        ? []
        : [ingressBinding(revision)]
  };
}

function ingressBinding(deploymentRevision: string): RuntimeAssemblyIngressBinding {
  return {
    selector: {
      protocol: 'http',
      host: 'actors.localhost',
      method: 'POST',
      path: '/actors'
    },
    deployment: {
      serviceId: SERVICE_ID,
      contractVersion: SERVICE_VERSION,
      deploymentRevision,
      deploymentArtifactIdentity:
        `skiff-deployment-artifact-v1:sha256:${'e'.repeat(64)}`
    },
    contract: {
      serviceId: SERVICE_ID,
      contractVersion: SERVICE_VERSION,
      serviceProtocolIdentity: SERVICE_PROTOCOL
    },
    operationMode: 'unary',
    contractOperationId:
      `skiff-contract-operation-v1:sha256:${'f'.repeat(64)}`
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
  return `skiff-runtime-assembly-v1:sha256:${character.repeat(64)}`;
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

async function expectPolicyClose(url: string, send: (ws: WebSocket) => void): Promise<void> {
  const ws = await openSocket(url);
  const closed = waitForClose(ws);
  send(ws);
  const [code] = await closed;
  expect(code).toBe(1008);
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
