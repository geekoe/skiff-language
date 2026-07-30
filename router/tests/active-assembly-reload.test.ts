import WebSocket from 'ws';
import { describe, expect, it, vi } from 'vitest';

import type { AssemblyActivationControl } from '../src/protocol/assemblyActivationProtocol.js';
import { AssemblyActivationCoordinator } from '../src/router/assemblyActivationCoordinator.js';
import {
  MemoryAssemblyActivationStateStore,
  initialActivationState,
  type AssemblyActivationStateStore
} from '../src/router/assemblyActivationStateStore.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import {
  MemoryRuntimeAssemblySnapshotLoader,
  RouterActiveAssemblySnapshotStore,
  type LoadedRuntimeAssembly
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY_A = identity('a');
const ASSEMBLY_B = identity('b');
const ASSEMBLY_C = identity('c');

describe('active RuntimeAssembly activation transaction', () => {
  it('keeps preparing beyond a 7s request budget and aborts only at the activation budget', async () => {
    vi.useFakeTimers();
    try {
      const snapshots = new RouterActiveAssemblySnapshotStore();
      const registry = new AssemblyRuntimeRegistry(snapshots);
      const stateStore = new MemoryAssemblyActivationStateStore(
        activationState({
          environment: 'test',
          generation: 1,
          assemblyIdentity: ASSEMBLY_A
        })
      );
      const controls: AssemblyActivationControl[] = [];
      const coordinator = new AssemblyActivationCoordinator({
        environment: 'test',
        stateStore,
        assemblyLoader: new MemoryRuntimeAssemblySnapshotLoader([
          assembly(ASSEMBLY_A),
          assembly(ASSEMBLY_B)
        ]),
        snapshots,
        registry,
        participants: registry,
        controlSender: {
          sendAssemblyControl: (_ws, control) => controls.push(control)
        },
        prepareTimeoutMs: 120_000
      });
      await coordinator.initialize();
      const runtime = fakeSocket();
      register(registry, runtime, 'replica-a', 1, ASSEMBLY_A);

      let settled = false;
      const activation = coordinator.activate({
        schemaVersion: 'skiff-assembly-activation-request-v2',
        environment: 'test',
        activationId: 'activation-independent-budget',
        expectedGeneration: 1,
        assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
      }).finally(() => {
        settled = true;
      });
      const timeoutResult = expect(activation).rejects.toThrow(
        /assembly activation prepare timed out/
      );
      await vi.advanceTimersByTimeAsync(20_001);
      expect(controlsOfType(controls, 'prepare')).toHaveLength(1);
      expect(settled).toBe(false);
      expect(coordinator.activationState().pending).not.toBeNull();

      await vi.advanceTimersByTimeAsync(99_998);
      expect(settled).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      await timeoutResult;
      expect(controlsOfType(controls, 'abort')).toHaveLength(1);
      expect((await stateStore.read('test')).pending).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('commits only after every frozen connected replica returns the exact staged ACK', async () => {
    const fixture = await coordinatorFixture();
    const runtimeA = fakeSocket();
    const runtimeB = fakeSocket();
    register(fixture.registry, runtimeA, 'replica-a', 1, ASSEMBLY_A);
    register(fixture.registry, runtimeB, 'replica-b', 1, ASSEMBLY_A);

    const activation = fixture.coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-2',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    });
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 2);
    expect(fixture.snapshots.get().assembly.assemblyIdentity).toBe(ASSEMBLY_A);

    fixture.coordinator.handleRuntimeControl(
      runtimeA,
      responseControl('prepared', 'replica-a', ASSEMBLY_B, 1)
    );
    await nextTurn();
    expect(fixture.snapshots.get().generation).toBe(1);

    fixture.coordinator.handleRuntimeControl(
      runtimeB,
      responseControl('prepared', 'replica-b', ASSEMBLY_B, 1)
    );
    await expect(activation).resolves.toMatchObject({
      committed: { generation: 2, assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B), },
      pending: null
    });
    expect(fixture.snapshots.get()).toMatchObject({
      generation: 2,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
      deploymentRuntimeBindings:
        assembly(ASSEMBLY_B).deploymentRuntimeBindings
    });
    expect(controlsOfType(fixture.controls, 'commit')).toHaveLength(2);
    fixture.coordinator.handleRuntimeControl(
      runtimeA,
      responseControl('prepared', 'replica-a', ASSEMBLY_B, 1)
    );
    await until(() => controlsOfType(fixture.controls, 'commit').length === 3);
    expect(fixture.registry.snapshot().map((replica) => replica.state)).toEqual([
      'draining',
      'draining'
    ]);
  });

  it('rejects registration and ACKs that match the assembly but not its config snapshot', async () => {
    const fixture = await coordinatorFixture();
    const runtime = fakeSocket();
    expect(() =>
      fixture.registry.register(runtime, {
        type: 'register',
        environment: 'test',
        generation: 1,
        assembly: { assemblyIdentity: ASSEMBLY_A },
        configSnapshot: configSnapshot(ASSEMBLY_B),
        replicaId: 'replica-a'
      })
    ).toThrow(/does not match committed generation/);

    register(fixture.registry, runtime, 'replica-a', 1, ASSEMBLY_A);
    const activation = fixture.coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-config-pair',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_B },
      configSnapshot: configSnapshot(ASSEMBLY_B)
    });
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 1);
    fixture.coordinator.handleRuntimeControl(runtime, {
      ...responseControl(
        'prepared',
        'replica-a',
        ASSEMBLY_B,
        1,
        'activation-config-pair'
      ),
      configSnapshot: configSnapshot(ASSEMBLY_C)
    });
    await until(() => vi.mocked(runtime.close).mock.calls.length === 1);
    expect(fixture.coordinator.activationState().pending).not.toBeNull();

    fixture.coordinator.handleRuntimeControl(
      runtime,
      responseControl(
        'prepared',
        'replica-a',
        ASSEMBLY_B,
        1,
        'activation-config-pair'
      )
    );
    await expect(activation).resolves.toMatchObject({
      committed: {
        generation: 2,
        assembly: { assemblyIdentity: ASSEMBLY_B },
        configSnapshot: configSnapshot(ASSEMBLY_B)
      }
    });
  });

  it('aborts reject and disconnect without moving the committed tuple', async () => {
    const fixture = await coordinatorFixture();
    const runtimeA = fakeSocket();
    const runtimeB = fakeSocket();
    register(fixture.registry, runtimeA, 'replica-a', 1, ASSEMBLY_A);
    register(fixture.registry, runtimeB, 'replica-b', 1, ASSEMBLY_A);
    const before = fixture.coordinator.activationState().committed;
    const activation = fixture.coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-reject',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_C }, configSnapshot: configSnapshot(ASSEMBLY_C),
    });
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 2);
    fixture.coordinator.handleRuntimeControl(
      runtimeA,
      responseControl('reject', 'replica-a', ASSEMBLY_C, 1, 'activation-reject')
    );
    await expect(activation).rejects.toThrow(/rejected activation during admission/);
    expect(fixture.coordinator.activationState().committed).toEqual(before);
    expect(fixture.coordinator.activationState().pending).toBeNull();
    expect(fixture.snapshots.get().assembly.assemblyIdentity).toBe(ASSEMBLY_A);
    expect(controlsOfType(fixture.controls, 'abort')).toHaveLength(2);

    const disconnected = fixture.coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-disconnect',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    });
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 4);
    fixture.registry.removeRuntimeConnection(runtimeB);
    fixture.coordinator.handleReplicaDisconnected('replica-b');
    await expect(disconnected).rejects.toThrow(/disconnected/);
    expect(fixture.coordinator.activationState().committed).toEqual(before);
  });

  it('replays an exact durable pending transaction on startup and rebuilds from committed only', async () => {
    const stateStore = new MemoryAssemblyActivationStateStore({
      schemaVersion: 'skiff-environment-activation-state-v2',
      environment: 'test',
      committed: { generation: 1, assembly: { assemblyIdentity: ASSEMBLY_A }, configSnapshot: configSnapshot(ASSEMBLY_A), },
      pending: {
        activationId: 'activation-recover',
        expectedGeneration: 1,
        candidateGeneration: 2,
        assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
        participantReplicaIds: ['replica-a', 'replica-b']
      }
    });
    const fixture = await coordinatorFixture(stateStore);
    expect(fixture.snapshots.get().assembly.assemblyIdentity).toBe(ASSEMBLY_A);
    const runtimeA = fakeSocket();
    const runtimeB = fakeSocket();
    register(fixture.registry, runtimeA, 'replica-a', 1, ASSEMBLY_A);
    register(fixture.registry, runtimeB, 'replica-b', 1, ASSEMBLY_A);
    fixture.coordinator.handleParticipantConnected('replica-a');
    fixture.coordinator.handleParticipantConnected('replica-b');
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 2);
    fixture.coordinator.handleRuntimeControl(
      runtimeA,
      responseControl('prepared', 'replica-a', ASSEMBLY_B, 1, 'activation-recover')
    );
    fixture.coordinator.handleRuntimeControl(
      runtimeB,
      responseControl('prepared', 'replica-b', ASSEMBLY_B, 1, 'activation-recover')
    );
    await until(() => fixture.snapshots.get().generation === 2);
    expect((await stateStore.read('test')).pending).toBeNull();
  });

  it('aborts a prepare timeout and recovers a crash-after-commit state from committed only', async () => {
    const snapshots = new RouterActiveAssemblySnapshotStore();
    const registry = new AssemblyRuntimeRegistry(snapshots);
    const stateStore = new MemoryAssemblyActivationStateStore(
      activationState({ environment: 'test', generation: 1, assemblyIdentity: ASSEMBLY_A })
    );
    const coordinator = new AssemblyActivationCoordinator({
      environment: 'test',
      stateStore,
      assemblyLoader: new MemoryRuntimeAssemblySnapshotLoader([
        assembly(ASSEMBLY_A),
        assembly(ASSEMBLY_B)
      ]),
      snapshots,
      registry,
      participants: registry,
      controlSender: { sendAssemblyControl: () => undefined },
      prepareTimeoutMs: 10
    });
    await coordinator.initialize();
    register(registry, fakeSocket(), 'replica-a', 1, ASSEMBLY_A);
    await expect(coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-timeout',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    })).rejects.toThrow(/timed out/);
    expect((await stateStore.read('test')).committed.generation).toBe(1);

    const committedStore = new MemoryAssemblyActivationStateStore(
      activationState({ environment: 'test', generation: 2, assemblyIdentity: ASSEMBLY_B })
    );
    const committedFixture = await coordinatorFixture(committedStore);
    expect(committedFixture.snapshots.get()).toMatchObject({
      generation: 2,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    });
    const stagedRuntime = fakeSocket();
    committedFixture.coordinator.handleRuntimeControl(
      stagedRuntime,
      responseControl('prepared', 'replica-after-crash', ASSEMBLY_B, 1, 'activation-before-crash')
    );
    await until(() => controlsOfType(committedFixture.controls, 'commit').length === 1);
    expect(controlsOfType(committedFixture.controls, 'commit')[0]).toMatchObject({
      activationId: 'activation-before-crash',
      expectedGeneration: 1,
      candidateGeneration: 2,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
      replicaId: 'replica-after-crash'
    });
  });

  it('rolls back a durable pending transaction when the commit adapter fails before CAS', async () => {
    const durable = new MemoryAssemblyActivationStateStore(
      activationState({
        environment: 'test',
        generation: 1,
        assemblyIdentity: ASSEMBLY_A
      })
    );
    const store: AssemblyActivationStateStore = {
      read: (environment) => durable.read(environment),
      prepare: (request, participants) => durable.prepare(request, participants),
      abort: (environment, pending) => durable.abort(environment, pending),
      commit: async () => {
        throw new Error('injected commit adapter failure');
      }
    };
    const fixture = await coordinatorFixture(store);
    const runtime = fakeSocket();
    register(fixture.registry, runtime, 'replica-a', 1, ASSEMBLY_A);
    const activation = fixture.coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-adapter-failure',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    });
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 1);
    fixture.coordinator.handleRuntimeControl(
      runtime,
      responseControl(
        'prepared',
        'replica-a',
        ASSEMBLY_B,
        1,
        'activation-adapter-failure'
      )
    );

    await expect(activation).rejects.toThrow(/injected commit adapter failure/);
    await expect(durable.read('test')).resolves.toMatchObject({
      committed: { generation: 1, assembly: { assemblyIdentity: ASSEMBLY_A }, configSnapshot: configSnapshot(ASSEMBLY_A), },
      pending: null
    });
    expect(fixture.snapshots.get()).toMatchObject({
      generation: 1,
      assembly: { assemblyIdentity: ASSEMBLY_A }, configSnapshot: configSnapshot(ASSEMBLY_A),
    });
    expect(controlsOfType(fixture.controls, 'abort')).toHaveLength(1);
  });

  it('converges to the durable commit when the adapter response is lost after CAS', async () => {
    const durable = new MemoryAssemblyActivationStateStore(
      activationState({
        environment: 'test',
        generation: 1,
        assemblyIdentity: ASSEMBLY_A
      })
    );
    const store: AssemblyActivationStateStore = {
      read: (environment) => durable.read(environment),
      prepare: (request, participants) => durable.prepare(request, participants),
      abort: (environment, pending) => durable.abort(environment, pending),
      commit: async (environment, pending, connected, prepared) => {
        await durable.commit(environment, pending, connected, prepared);
        throw new Error('injected lost commit response');
      }
    };
    const fixture = await coordinatorFixture(store);
    const runtime = fakeSocket();
    register(fixture.registry, runtime, 'replica-a', 1, ASSEMBLY_A);
    const activation = fixture.coordinator.activate({
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-lost-response',
      expectedGeneration: 1,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    });
    await until(() => controlsOfType(fixture.controls, 'prepare').length === 1);
    fixture.coordinator.handleRuntimeControl(
      runtime,
      responseControl(
        'prepared',
        'replica-a',
        ASSEMBLY_B,
        1,
        'activation-lost-response'
      )
    );

    await expect(activation).resolves.toMatchObject({
      committed: { generation: 2, assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B), },
      pending: null
    });
    expect(fixture.snapshots.get()).toMatchObject({
      generation: 2,
      assembly: { assemblyIdentity: ASSEMBLY_B }, configSnapshot: configSnapshot(ASSEMBLY_B),
    });
    expect(controlsOfType(fixture.controls, 'commit')).toHaveLength(1);
  });
});

async function coordinatorFixture(
  stateStore: AssemblyActivationStateStore = new MemoryAssemblyActivationStateStore(
    activationState({ environment: 'test', generation: 1, assemblyIdentity: ASSEMBLY_A })
  )
) {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  const registry = new AssemblyRuntimeRegistry(snapshots);
  const controls: AssemblyActivationControl[] = [];
  const coordinator = new AssemblyActivationCoordinator({
    environment: 'test',
    stateStore,
    assemblyLoader: new MemoryRuntimeAssemblySnapshotLoader([
      assembly(ASSEMBLY_A),
      assembly(ASSEMBLY_B),
      assembly(ASSEMBLY_C)
    ]),
    snapshots,
    registry,
    participants: registry,
    controlSender: {
      sendAssemblyControl: (_ws, control) => controls.push(control)
    },
    prepareTimeoutMs: 1000
  });
  await coordinator.initialize();
  return { coordinator, controls, registry, snapshots };
}

function register(
  registry: AssemblyRuntimeRegistry,
  ws: WebSocket,
  replicaId: string,
  generation: number,
  assemblyIdentity: string
): void {
  registry.register(ws, {
    type: 'register',
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    configSnapshot: configSnapshot(assemblyIdentity),
    replicaId
  });
}

function responseControl(
  type: 'prepared' | 'reject',
  replicaId: string,
  assemblyIdentity: string,
  expectedGeneration: number,
  activationId = 'activation-2'
): AssemblyActivationControl {
  const base = {
    environment: 'test',
    activationId,
    expectedGeneration,
    candidateGeneration: expectedGeneration + 1,
    assembly: { assemblyIdentity },
    configSnapshot: configSnapshot(assemblyIdentity),
    replicaId
  };
  return type === 'reject'
    ? { ...base, type, reason: 'admission' }
    : { ...base, type };
}

function fakeSocket(): WebSocket {
  return {
    readyState: WebSocket.OPEN,
    close: vi.fn()
  } as unknown as WebSocket;
}

function assembly(assemblyIdentity: string): LoadedRuntimeAssembly {
  const deployment = {
    serviceId: 'example.com/runtime',
    contractVersion: '1.0.0',
    deploymentRevision:
      assemblyIdentity === ASSEMBLY_A ? 'revision-a' : 'revision-b',
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v4:sha256:${'d'.repeat(64)}`
  };
  return {
    schemaVersion: 'skiff-runtime-assembly-v3',
    assemblyIdentity,
    resolvedDeployments: [deployment],
    resolvedContracts: [{
      serviceId: deployment.serviceId,
      contractVersion: deployment.contractVersion,
      serviceProtocolIdentity:
        `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`
    }],
    deploymentRuntimeBindings: [{
      deployment,
      packageBuildId:
        `skiff-package-build-v10:sha256:${'f'.repeat(64)}`
    }],
    gatewayIngress: []
  };
}

function identity(character: string): string {
  return `skiff-runtime-assembly-v3:sha256:${character.repeat(64)}`;
}

function configSnapshot(assemblyIdentity: string) {
  const marker = assemblyIdentity.slice(-1);
  return {
    snapshotId: `skiff-runtime-config-snapshot-v1:${marker.repeat(32)}`
  };
}

function activationState(input: {
  environment: string;
  generation: number;
  assemblyIdentity: string;
}) {
  return initialActivationState({
    ...input,
    configSnapshotId: configSnapshot(input.assemblyIdentity).snapshotId
  });
}

function controlsOfType(controls: readonly AssemblyActivationControl[], type: string) {
  return controls.filter((control) => control.type === type);
}

async function nextTurn(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
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
