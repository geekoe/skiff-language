import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { ActorManager } from '../src/actor/index.js';
import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import type { AssemblyActivationControl } from '../src/protocol/assemblyActivationProtocol.js';
import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RequestCancelFrameHeader,
  type RuntimeHealthCounters,
  type RuntimeHealthFrameHeader
} from '../src/protocol/envelope.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeProtocol.js';
import { ActorGetCreateActivationCoordinator } from '../src/router/actorGetCreateActivationCoordinator.js';
import { ActorRuntimeDisconnectController } from '../src/router/actorRuntimeDisconnectController.js';
import { ActorSpawnRuntimeControl } from '../src/router/actorSpawnRuntimeControl.js';
import {
  initialActivationState,
  MemoryAssemblyActivationStateStore
} from '../src/router/assemblyActivationStateStore.js';
import { AssemblyActivationCoordinator } from '../src/router/assemblyActivationCoordinator.js';
import { AssemblyControlPlane } from '../src/router/assemblyControlPlane.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  MemoryRuntimeAssemblySnapshotLoader,
  RouterActiveAssemblySnapshotStore,
  type LoadedRuntimeAssembly
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY_A = identity('a');
const RUNTIME_ID = 'runtime-loop-risk-assembly';
const SERVICE_ID = 'example.com/actors';
const SERVICE_VERSION = '1.0.0';
const SERVICE_PROTOCOL =
  `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`;
const PACKAGE_BUILD_ID = `skiff-package-build-v10:sha256:${'d'.repeat(64)}`;
const CURRENT_TEST_GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'9'.repeat(64)}`;
const TEST_HOST = 'case-0.package-test.skiff.localhost';
const TEST_PATH = '/__skiff/package-test/0';
const CONFIG_SNAPSHOT = {
  snapshotId: 'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
};
const fixtures: Fixture[] = [];

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()?.close();
  }
});

describe('loop-risk health detail on the AssemblyControlPlane', () => {
  it('exposes runtime.health counters and accepts fresh zero updates', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);

    const nonzero = nonzeroRuntimeCounters();
    ws.send(encodeRuntimeFrame(runtimeHealthFrame(RUNTIME_ID, nonzero)));

    let health = await waitForRuntimeCounters(fixture, RUNTIME_ID, nonzero);
    expect(health.router).toMatchObject({
      dispatcher: {
        pendingUnary: 0,
        pendingStream: 0
      },
      httpStream: {
        backpressureWaiters: 0,
        backpressureCancels: 0
      }
    });
    expect(health.runtimes).toHaveLength(1);
    expect(runtimeSnapshot(health, RUNTIME_ID)).toMatchObject({
      runtimeId: RUNTIME_ID,
      connected: true,
      fresh: true,
      counters: nonzero
    });

    ws.send(encodeRuntimeFrame(runtimeHealthFrame(RUNTIME_ID, zeroRuntimeCounters())));

    health = await waitForRuntimeCounters(fixture, RUNTIME_ID, zeroRuntimeCounters());
    expect(runtimeSnapshot(health, RUNTIME_ID)).toMatchObject({
      runtimeId: RUNTIME_ID,
      connected: true,
      fresh: true,
      counters: zeroRuntimeCounters()
    });
  });

  it('reports dispatcher pending counters and returns them to zero', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);
    ws.send(encodeRuntimeFrame(runtimeHealthFrame(RUNTIME_ID, zeroRuntimeCounters())));
    await waitForRuntimeCounters(fixture, RUNTIME_ID, zeroRuntimeCounters());

    const responsePromise = postControlJson(
      `${fixture.controlUrl}/__skiff/test-dispatch`,
      testDispatchBody()
    );
    const requestFrame = await nextRuntimeFrame(ws, 'request.start');
    const validation = validateRuntimeAssemblyRequestStartFrameHeader(requestFrame.header);
    expect(validation.ok).toBe(true);
    if (!validation.ok) {
      throw new Error(validation.error);
    }

    let health = await readLoopRiskHealth(fixture);
    expect(health.router.dispatcher).toMatchObject({
      pendingUnary: 1,
      pendingStream: 0
    });

    sendRootResponseEnd(ws, validation.envelope.requestId);
    await expect(responsePromise).resolves.toMatchObject({ status: 200 });

    health = await waitForDispatcherZero(fixture, 5000);
    expect(health.router.dispatcher).toMatchObject({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(runtimeSnapshot(health, RUNTIME_ID).counters).toEqual(zeroRuntimeCounters());
  });

  it('replaces the health record when a runtimeId reconnects', async () => {
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A
    });
    const first = await openSocket(fixture.url);
    sendCapabilities(first, RUNTIME_ID);
    sendActivation(first, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);
    first.send(
      encodeRuntimeFrame(runtimeHealthFrame(RUNTIME_ID, nonzeroRuntimeCounters()))
    );
    await waitForRuntimeCounters(fixture, RUNTIME_ID, nonzeroRuntimeCounters());

    first.close();
    await until(() => fixture.runtimeRegistry.capabilityConnectionsSnapshot().length === 0);
    const second = await openSocket(fixture.url);
    sendCapabilities(second, RUNTIME_ID);
    sendActivation(second, registration(1, ASSEMBLY_A));
    second.send(
      encodeRuntimeFrame(runtimeHealthFrame(RUNTIME_ID, zeroRuntimeCounters()))
    );

    const health = await waitForRuntimeCounters(fixture, RUNTIME_ID, zeroRuntimeCounters());
    const sessions = health.runtimes.filter(
      (runtime) => runtime.runtimeId === RUNTIME_ID
    );
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({
      runtimeId: RUNTIME_ID,
      connected: true,
      fresh: true,
      counters: zeroRuntimeCounters()
    });
  });

  it('drains a bounded runtime dispatch cancel storm to zero-window health', async () => {
    // Router unit tests keep this below the 1000-attempt stable-instance stress
    // target so the suite stays deterministic while still exercising the same
    // dispatcher cancel terminal path and health zero-window schema.
    const stormAttempts = 96;
    const fixture = await createFixture({
      generation: 1,
      assemblyIdentity: ASSEMBLY_A,
      testGateway: true,
      maxConcurrency: stormAttempts
    });
    const ws = await openSocket(fixture.url);
    sendCapabilities(ws, RUNTIME_ID);
    sendActivation(ws, registration(1, ASSEMBLY_A));
    await until(() => fixture.assemblyRegistry.healthyParticipantReplicaIds().length === 1);
    ws.send(
      encodeRuntimeFrame(
        runtimeHealthFrame(RUNTIME_ID, nonzeroRuntimeCounters())
      )
    );
    await waitForRuntimeCounters(fixture, RUNTIME_ID, nonzeroRuntimeCounters());

    const cancelFrames = collectRuntimeCancelFrames(
      ws,
      stormAttempts,
      'loop-risk cancel storm cancels'
    );
    const dispatches = Array.from(
      { length: stormAttempts },
      () => postControlJson(
        `${fixture.controlUrl}/__skiff/test-dispatch`,
        testDispatchBody(50)
      )
    );
    const cancels = await cancelFrames;
    expect(cancels).toHaveLength(stormAttempts);
    expect(cancels.every((cancel) => cancel.reason === 'timeout')).toBe(true);
    const results = await Promise.all(dispatches);
    expect(results.every((result) => result.status === 504)).toBe(true);

    ws.send(
      encodeRuntimeFrame(
        runtimeHealthFrame(RUNTIME_ID, zeroRuntimeCounters())
      )
    );
    const health = await waitForLoopRiskZeroWindow(
      fixture,
      RUNTIME_ID,
      5000
    );
    expect(health.router).toEqual({
      dispatcher: {
        pendingUnary: 0,
        pendingStream: 0
      },
      httpStream: {
        backpressureWaiters: 0,
        backpressureCancels: 0
      }
    });
  });
});

interface Fixture {
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
    maxConcurrency?: number;
  }
): Promise<Fixture> {
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
      assemblyIdentity: initial.assemblyIdentity,
      configSnapshotId: CONFIG_SNAPSHOT.snapshotId
    })),
    assemblyLoader: new MemoryRuntimeAssemblySnapshotLoader([
      assembly(initial.assemblyIdentity, initial.testGateway ?? false)
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
    snapshots,
    httpStreamCounters: () => ({
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 0
    })
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
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.capabilities',
    runtimeId,
    capabilities: {
      packageTestDispatch: true,
      requestCancel: true
    }
  }));
}

function sendActivation(ws: WebSocket, control: AssemblyActivationControl): void {
  ws.send(encodeAssemblyActivationFrame('runtimeToRouter', control));
}

function registration(
  generation: number,
  assemblyIdentity: string
): AssemblyActivationControl {
  return {
    type: 'register',
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    configSnapshot: CONFIG_SNAPSHOT,
    replicaId: RUNTIME_ID
  };
}

function runtimeHealthFrame(
  runtimeId: string,
  counters: RuntimeHealthCounters
): RuntimeHealthFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'runtime.health',
    runtimeId,
    observedAt: new Date().toISOString(),
    counters
  };
}

function zeroRuntimeCounters(): RuntimeHealthCounters {
  return {
    outboundRequestsPending: 0,
    outboundStreamLeasesActive: 0,
    streamRuntimeStreamsActive: 0,
    flagBackedCancelWaitersActive: 0,
    spawnedTasksActive: 0
  };
}

function nonzeroRuntimeCounters(): RuntimeHealthCounters {
  return {
    outboundRequestsPending: 1,
    outboundStreamLeasesActive: 1,
    streamRuntimeStreamsActive: 1,
    flagBackedCancelWaitersActive: 1,
    spawnedTasksActive: 1
  };
}

interface LoopRiskHealthPayload {
  observedAt: string;
  router: {
    dispatcher: {
      pendingUnary: number;
      pendingStream: number;
    };
    httpStream: {
      backpressureWaiters: number;
      backpressureCancels: number;
    };
  };
  runtimes: Array<{
    runtimeId: string;
    connected: boolean;
    fresh: boolean;
    counters: RuntimeHealthCounters;
  }>;
}

async function readLoopRiskHealth(fixture: Fixture): Promise<LoopRiskHealthPayload> {
  const response = await fetch(`${fixture.controlUrl}/__router/health?detail=loop-risk`);
  expect(response.status).toBe(200);
  const payload = (await response.json()) as {
    loopRisk: LoopRiskHealthPayload;
  };
  expect(payload.loopRisk.observedAt).toEqual(expect.any(String));
  return payload.loopRisk;
}

async function waitForRuntimeCounters(
  fixture: Fixture,
  runtimeId: string,
  counters: RuntimeHealthCounters
): Promise<LoopRiskHealthPayload> {
  let latest = await readLoopRiskHealth(fixture);
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (
      latest.runtimes.some(
        (runtime) =>
          runtime.runtimeId === runtimeId &&
          runtime.connected &&
          runtime.fresh &&
          JSON.stringify(runtime.counters) === JSON.stringify(counters)
      )
    ) {
      return latest;
    }
    await delay(10);
    latest = await readLoopRiskHealth(fixture);
  }
  expect(latest.runtimes).toContainEqual(
    expect.objectContaining({
      runtimeId,
      connected: true,
      fresh: true,
      counters
    })
  );
  return latest;
}

async function waitForDispatcherZero(
  fixture: Fixture,
  timeoutMs: number
): Promise<LoopRiskHealthPayload> {
  const startedAt = Date.now();
  let latest = await readLoopRiskHealth(fixture);
  while (Date.now() - startedAt <= timeoutMs) {
    if (
      latest.router.dispatcher.pendingUnary === 0 &&
      latest.router.dispatcher.pendingStream === 0
    ) {
      return latest;
    }
    await delay(25);
    latest = await readLoopRiskHealth(fixture);
  }
  expect(latest.router.dispatcher).toEqual({
    pendingUnary: 0,
    pendingStream: 0
  });
  return latest;
}

async function waitForLoopRiskZeroWindow(
  fixture: Fixture,
  runtimeId: string,
  timeoutMs: number
): Promise<LoopRiskHealthPayload> {
  const startedAt = Date.now();
  let latest = await readLoopRiskHealth(fixture);
  while (Date.now() - startedAt <= timeoutMs) {
    if (routerLoopRiskCountersAreZero(latest)) {
      const runtime = latest.runtimes.find(
        (snapshot) =>
          snapshot.runtimeId === runtimeId &&
          snapshot.connected &&
          snapshot.fresh &&
          JSON.stringify(snapshot.counters) === JSON.stringify(zeroRuntimeCounters())
      );
      if (runtime) {
        return latest;
      }
    }
    await delay(25);
    latest = await readLoopRiskHealth(fixture);
  }
  expect(routerLoopRiskCountersAreZero(latest)).toBe(true);
  expect(latest.runtimes).toContainEqual(
    expect.objectContaining({
      runtimeId,
      connected: true,
      fresh: true,
      counters: zeroRuntimeCounters()
    })
  );
  return latest;
}

function routerLoopRiskCountersAreZero(health: LoopRiskHealthPayload): boolean {
  return (
    health.router.dispatcher.pendingUnary === 0 &&
    health.router.dispatcher.pendingStream === 0 &&
    health.router.httpStream.backpressureWaiters === 0 &&
    health.router.httpStream.backpressureCancels === 0
  );
}

function runtimeSnapshot(
  health: LoopRiskHealthPayload,
  runtimeId: string
): LoopRiskHealthPayload['runtimes'][number] {
  const runtime = health.runtimes.find((item) => item.runtimeId === runtimeId);
  expect(runtime).toBeDefined();
  return runtime!;
}

function testDispatchBody(timeoutMs = 1000): Record<string, unknown> {
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
    timeoutMs
  };
}

async function postControlJson(
  url: string,
  body: unknown
): Promise<{ status: number; body: unknown }> {
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

async function nextRuntimeFrame(
  ws: WebSocket,
  type: string
): Promise<ReturnType<typeof decodeRuntimeFrame>> {
  const data = await nextBinaryMessage(ws);
  const frame = decodeRuntimeFrame(data);
  expect(frame.header.type).toBe(type);
  return frame;
}

async function nextBinaryMessage(ws: WebSocket): Promise<Buffer> {
  return await new Promise<Buffer>((resolve, reject) => {
    const timeout = setTimeout(
      () => reject(new Error('timed out waiting for binary frame')),
      1000
    );
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

function collectRuntimeCancelFrames(
  ws: WebSocket,
  count: number,
  label: string,
  timeoutMs = 5000
): Promise<RequestCancelFrameHeader[]> {
  return new Promise((resolve, reject) => {
    const cancels: RequestCancelFrameHeader[] = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, timeoutMs);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'request.cancel') {
        return;
      }
      cancels.push(frame.header);
      if (cancels.length === count) {
        cleanup();
        resolve(cancels);
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
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

function assembly(
  assemblyIdentity: string,
  includeTestGateway = false
): LoadedRuntimeAssembly {
  const revision = deploymentRevision(assemblyIdentity);
  const deployment = deploymentRef(revision);
  return {
    schemaVersion: 'skiff-runtime-assembly-v3',
    assemblyIdentity,
    resolvedDeployments: [deployment],
    resolvedContracts: [
      {
        serviceId: SERVICE_ID,
        contractVersion: SERVICE_VERSION,
        serviceProtocolIdentity: SERVICE_PROTOCOL
      }
    ],
    deploymentRuntimeBindings: [
      {
        deployment,
        packageBuildId: PACKAGE_BUILD_ID
      }
    ],
    gatewayIngress: includeTestGateway
      ? [
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
          }
        ]
      : []
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

function identity(character: string): string {
  return `skiff-runtime-assembly-v3:sha256:${character.repeat(64)}`;
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

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function until(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) {
      return;
    }
    await delay(5);
  }
  throw new Error('condition was not reached');
}
