import WebSocket from 'ws';

import { ActorRuntimeDisconnectController } from '../../src/router/actorRuntimeDisconnectController.js';
import { ProductionActorMethodRouter } from '../../src/router/productionActorMethodRouter.js';
import { RuntimeDispatcher } from '../../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../../src/router/runtimeRegistry.js';
import {
  ACTOR_ARGUMENTS_ENCODING_V1,
  ACTOR_RETURN_ENCODING_V1,
  type ActorMethodFrameHeader,
  type ActorMethodInvokeFrameHeader,
} from '../../src/protocol/actorMethodProtocol.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type SpawnSubmitRequestFrameHeader,
} from '../../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../../src/protocol/runtimeAssemblyRequest.js';

const sockets: WebSocket[] = [];
const endpoints: RuntimeEndpoint[] = [];

export const SERVICE_ID = 'example.com/actor';
export const EXTERNAL_SERVICE_ID = 'example.com/external-actor';
export const DECLARATION_OWNER = {
  unit: { kind: 'service' as const },
  file: { kind: 'loadedFileIndex' as const, value: 0 },
  actorSymbol: 'example.Counter',
};
export const ACTOR_ABI = identity('skiff-actor-abi-v1:sha256', 'a');
export const ACTOR_IMPLEMENTATION = identity(
  'skiff-actor-implementation-v1:sha256',
  'b'
);
export const NEXT_ACTOR_IMPLEMENTATION = identity(
  'skiff-actor-implementation-v1:sha256',
  '8'
);
export const METHOD = identity('skiff-actor-method-v1:sha256', 'e');
export const BUILD = identity('skiff-service-build-v1:sha256', 'c');
export const SERVICE_PROTOCOL = identity(
  'skiff-service-protocol-v5:sha256',
  'd'
);
export const ASSEMBLY =
  'skiff-runtime-assembly-v3:sha256:' + 'f'.repeat(64);
export const ARTIFACT =
  'skiff-service-artifact-v1:sha256:' + 'g'.repeat(64);
export const TEST_ARTIFACT =
  'skiff-deployment-artifact-v4:sha256:' + 'a'.repeat(64);
export const GATEWAY_ENTRY =
  'skiff-gateway-entry-v2:sha256:' + '9'.repeat(64);
export const TEST_CAPABILITY = 'test-case:actor_spawn_1.capability';

type ActorRecord = Awaited<
  ReturnType<ReturnType<RuntimeRegistry['actorManager']>['getOrCreate']>
>;

export interface ActorRoutingHarnessOptions {
  dispatcher?: boolean;
  secondRuntime?: boolean;
  externalRuntime?: boolean;
  now?: () => Date;
  onHasMethod?: () => void | Promise<void>;
  actorInvocationCorrelationCapacity?: number;
  id?: () => string;
}

interface InternalHarnessOptions extends ActorRoutingHarnessOptions {
  mode: 'spawn' | 'capability';
}

/**
 * Close every endpoint and socket created by this module in the current test
 * worker. Tests should register this once with `afterEach`.
 */
export async function cleanupActorRoutingHarnesses(): Promise<void> {
  for (const socket of sockets.splice(0)) socket.close();
  await Promise.all(endpoints.splice(0).map((endpoint) => endpoint.close()));
}

/** Preserve the original actor-spawn-submit harness defaults. */
export function spawnHarness(
  options: ActorRoutingHarnessOptions = {}
) {
  return createHarness({ ...options, mode: 'spawn' });
}

/**
 * Capability-routing variant. Importing this as `spawnHarness` lets the
 * capability suite retain its existing call sites and incremental ID ledger.
 */
export function capabilityHarness(
  options: ActorRoutingHarnessOptions = {}
) {
  return createHarness({ ...options, mode: 'capability' });
}

async function createHarness({
  mode,
  dispatcher = false,
  secondRuntime = false,
  externalRuntime = false,
  now,
  onHasMethod,
  actorInvocationCorrelationCapacity,
  id,
}: InternalHarnessOptions) {
  const registry = new RuntimeRegistry();
  const runtimeConnectionOverrides = new Map<string, WebSocket>();
  const authorityByRuntimeId = new Map([
    ['runtime-a', {
      serviceId: SERVICE_ID,
      serviceProtocolIdentity: SERVICE_PROTOCOL,
      generation: 1,
    }],
    ['runtime-b', {
      serviceId: SERVICE_ID,
      serviceProtocolIdentity: SERVICE_PROTOCOL,
      generation: 1,
    }],
    ['runtime-c', {
      serviceId: EXTERNAL_SERVICE_ID,
      serviceProtocolIdentity: SERVICE_PROTOCOL,
      generation: 1,
    }],
  ]);
  const issuedIds: string[] = [];
  let afterOwnerInvokeSend:
    | ((ws: WebSocket, bytes: Buffer) => void)
    | undefined;
  let nextGeneratedId = 0;
  const disconnect = new ActorRuntimeDisconnectController(
    registry.actorManager()
  );
  const endpoint = new RuntimeEndpoint({
    registry,
    actorRuntimeDisconnect: disconnect,
  });
  endpoints.push(endpoint);
  const nextId = id ?? (mode === 'capability'
    ? () => `spawn-id-${++nextGeneratedId}`
    : () => 'spawn-id');
  const actorMethods = new ProductionActorMethodRouter({
    registry,
    actorOwnerRouteAuthority: ({ runtimeId, serviceId }) => {
      const authority = authorityByRuntimeId.get(runtimeId);
      if (authority === undefined || authority.serviceId !== serviceId) {
        return undefined;
      }
      return {
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: authority.generation,
      };
    },
    runtimeDirectory: {
      actorRuntimeCandidates: (serviceId) =>
        registry.actorRuntimeCandidates(serviceId),
      runtimeConnection: (runtimeId) => {
        const override = runtimeConnectionOverrides.get(runtimeId);
        if (override !== undefined) return { runtimeId, ws: override };
        return registry.runtimeConnection(runtimeId);
      },
      runtimeIdForConnection: (ws) => {
        for (const [runtimeId, override] of runtimeConnectionOverrides) {
          if (override === ws) return runtimeId;
        }
        const runtimeId = registry.runtimeCapabilityIdentityForConnection(ws);
        return runtimeId !== undefined &&
          runtimeConnectionOverrides.has(runtimeId)
          ? undefined
          : runtimeId;
      },
    },
    disconnectController: disconnect,
    catalog: {
      hasMethod: mode === 'spawn'
        ? async () => {
            await onHasMethod?.();
            return true;
          }
        : () => {
            const result = onHasMethod?.();
            return result instanceof Promise
              ? result.then(() => true)
              : true;
          },
      declarationOwnerFor: () => DECLARATION_OWNER,
    },
    send: (ws, bytes) => {
      ws.send(bytes);
      if (
        mode === 'capability' &&
        decodeBinaryFrame(bytes).header.type === 'actor.owner.invoke'
      ) {
        afterOwnerInvokeSend?.(ws, bytes);
      }
    },
    id: () => {
      const issued = nextId();
      issuedIds.push(issued);
      return issued;
    },
    ...(actorInvocationCorrelationCapacity === undefined
      ? {}
      : { actorInvocationCorrelationCapacity }),
    ...(now === undefined ? {} : { now }),
  });
  endpoint.setActorMethods(actorMethods);
  const listening = await endpoint.listen({ port: 0 });
  const left = await runtime(listening.url, 'runtime-a');
  const right = secondRuntime
    ? await runtime(listening.url, 'runtime-b')
    : undefined;
  const external = externalRuntime
    ? await runtime(listening.url, 'runtime-c', EXTERNAL_SERVICE_ID)
    : undefined;
  let dispatcherInstance: RuntimeDispatcher | undefined;
  if (dispatcher) {
    const runtimeIdByWs = new Map<WebSocket, string>([
      [registry.runtimeConnection('runtime-a')!.ws, 'runtime-a'],
      ...(right === undefined
        ? []
        : [[registry.runtimeConnection('runtime-b')!.ws, 'runtime-b'] as const]),
      ...(external === undefined
        ? []
        : [[registry.runtimeConnection('runtime-c')!.ws, 'runtime-c'] as const]),
    ]);
    const runtimeA = registry.runtimeConnection('runtime-a')!.ws;
    dispatcherInstance = new RuntimeDispatcher({
      registry: {
        setInFlightCounter: () => {},
        pickDispatchConnection: () => null,
        refreshAllRuntimeStates: () => {},
        refreshRuntimeStatesForRequest: () => {},
        runtimeConnection: (runtimeId) => registry.runtimeConnection(runtimeId),
        runtimeCapabilityIdentityForConnection: (ws) =>
          registry.runtimeCapabilityIdentityForConnection(ws),
        pickAssemblyTestDispatchConnection: () => ({
          runtimeId: 'runtime-a',
          ws: runtimeA,
          runtimeAssemblyAuthority: {
            assemblyIdentity: ASSEMBLY,
            assemblyGeneration: 1,
            deployment: {
              serviceId: SERVICE_ID,
              contractVersion: '1.0.0',
              deploymentRevision: 'rev-1',
              deploymentArtifactIdentity: TEST_ARTIFACT,
            },
            buildId: BUILD,
            serviceProtocolIdentity: SERVICE_PROTOCOL,
          },
        }),
        spawnSubmitParentAuthority: (ws) => {
          const runtimeId = runtimeIdByWs.get(ws);
          if (runtimeId === undefined) return undefined;
          const authority = authorityByRuntimeId.get(runtimeId);
          if (authority === undefined) return undefined;
          return {
            runtimeId,
            buildId: BUILD,
            serviceProtocolIdentity: authority.serviceProtocolIdentity,
            assemblyIdentity: ASSEMBLY,
            assemblyGeneration: authority.generation,
            deployment: {
              serviceId: authority.serviceId,
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
    disconnectController: disconnect,
    dispatcher: dispatcherInstance,
    left,
    right,
    external,
    url: listening.url,
    ownerSocket: left,
    issuedIds,
    overrideRuntimeConnection: (runtimeId: string, ws: WebSocket) => {
      runtimeConnectionOverrides.set(runtimeId, ws);
    },
    setAfterOwnerInvokeSend: (
      callback: (ws: WebSocket, bytes: Buffer) => void
    ) => {
      afterOwnerInvokeSend = callback;
    },
    setCurrentAuthorityGeneration: (generation: number) => {
      for (const authority of authorityByRuntimeId.values()) {
        authority.generation = generation;
      }
    },
  };
}

export function spawnSubmit({
  runtimeId,
  callerRequestId,
  actor,
  traceId,
  serviceProtocolIdentity = SERVICE_PROTOCOL,
  generation = 1,
  serviceId = SERVICE_ID,
}: {
  runtimeId: string;
  callerRequestId: string;
  actor: ActorRecord;
  traceId?: string;
  serviceProtocolIdentity?: string;
  generation?: number;
  serviceId?: string;
}): SpawnSubmitRequestFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.request',
    rpcId: 'spawn-rpc-1',
    runtimeId,
    activationIdentity: {
      assemblyIdentity: ASSEMBLY,
      generation,
      runtimeReplicaId: runtimeId,
      deploymentRevision: 'rev-1',
    },
    targetKind: 'actorMethod',
    serviceId,
    serviceVersion: '1.0.0',
    serviceProtocolIdentity,
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

export function spawnContext(registry: RuntimeRegistry, runtimeId: string) {
  const connection = registry.runtimeConnection(runtimeId);
  if (connection === undefined) throw new Error('runtime connection missing');
  return {
    originRuntimeId: runtimeId,
    originRuntimeConnection: connection.ws,
  };
}

export function rootAuthority(
  runtimeId: string,
  testCaseCapability: string,
  generation = 1
) {
  return Object.freeze({
    runtimeId,
    buildId: BUILD,
    serviceProtocolIdentity: SERVICE_PROTOCOL,
    assemblyIdentity: ASSEMBLY,
    assemblyGeneration: generation,
    testCaseCapability,
    deployment: Object.freeze({
      serviceId: SERVICE_ID,
      contractVersion: '1.0.0',
      deploymentRevision: 'rev-1',
      deploymentArtifactIdentity: ARTIFACT,
    }),
  });
}

export function actorBootstrap(
  keyByte = 1,
  serviceId = SERVICE_ID
) {
  return {
    actorKey: {
      serviceId,
      actorTypeIdentity: 'actor.example.Counter',
      actorIdTypeIdentity: 'type.example.CounterId',
      actorIdEncodingVersion: 'skiff-canonical-v1',
      canonicalActorIdKeyBytes: new Uint8Array([keyByte]),
    },
    actorAbiIdentity: ACTOR_ABI,
    actorImplementationIdentity: ACTOR_IMPLEMENTATION,
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    encodedBootstrapBytes: Buffer.from('{}'),
  };
}

export function actorKeyOf(actor: ActorRecord) {
  return {
    serviceId: actor.serviceId,
    actorTypeIdentity: actor.actorTypeIdentity,
    actorIdTypeIdentity: actor.actorIdTypeIdentity,
    actorIdEncodingVersion: actor.actorIdEncodingVersion,
    canonicalActorIdKeyBytes: actor.canonicalActorIdKeyBytes,
    actorIdHash: actor.actorIdHash,
  };
}

export function invocation(
  actor: ActorRecord,
  invocationId: string,
  testMetadata: {
    testCaseCapability: string;
    testCaseParentRequestId: string;
  } | undefined = undefined
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
    ...(testMetadata ?? {}),
  };
}

export function testRoot(
  requestId: string,
  testCaseCapability: string,
  generation = 1
): RuntimeAssemblyRequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId,
    mode: 'unary',
    caller: { kind: 'gateway' },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity: ASSEMBLY,
      assemblyGeneration: generation,
      deployment: {
        serviceId: SERVICE_ID,
        contractVersion: '1.0.0',
        deploymentRevision: 'rev-1',
        deploymentArtifactIdentity: TEST_ARTIFACT,
      },
      gatewayEntryIdentity: GATEWAY_ENTRY,
      ingress: { protocol: 'http', method: 'POST', path: '/actor-test' },
    },
    deadline: {
      timeoutMs: 60_000,
      expiresAt: new Date(Date.now() + 60_000).toISOString(),
    },
    trace: { traceId: 'trace:root', spanId: 'span:root' },
    httpRequest: {
      method: 'POST',
      url: 'http://actor.local/actor-test',
      path: '/actor-test',
      query: [],
      headers: [],
    },
    testEffectsEnabled: true,
    testCaseCapability,
  };
}

export function terminalFrame(
  kind: 'return' | 'error' | 'cancel',
  actor: ActorRecord,
  invocationId: string
): ActorMethodFrameHeader {
  if (kind === 'return') {
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.return',
      invocationId,
      returnEncodingVersion: ACTOR_RETURN_ENCODING_V1,
    };
  }
  if (kind === 'error') {
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'actor.method.error',
      invocationId,
      error: {
        name: 'actorUpgradingError',
        actorRef: invocation(actor, invocationId).actorRef,
        retryAfterMs: 1,
      },
    };
  }
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.method.cancel',
    invocationId,
    cancellationCorrelation: `cancel-${invocationId}`,
    reason: 'cancelled',
  };
}

export function terminalLedgerState(
  kind: 'return' | 'error' | 'cancel'
): 'completed' | 'failed' | 'cancelled' {
  return kind === 'return'
    ? 'completed'
    : kind === 'error'
      ? 'failed'
      : 'cancelled';
}

export async function runtime(
  url: string,
  runtimeId: string,
  serviceId = SERVICE_ID
): Promise<WebSocket> {
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
    serviceId,
    revisionId: 'a'.repeat(64),
    buildId: BUILD,
    serviceProtocolIdentity: SERVICE_PROTOCOL,
    targets: ['actor.example.Counter.increment'],
  }));
  const response = decodeBinaryFrame(await nextBinary(socket));
  if (response.header.type !== 'runtime.registered') {
    throw new Error(`expected runtime.registered, got ${response.header.type}`);
  }
  return socket;
}

export function nextBinary(socket: WebSocket): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const onMessage = (data: WebSocket.RawData, binary: boolean) => {
      socket.off('error', onError);
      if (!binary) reject(new Error('expected binary frame'));
      else resolve(
        Buffer.isBuffer(data) ? data : Buffer.from(data as ArrayBuffer)
      );
    };
    const onError = (error: Error) => {
      socket.off('message', onMessage);
      reject(error);
    };
    socket.once('message', onMessage);
    socket.once('error', onError);
  });
}

export function nextBinaryMessages(
  socket: WebSocket,
  count: number
): Promise<Buffer[]> {
  return new Promise((resolve, reject) => {
    const messages: Buffer[] = [];
    const onMessage = (data: WebSocket.RawData, binary: boolean) => {
      if (!binary) {
        cleanup();
        reject(new Error('expected binary frame'));
        return;
      }
      messages.push(
        Buffer.isBuffer(data) ? data : Buffer.from(data as ArrayBuffer)
      );
      if (messages.length === count) {
        cleanup();
        resolve(messages);
      }
    };
    const onError = (error: Error) => {
      cleanup();
      reject(error);
    };
    const cleanup = () => {
      socket.off('message', onMessage);
      socket.off('error', onError);
    };
    socket.on('message', onMessage);
    socket.on('error', onError);
  });
}

export function waitForClose(socket: WebSocket): Promise<[number, string]> {
  return new Promise((resolve) => {
    socket.once('close', (code, reason) => {
      resolve([code, reason.toString()]);
    });
  });
}

export async function waitFor(predicate: () => boolean): Promise<void> {
  const started = Date.now();
  while (!predicate()) {
    if (Date.now() - started > 1_000) {
      throw new Error('timed out waiting for predicate');
    }
    await delay(5);
  }
}

export async function waitForAsync(
  predicate: () => Promise<boolean>
): Promise<void> {
  const started = Date.now();
  while (!(await predicate())) {
    if (Date.now() - started > 1_000) {
      throw new Error('timed out waiting for async predicate');
    }
    await delay(5);
  }
}

export function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

export function fakeOpenSocket(send: () => void): WebSocket {
  return {
    readyState: WebSocket.OPEN,
    send,
  } as unknown as WebSocket;
}

export function identity(prefix: string, digit: string): string {
  return `${prefix}:${digit.repeat(64)}`;
}
