import { request } from 'node:http';
import { connect } from 'node:net';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import {
  AssemblyWebSocketGateway,
  CANONICAL_WEBSOCKET_INGRESS_ARGS,
  canonicalWebSocketIngressIdentity
} from '../src/gateway/assemblyWebSocketGateway.js';
import type { ConnectionSendEnvelope } from '../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeAssemblyRequest.js';
import { validateRuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeProtocol.js';
import { AssemblyHttpGateway } from '../src/router/assemblyHttpGateway.js';
import type { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import type { ConnectionSendHandler } from '../src/router/runtimeEndpoint.js';
import type {
  RuntimeDispatchConnection,
  RuntimeUnaryDispatchFrameHeader
} from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const CODEX_MODELS_OPERATION = operationIdentity('b');
const AIHUB_MODELS_OPERATION = operationIdentity('c');
const AIHUB_SOCKET_OPERATION = operationIdentity('d');
const AGINE_SOCKET_OPERATION = operationIdentity('e');
const calls: RuntimeAssemblyRequestStartFrameHeader[] = [];
const resources: Array<{ close(): Promise<void> }> = [];

afterEach(async () => {
  while (resources.length > 0) {
    await resources.pop()!.close();
  }
  calls.length = 0;
});

describe('RuntimeAssembly Host ingress', () => {
  it('disambiguates the same HTTP path by Host and ignores legacy selectors', async () => {
    const { gateway, url } = await listenHttp();
    resources.push(gateway);
    const codex = await httpGet(url, 'codex-relay.localhost', {
      'x-skiff-service': 'wrong/service',
      'x-skiff-version': 'wrong-version',
      'x-skiff-release': 'wrong-release'
    }, '?service=also-wrong&version=also-wrong');
    expect(codex.status).toBe(200);
    expect(calls.at(-1)).toMatchObject({
      routing: { contractOperationId: CODEX_MODELS_OPERATION }
    });

    const aihub = await httpGet(url, 'aihub.localhost');
    expect(aihub.status).toBe(200);
    expect(calls.at(-1)).toMatchObject({
      routing: { contractOperationId: AIHUB_MODELS_OPERATION }
    });

    const unknown = await httpGet(url, 'unknown.localhost');
    expect(unknown.status).toBe(404);
    expect(await httpWithoutHost(url)).toBe(421);
    expect(calls).toHaveLength(2);
  });

  it('dispatches canonical WebSocket ingress with pinned generation and exact direct-send identity', async () => {
    const snapshots = snapshotStore();
    const runtimeConnectionSend = connectionSendSource();
    const runtimeA = runtimeDispatchConnection('runtime-A');
    const runtimeB = runtimeDispatchConnection('runtime-B');
    let currentRuntime = runtimeA;
    const registrySelections: RuntimeAssemblyRequestStartFrameHeader[] = [];
    const dispatchConnections: Array<RuntimeDispatchConnection | undefined> = [];
    const dispatcher = fakeDispatcher(dispatchConnections);
    const http = new AssemblyHttpGateway({ snapshots, dispatcher: fakeDispatcher(), port: 0 });
    const httpListen = await http.listen();
    resources.push(http);
    const websocket = new AssemblyWebSocketGateway({
      snapshots,
      dispatcher,
      runtimeConnectionSend,
      registry: {
        pickDispatchConnection(header) {
          const validation = validateRuntimeAssemblyRequestStartFrameHeader(header);
          if (!validation.ok) throw new Error(validation.error);
          registrySelections.push(validation.envelope);
          return currentRuntime;
        }
      },
      server: httpListen.server
    });
    await websocket.listen();
    resources.push(websocket);

    const oldClient = await openWebSocket(httpListen.url, 'aihub.localhost', '/ws?service=wrong');
    resources.push(webSocketResource(oldClient));
    const oldConnect = calls.at(-1)!;
    expect(oldConnect).toMatchObject({
      mode: 'unary',
      routing: {
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 7,
        contractOperationId: AIHUB_SOCKET_OPERATION,
        ingress: {
          protocol: 'webSocket',
          host: 'aihub.localhost',
          method: null,
          path: '/ws'
        }
      },
      websocketAdapter: {
        kind: 'connect',
        adapterArgs: CANONICAL_WEBSOCKET_INGRESS_ARGS
      }
    });
    expect(oldConnect.websocketAdapter).not.toHaveProperty('contextExpectation');
    expect(dispatchConnections).toEqual([runtimeA]);
    expect(registrySelections).toHaveLength(1);
    const connectionId = oldConnect.websocketAdapter!.connectRequest!.connectionId;
    const websocketEntryId = oldConnect.websocketEntryId!;

    snapshots.replace(snapshot(8, '9'));
    currentRuntime = runtimeB;
    oldClient.send('generation-A');
    await waitFor(() => calls.length === 2, 'old generation receive');
    expect(calls[1]).toMatchObject({
      routing: {
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 7,
        contractOperationId: AIHUB_SOCKET_OPERATION
      },
      gatewayEntryIdentity: oldConnect.gatewayEntryIdentity,
      websocketEntryId,
      websocketAdapter: {
        kind: 'receive',
        adapterArgs: CANONICAL_WEBSOCKET_INGRESS_ARGS,
        receiveEvent: {
          connectionId,
          contextCodec: {
            operationAbiId: AIHUB_SOCKET_OPERATION,
            contextTypeIdentity: 'skiff-contract-type-v1:sha256:context'
          },
          payloadSegments: [
            { kind: 'websocket.context', offset: 0, length: 0 },
            { kind: 'websocket.message', offset: 0, length: 12 }
          ]
        }
      }
    });
    expect(dispatchConnections).toEqual([runtimeA, runtimeA]);
    expect(registrySelections).toHaveLength(1);

    const newClient = await openWebSocket(httpListen.url, 'aihub.localhost', '/ws');
    resources.push(webSocketResource(newClient));
    expect(calls.at(-1)).toMatchObject({
      routing: {
        assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${'9'.repeat(64)}`,
        assemblyGeneration: 8
      },
      gatewayEntryIdentity: oldConnect.gatewayEntryIdentity,
      websocketEntryId
    });
    expect(dispatchConnections).toEqual([runtimeA, runtimeA, runtimeB]);
    expect(registrySelections).toHaveLength(2);

    const received: WebSocket.RawData[] = [];
    oldClient.on('message', (data) => received.push(data));
    runtimeConnectionSend.emit({
      type: 'connection.send',
      serviceId: 'service/aihub.localhost',
      websocketEntryId: `skiff-websocket-entry-v1:sha256:${'0'.repeat(64)}`,
      connectionId,
      payloadKind: 'text',
      payloadBytes: Buffer.from('wrong-entry')
    });
    runtimeConnectionSend.emit({
      type: 'connection.send',
      serviceId: 'service/agine.localhost',
      websocketEntryId,
      connectionId,
      payloadKind: 'text',
      payloadBytes: Buffer.from('wrong-service')
    });
    await delay(20);
    expect(received).toHaveLength(0);
    runtimeConnectionSend.emit({
      type: 'connection.send',
      serviceId: 'service/aihub.localhost',
      websocketEntryId,
      connectionId,
      payloadKind: 'text',
      payloadBytes: Buffer.from('canonical-direct')
    });
    await waitFor(() => received.length === 1, 'canonical direct send');
    expect(String(received[0])).toBe('canonical-direct');

    const agineClient = await openWebSocket(httpListen.url, 'agine.localhost', '/ws');
    resources.push(webSocketResource(agineClient));
    await expect(openWebSocket(httpListen.url, 'unknown.localhost', '/ws')).rejects.toThrow();
  });

  it('keeps canonical WebSocket entry identity stable across implementation generations and sensitive to ABI', () => {
    const original = binding(
      'webSocket',
      'aihub.localhost',
      null,
      '/ws',
      AIHUB_SOCKET_OPERATION
    );
    const samePublicIngress = structuredClone(original);
    samePublicIngress.deployment.deploymentRevision = 'generation-B';
    samePublicIngress.deployment.deploymentArtifactIdentity =
      `skiff-deployment-artifact-v1:sha256:${'8'.repeat(64)}`;
    const changedAbi = structuredClone(original);
    changedAbi.contractOperationId = operationIdentity('9');

    expect(canonicalWebSocketIngressIdentity(samePublicIngress)).toEqual(
      canonicalWebSocketIngressIdentity(original)
    );
    expect(canonicalWebSocketIngressIdentity(changedAbi)).not.toEqual(
      canonicalWebSocketIngressIdentity(original)
    );
  });
});

async function listenHttp() {
  const gateway = new AssemblyHttpGateway({
    snapshots: snapshotStore(),
    dispatcher: fakeDispatcher(),
    port: 0
  });
  const listen = await gateway.listen();
  return { gateway, url: listen.url };
}

function snapshotStore(): RouterActiveAssemblySnapshotStore {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace(snapshot(7, 'a'));
  return snapshots;
}

function snapshot(generation: number, assemblyCharacter: string) {
  return {
    environment: 'test',
    generation,
    assembly: {
      assemblyIdentity: `skiff-runtime-assembly-v1:sha256:${assemblyCharacter.repeat(64)}`
    },
    ingress: new RuntimeAssemblyIngressIndex([
      binding('http', 'codex-relay.localhost', 'GET', '/v1/models', CODEX_MODELS_OPERATION),
      binding('http', 'aihub.localhost', 'GET', '/v1/models', AIHUB_MODELS_OPERATION),
      binding('webSocket', 'aihub.localhost', null, '/ws', AIHUB_SOCKET_OPERATION),
      binding('webSocket', 'agine.localhost', null, '/ws', AGINE_SOCKET_OPERATION)
    ])
  };
}

function binding(
  protocol: 'http' | 'webSocket',
  host: string,
  method: string | null,
  path: string,
  operation: string
): RuntimeAssemblyIngressBinding {
  return {
    selector: { protocol, host, method, path },
    deployment: {
      serviceId: `service/${host}`,
      contractVersion: '1.0.0',
      deploymentRevision: 'revision',
      deploymentArtifactIdentity: `skiff-deployment-artifact-v1:sha256:${'f'.repeat(64)}`
    },
    contract: {
      serviceId: `service/${host}`,
      contractVersion: '1.0.0',
      serviceProtocolIdentity: `skiff-service-protocol-v2:sha256:${host.startsWith('codex') ? '1'.repeat(64) : host.startsWith('aihub') ? '2'.repeat(64) : '3'.repeat(64)}`
    },
    contractOperationId: operation
  };
}

function fakeDispatcher(
  dispatchConnections?: Array<RuntimeDispatchConnection | undefined>
): RuntimeDispatcher {
  return {
    dispatchBinary: async (
      input: { header: RuntimeUnaryDispatchFrameHeader },
      _timeoutMs: number,
      options: { connection?: RuntimeDispatchConnection } = {}
    ) => {
      dispatchConnections?.push(options.connection);
      const validation = validateRuntimeAssemblyRequestStartFrameHeader(input.header);
      if (!validation.ok) {
        throw new Error(validation.error);
      }
      calls.push(validation.envelope);
      return {
        header: {
          schemaVersion: 'skiff-runtime-frame-v1',
          type: 'response.end',
          requestId: validation.envelope.requestId,
          payloadPresent: false,
          ...(validation.envelope.websocketAdapter?.kind === 'connect'
            ? {
                websocketConnect: {
                  result: 'accept' as const,
                  contextPayloadPresent: true,
                  contextCodec: {
                    operationAbiId: validation.envelope.routing.contractOperationId,
                    contextTypeIdentity: 'skiff-contract-type-v1:sha256:context'
                  }
                }
              }
            : {})
        },
        payloadBytes: new Uint8Array()
      };
    }
  } as unknown as RuntimeDispatcher;
}

function runtimeDispatchConnection(runtimeId: string): RuntimeDispatchConnection {
  return { runtimeId, ws: {} as WebSocket };
}

function operationIdentity(character: string): string {
  return `skiff-contract-operation-v1:sha256:${character.repeat(64)}`;
}

async function httpGet(
  baseUrl: string,
  host: string,
  headers: Record<string, string> = {},
  query = ''
): Promise<{ status: number; body: string }> {
  const base = new URL(baseUrl);
  return await new Promise((resolve, reject) => {
    const outgoing = request(
      {
        hostname: base.hostname,
        port: base.port,
        path: `/v1/models${query}`,
        method: 'GET',
        headers: { host, ...headers }
      },
      (response) => {
        const chunks: Buffer[] = [];
        response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
        response.on('end', () => resolve({
          status: response.statusCode ?? 0,
          body: Buffer.concat(chunks).toString('utf8')
        }));
      }
    );
    outgoing.on('error', reject);
    outgoing.end();
  });
}

async function openWebSocket(baseUrl: string, host: string, path: string): Promise<WebSocket> {
  const base = new URL(baseUrl);
  const ws = new WebSocket(`ws://${base.hostname}:${base.port}${path}`, {
    headers: {
      Host: host,
      'X-Skiff-Service': 'wrong/service',
      'X-Skiff-Version': 'wrong-version'
    }
  });
  await new Promise<void>((resolve, reject) => {
    ws.once('open', () => {
      resolve();
    });
    ws.once('error', reject);
  });
  return ws;
}

function webSocketResource(ws: WebSocket): { close(): Promise<void> } {
  return {
    close: async () => {
      if (ws.readyState === WebSocket.CLOSED) return;
      const closed = new Promise<void>((resolve) => ws.once('close', () => resolve()));
      ws.close();
      await closed;
    }
  };
}

function connectionSendSource() {
  let handler: ConnectionSendHandler | undefined;
  return {
    onConnectionSend(next: ConnectionSendHandler) {
      handler = next;
      return () => {
        if (handler === next) handler = undefined;
      };
    },
    emit(message: ConnectionSendEnvelope) {
      handler?.(message, {} as WebSocket);
    }
  };
}

async function waitFor(predicate: () => boolean, label: string): Promise<void> {
  const deadline = Date.now() + 1_000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`timed out waiting for ${label}`);
    await delay(5);
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function httpWithoutHost(baseUrl: string): Promise<number> {
  const base = new URL(baseUrl);
  return await new Promise<number>((resolve, reject) => {
    const socket = connect(Number(base.port), base.hostname);
    let response = '';
    socket.setEncoding('utf8');
    socket.once('connect', () => socket.write('GET /v1/models HTTP/1.0\r\n\r\n'));
    socket.on('data', (chunk) => {
      response += chunk;
    });
    socket.once('end', () => {
      const match = /^HTTP\/1\.1 (\d{3})/.exec(response);
      resolve(Number(match?.[1] ?? 0));
    });
    socket.once('error', reject);
  });
}
