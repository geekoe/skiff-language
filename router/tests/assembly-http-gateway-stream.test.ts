import { EventEmitter } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { request as httpRequest } from 'node:http';
import type { ServerResponse } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import WebSocket from 'ws';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  decodeBinaryFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION
} from '../src/protocol/envelope.js';
import { runtimeFrameHeaderFixtures } from '../src/protocol/runtimeProtocol.js';
import { AssemblyHttpGateway } from '../src/router/assemblyHttpGateway.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { HttpStreamResponseWriter } from '../src/router/httpStreamResponseWriter.js';
import {
  RuntimeDispatcher,
  type PendingTerminal
} from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type LoadedRuntimeAssembly,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';
import { writeCurrentScopeCompilerGeneratedArtifactRoot } from './helpers/compilerArtifacts.js';

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'b'.repeat(64)}`;
const RUNTIME_ID = 'runtime-assembly-stream';
const HOST = 'stream.example.test';
const PATH = '/events';

const binding: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', method: 'POST', path: PATH },
  deployment: {
    serviceId: 'example.com/stream',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity:
      `skiff-deployment-artifact-v4:sha256:${'c'.repeat(64)}`
  },
  gatewayEntryKey: 'events',
  gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
  adapterKind: 'rawHttp',
  operationMode: 'serverStream'
};

const fixtures: StreamFixture[] = [];
let currentScopeRoot: string;
let currentScopeAssembly: LoadedRuntimeAssembly;

beforeAll(async () => {
  currentScopeRoot = await mkdtemp(
    join(tmpdir(), 'skiff-router-current-scope-stream-')
  );
  const generated =
    await writeCurrentScopeCompilerGeneratedArtifactRoot(currentScopeRoot);
  currentScopeAssembly = await new FilesystemRuntimeAssemblySnapshotLoader(
    currentScopeRoot
  ).load(generated.receipt.baseAssembly);
}, 120_000);

afterAll(async () => {
  await rm(currentScopeRoot, { recursive: true, force: true });
});

afterEach(async () => {
  while (fixtures.length > 0) {
    await fixtures.pop()!.close();
  }
});

describe('RuntimeAssembly HTTP serverStream ingress', () => {
  it('dispatches the exact S0 server-stream binding to one observable terminal', async () => {
    const exact = currentScopeAssembly.gatewayIngress.find(
      (candidate) =>
        candidate.selector.protocol === 'http' &&
        candidate.selector.path === '/current-scope/stream'
    );
    if (exact === undefined) {
      throw new Error('current-scope server-stream binding is missing');
    }
    const fixture = await createFixture({}, {
      assemblyIdentity: currentScopeAssembly.assemblyIdentity,
      generation: 1,
      binding: exact
    });
    const response = sendHttp(
      fixture.url,
      Buffer.from([9, 8, 7]),
      {
        ...exact.selector,
        serviceId: exact.deployment.serviceId,
        contractVersion: exact.deployment.contractVersion
      }
    );
    const requestFrame = decodeBinaryFrame(
      await nextBinaryMessage(fixture.runtime)
    );
    expect(requestFrame.header).toMatchObject({
      type: 'request.start',
      mode: 'serverStream',
      routing: {
        assemblyIdentity: currentScopeAssembly.assemblyIdentity,
        assemblyGeneration: 1,
        gatewayEntryIdentity:
          'skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0',
        ingress: exact.selector
      }
    });
    const requestId = String(requestFrame.header.requestId);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.start',
      requestId,
      httpResponse: {
        status: 206,
        headers: [{ name: 'x-source-receipt', value: 'current-scope' }]
      }
    }));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 0
    }, Buffer.from([1, 2])));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 1
    }, Buffer.from([3, 4])));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: false
    }));

    await expect(response).resolves.toEqual({
      status: 206,
      headers: expect.objectContaining({
        'x-source-receipt': 'current-scope'
      }),
      body: Buffer.from([1, 2, 3, 4])
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(fixture.gateway.streamLifecycleCounters()).toEqual({
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 0
    });
  });

  it('selects the exact gateway binding and preserves ordered binary chunks', async () => {
    const fixture = await createFixture();
    const response = sendHttp(fixture.url, Buffer.from([9, 8, 7]));
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    expect(requestFrame.header).toMatchObject({
      type: 'request.start',
      mode: 'serverStream',
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 4,
        deployment: binding.deployment,
        gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
        ingress: {
          protocol: 'http',
          method: 'POST',
          path: PATH
        }
      }
    });
    expect(Buffer.from(requestFrame.payloadBytes)).toEqual(Buffer.from([9, 8, 7]));
    const requestId = String(requestFrame.header.requestId);

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.start',
      requestId,
      httpResponse: {
        status: 202,
        headers: [
          { name: 'content-type', value: 'application/octet-stream' },
          { name: 'x-stream-mode', value: 'serverStream' }
        ]
      }
    }));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 0
    }, Buffer.from([0, 255])));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 1
    }, Buffer.from([17, 128])));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: false
    }));

    await expect(response).resolves.toEqual({
      status: 202,
      headers: expect.objectContaining({
        'content-type': 'application/octet-stream',
        'x-stream-mode': 'serverStream'
      }),
      body: Buffer.from([0, 255, 17, 128])
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(fixture.gateway.streamLifecycleCounters()).toEqual({
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 0
    });
  });

  it('enforces maxResponseBytes cumulatively across streaming chunks', async () => {
    const fixture = await createFixture({ maxResponseBytes: 3 });
    const response = sendHttp(fixture.url, Buffer.alloc(0));
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const requestId = String(requestFrame.header.requestId);

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.start',
      requestId,
      httpResponse: { status: 200, headers: [] }
    }));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 0
    }, Buffer.from([1, 2])));
    const cancelFrame = nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.chunk',
      requestId,
      seq: 1
    }, Buffer.from([3, 4])));

    await expect(response).resolves.toMatchObject({
      status: 200,
      body: Buffer.from([1, 2])
    });
    expect(decodeBinaryFrame(await cancelFrame).header).toMatchObject({
      type: 'request.cancel',
      requestId
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('fails closed on invalid start, chunk, and end ordering or payload metadata', async () => {
    const cases: Array<{
      name: string;
      send(runtime: WebSocket, requestId: string): void;
    }> = [
      {
        name: 'chunk before start',
        send: (runtime, requestId) => runtime.send(encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId,
          seq: 0
        }, Buffer.from([1])))
      },
      {
        name: 'end before start',
        send: (runtime, requestId) => runtime.send(encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId,
          payloadPresent: false
        }))
      },
      {
        name: 'duplicate start',
        send: (runtime, requestId) => {
          const start = encodeRuntimeFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.start',
            requestId,
            httpResponse: { status: 200, headers: [] }
          });
          runtime.send(start);
          runtime.send(start);
        }
      },
      {
        name: 'start payload',
        send: (runtime, requestId) => runtime.send(encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.start',
          requestId,
          httpResponse: { status: 200, headers: [] }
        }, Buffer.from([1])))
      },
      {
        name: 'end payload',
        send: (runtime, requestId) => {
          runtime.send(encodeRuntimeFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.start',
            requestId,
            httpResponse: { status: 200, headers: [] }
          }));
          runtime.send(encodeRuntimeFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.end',
            requestId,
            payloadPresent: true
          }, Buffer.from([1])));
        }
      },
      {
        name: 'end metadata',
        send: (runtime, requestId) => {
          runtime.send(encodeRuntimeFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.start',
            requestId,
            httpResponse: { status: 200, headers: [] }
          }));
          runtime.send(encodeRuntimeFrame({
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.end',
            requestId,
            payloadPresent: false,
            httpResponse: { status: 200, headers: [] }
          }));
        }
      }
    ];

    for (const invalidCase of cases) {
      const fixture = await createFixture();
      const response = sendHttp(fixture.url, Buffer.alloc(0));
      const requestFrame = decodeBinaryFrame(
        await nextBinaryMessage(fixture.runtime)
      );
      const requestId = String(requestFrame.header.requestId);
      const cancelFrame = nextBinaryMessage(fixture.runtime);
      invalidCase.send(fixture.runtime, requestId);

      await expect(response, invalidCase.name).resolves.toMatchObject({
        body: expect.any(Buffer)
      });
      expect(
        decodeBinaryFrame(await cancelFrame).header,
        invalidCase.name
      ).toMatchObject({
        type: 'request.cancel',
        requestId
      });
      expect(
        fixture.dispatcher.pendingLifecycleCounters(),
        invalidCase.name
      ).toEqual({
        pendingUnary: 0,
        pendingStream: 0
      });
    }
  });

  it('uses the effective timeout while waiting for response.start', async () => {
    const fixture = await createFixture({ requestTimeoutMs: 40 });
    const response = sendHttp(fixture.url, Buffer.alloc(0));
    const requestFrame = decodeBinaryFrame(
      await nextBinaryMessage(fixture.runtime)
    );
    const requestId = String(requestFrame.header.requestId);
    const cancelFrame = nextBinaryMessage(fixture.runtime);

    await expect(response).resolves.toMatchObject({ status: 504 });
    expect(decodeBinaryFrame(await cancelFrame).header).toMatchObject({
      type: 'request.cancel',
      requestId
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('cancels a streaming Runtime request when the HTTP client disconnects', async () => {
    const fixture = await createFixture();
    const pendingHttp = startHttp(fixture.url, Buffer.from([1, 2, 3]));
    const responseOutcome = pendingHttp.response.catch((error: unknown) => error);
    const requestFrame = decodeBinaryFrame(
      await nextBinaryMessage(fixture.runtime)
    );
    const requestId = String(requestFrame.header.requestId);
    const cancelFrame = nextBinaryMessage(fixture.runtime);
    pendingHttp.request.destroy();

    expect(await responseOutcome).toBeInstanceOf(Error);
    expect(decodeBinaryFrame(await cancelFrame).header).toMatchObject({
      type: 'request.cancel',
      requestId
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(fixture.gateway.streamLifecycleCounters()).toEqual({
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 0
    });
  });

  it('turns backpressure drain timeout into one terminal cancel', async () => {
    const response = Object.assign(new EventEmitter(), {
      destroyed: false,
      headersSent: true,
      writableEnded: false,
      write: vi.fn(() => false),
      end: vi.fn((callback?: () => void) => callback?.())
    }) as unknown as ServerResponse;
    const counters = {
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 0
    };
    const writer = new HttpStreamResponseWriter({
      response,
      clientDisconnectSignal: new AbortController().signal,
      backpressureDrainTimeoutMs: 10,
      counters,
      maxResponseBytes: 100,
      writeHeaders: () => undefined
    });
    const terminal = new Promise<PendingTerminal>((resolve) => {
      writer.enqueueChunk(
        {
          header: {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.chunk',
            requestId: 'backpressure',
            seq: 0
          },
          payloadBytes: new Uint8Array([1])
        },
        resolve
      );
    });

    const result = await terminal;
    expect(result).toMatchObject({
      source: 'backpressure',
      kind: 'cancelled'
    });
    await new Promise<void>((resolve) => setImmediate(resolve));
    writer.closeFromPendingTerminal(result);
    expect(counters).toEqual({
      activeWriters: 0,
      backpressureWaiters: 0,
      backpressureCancels: 1
    });
  });

  it('maps a fixed stream failure before response.start without exposing its payload', async () => {
    const fixture = await createFixture();
    const response = sendHttp(fixture.url, Buffer.alloc(0));
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const payloadBytes = Buffer.from(JSON.stringify({
      kind: 'internalError',
      payload: {
        message:
          'provider-private-secret /callee/private/source.skiff calleePrivateFunction stack',
        traceId: 'trace-stream-fixed',
        errorId: 'error-stream-fixed'
      }
    }), 'utf8');

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: String(requestFrame.header.requestId),
      errorKind: 'fixedService'
    }, payloadBytes));

    const completed = await response;
    expect(completed.status).toBe(500);
    expect(JSON.parse(completed.body.toString())).toEqual({
      error: {
        code: 'FixedServiceError',
        message: 'Service request failed',
        details: {
          traceId: 'trace-stream-fixed',
          errorId: 'error-stream-fixed'
        }
      }
    });
    expect(completed.body.toString()).not.toContain('provider-private-secret');
    expect(completed.body.toString()).not.toContain('/callee/private/source.skiff');
    expect(completed.body.toString()).not.toContain('calleePrivateFunction');
    expect(completed.body.toString()).not.toContain('stack');
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });
});

interface StreamFixture {
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  gateway: AssemblyHttpGateway;
  runtime: WebSocket;
  url: string;
  close(): Promise<void>;
}

async function createFixture(
  limits: {
    maxRequestBytes?: number;
    maxResponseBytes?: number;
    requestTimeoutMs?: number;
  } = {},
  active: {
    assemblyIdentity: string;
    generation: number;
    binding: RuntimeAssemblyIngressBinding;
  } = {
    assemblyIdentity: ASSEMBLY,
    generation: 4,
    binding
  }
): Promise<StreamFixture> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: active.generation,
    assembly: { assemblyIdentity: active.assemblyIdentity },
    configSnapshot: {
      snapshotId:
        'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    resolvedDeployments: [active.binding.deployment],
    resolvedContracts: [{
      serviceId: active.binding.deployment.serviceId,
      contractVersion: active.binding.deployment.contractVersion,
      serviceProtocolIdentity:
        `skiff-service-protocol-v5:sha256:${'c'.repeat(64)}`
    }],
    deploymentRuntimeBindings: [{
      deployment: active.binding.deployment,
      packageBuildId:
        `skiff-package-build-v10:sha256:${'f'.repeat(64)}`
    }],
    ingress: new RuntimeAssemblyIngressIndex([active.binding])
  });
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const endpoint = new RuntimeEndpoint({
    registry: new RuntimeRegistry(),
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
      http: { maxResponseBytes: 67108864 },
      activation: {
        environment: 'test',
        generation: active.generation,
        assembly: { assemblyIdentity: active.assemblyIdentity },
        configSnapshot: {
          snapshotId:
            'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
        }
      }
    }
  });
  const dispatcher = new RuntimeDispatcher({
    registry: assemblyRegistry,
    frameSender: endpoint,
    maxConcurrency: 64
  });
  endpoint.setDispatcher(dispatcher);
  const endpointAddress = await endpoint.listen({ port: 0 });
  const gateway = new AssemblyHttpGateway({
    snapshots,
    dispatcher,
    port: 0,
    maxRequestBytes: limits.maxRequestBytes ?? 67108864,
    maxResponseBytes: limits.maxResponseBytes ?? 67108864,
    requestTimeoutMs: limits.requestTimeoutMs ?? 1_000
  });
  const gatewayAddress = await gateway.listen();
  const runtime = await openSocket(endpointAddress.url);
  runtime.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId: RUNTIME_ID
  }));
  runtime.send(encodeAssemblyActivationFrame('runtimeToRouter', {
    type: 'register',
    environment: 'test',
    generation: active.generation,
    assembly: { assemblyIdentity: active.assemblyIdentity },
    configSnapshot: {
      snapshotId:
        'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    },
    replicaId: RUNTIME_ID
  }));
  await until(() =>
    assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID)
  );
  const fixture: StreamFixture = {
    dispatcher,
    endpoint,
    gateway,
    runtime,
    url: gatewayAddress.url,
    close: async () => {
      await gateway.close();
      await endpoint.close();
    }
  };
  fixtures.push(fixture);
  return fixture;
}

async function sendHttp(
  baseUrl: string,
  body: Buffer,
  selector: {
    method: string | null;
    path: string;
    serviceId?: string;
    contractVersion?: string;
  } = { method: 'POST', path: PATH }
): Promise<{
  status: number;
  headers: Record<string, string | string[] | undefined>;
  body: Buffer;
}> {
  return await startHttp(baseUrl, body, selector).response;
}

function startHttp(
  baseUrl: string,
  body: Buffer,
  selector: {
    method: string | null;
    path: string;
    serviceId?: string;
    contractVersion?: string;
  } = { method: 'POST', path: PATH }
): {
  request: ReturnType<typeof httpRequest>;
  response: Promise<{
    status: number;
    headers: Record<string, string | string[] | undefined>;
    body: Buffer;
  }>;
} {
  const url = new URL(selector.path, baseUrl);
  let request: ReturnType<typeof httpRequest>;
  const response = new Promise<{
    status: number;
    headers: Record<string, string | string[] | undefined>;
    body: Buffer;
  }>((resolve, reject) => {
    request = httpRequest(url, {
      method: selector.method ?? 'POST',
      headers: {
        Host: HOST,
        'x-skiff-service': selector.serviceId ?? binding.deployment.serviceId,
        'x-skiff-version':
          selector.contractVersion ?? binding.deployment.contractVersion,
        'content-length': String(body.byteLength)
      }
    }, (response) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.once('end', () => resolve({
        status: response.statusCode ?? 0,
        headers: response.headers,
        body: Buffer.concat(chunks)
      }));
    });
    request.once('error', reject);
    request.end(body);
  });
  return { request: request!, response };
}

async function openSocket(url: string): Promise<WebSocket> {
  const ws = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
  return ws;
}

async function nextBinaryMessage(ws: WebSocket): Promise<Buffer> {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error('timed out waiting for RuntimeAssembly request'));
    }, 1_000);
    const onMessage = (data: WebSocket.RawData, isBinary: boolean) => {
      clearTimeout(timeout);
      if (!isBinary) {
        cleanup();
        reject(new Error('expected binary runtime frame'));
        return;
      }
      const buffer = Array.isArray(data)
        ? Buffer.concat(data)
        : data instanceof ArrayBuffer
          ? Buffer.from(new Uint8Array(data))
          : Buffer.from(data.buffer, data.byteOffset, data.byteLength);
      try {
        if (decodeBinaryFrame(buffer).header.type === 'runtime.registered') {
          // Skip the registered ACK handshake frame.
          return;
        }
      } catch {
        // Not a decodable binary frame; pass through.
      }
      cleanup();
      resolve(buffer);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

async function until(predicate: () => boolean): Promise<void> {
  const deadline = Date.now() + 1_000;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error('timed out waiting for runtime registration');
    }
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
}
