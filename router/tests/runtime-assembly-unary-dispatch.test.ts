import { request as httpRequest } from 'node:http';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  decodeBinaryFrame,
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION
} from '../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeAssemblyRequest.js';
import {
  runtimeFrameHeaderFixtures,
  validateRuntimeAssemblyRequestStartFrameHeader
} from '../src/protocol/runtimeProtocol.js';
import {
  assemblyHttpRequestHeader,
  AssemblyHttpGateway
} from '../src/router/assemblyHttpGateway.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import {
  FixedServiceResponseError,
  RuntimeResponseError
} from '../src/router/errors.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import type { RuntimeUnaryDispatchFrameHeader } from '../src/router/runtimeRegistry.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v1:sha256:${'a'.repeat(64)}`;
const OPERATION = `skiff-contract-operation-v1:sha256:${'b'.repeat(64)}`;
const RUNTIME_ID = 'runtime-unary-a';
const HOST = 'api.localhost';
const PATH = '/v1/invoke';
const PRIVATE_SENTINELS = [
  'provider-private-secret',
  '/callee/private/source.skiff',
  'calleePrivateFunction',
  'sourceFrames',
  'stack'
] as const;
const fixtures: UnaryFixture[] = [];

describe('RuntimeAssembly canonical HTTP unary dispatch', () => {
  afterEach(async () => {
    while (fixtures.length > 0) {
      await fixtures.pop()!.close();
    }
  });

  it('writes validator-accepted nested headers and preserves zero and opaque payloads', async () => {
    const fixture = await createFixture();

    const zeroResponse = sendHttp(fixture.httpUrl, new Uint8Array());
    const zeroFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const zeroValidation = validateRuntimeAssemblyRequestStartFrameHeader(zeroFrame.header);
    expect(zeroValidation).toMatchObject({ ok: true });
    if (!zeroValidation.ok) throw new Error(zeroValidation.error);
    expect(zeroValidation.envelope).toMatchObject({
      type: 'request.start',
      mode: 'unary',
      caller: { kind: 'gateway' },
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 7,
        contractOperationId: OPERATION,
        ingress: { protocol: 'http', host: HOST, method: 'POST', path: PATH }
      },
      httpRequest: {
        method: 'POST',
        path: PATH
      },
      testEffectsEnabled: false,
      testEffectDoubles: {}
    });
    expect(zeroValidation.envelope).not.toHaveProperty('target');
    expect(zeroValidation.envelope).not.toHaveProperty('operationAbiId');
    expect(zeroValidation.envelope).not.toHaveProperty('buildId');
    expect(zeroValidation.envelope).not.toHaveProperty('serviceProtocolIdentity');
    expect(zeroValidation.envelope).not.toHaveProperty('assemblyIdentity');
    expect(zeroValidation.envelope).not.toHaveProperty('assemblyGeneration');
    expect(zeroValidation.envelope).not.toHaveProperty('contractOperationId');
    expect(zeroFrame.payloadBytes).toHaveLength(0);

    const opaqueResponseBytes = new Uint8Array([0, 255, 17, 128]);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: zeroValidation.envelope.requestId,
      payloadPresent: true
    }, opaqueResponseBytes));
    const completedZero = await zeroResponse;
    expect(completedZero.status).toBe(200);
    expect(completedZero.headers['content-type']).toBeUndefined();
    expect(completedZero.body).toEqual(Buffer.from(opaqueResponseBytes));

    const opaqueRequestBytes = new Uint8Array([123, 0, 255, 34]);
    const opaqueResponse = sendHttp(fixture.httpUrl, opaqueRequestBytes, '?mode=opaque');
    const opaqueFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const opaqueValidation = validateRuntimeAssemblyRequestStartFrameHeader(opaqueFrame.header);
    expect(opaqueValidation).toMatchObject({ ok: true });
    if (!opaqueValidation.ok) throw new Error(opaqueValidation.error);
    expect(Buffer.from(opaqueFrame.payloadBytes)).toEqual(Buffer.from(opaqueRequestBytes));
    expect(new URL(opaqueValidation.envelope.httpRequest!.url).host).toBe(HOST);
    expect(opaqueValidation.envelope.httpRequest).toMatchObject({
      method: opaqueValidation.envelope.routing.ingress.method,
      path: opaqueValidation.envelope.routing.ingress.path
    });

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: opaqueValidation.envelope.requestId,
      payloadPresent: true,
      httpResponse: {
        status: 201,
        headers: [{ name: 'x-runtime-result', value: 'opaque' }]
      }
    }, new Uint8Array([9, 8, 7])));
    const completedOpaque = await opaqueResponse;
    expect(completedOpaque.status).toBe(201);
    expect(completedOpaque.headers['x-runtime-result']).toBe('opaque');
    expect(completedOpaque.body).toEqual(Buffer.from([9, 8, 7]));
  });

  it('rejects oversized requests before Runtime dispatch', async () => {
    const fixture = await createFixture({ maxRequestBytes: 3 });

    const response = await sendHttp(fixture.httpUrl, new Uint8Array([1, 2, 3, 4]));
    expect(response.status).toBe(413);
    expect(JSON.parse(response.body.toString())).toMatchObject({
      error: { code: 'RequestTooLarge' }
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('rejects an oversized unary Runtime response at the Router boundary', async () => {
    const fixture = await createFixture({ maxResponseBytes: 3 });
    const response = sendHttp(fixture.httpUrl, new Uint8Array());
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: String(requestFrame.header.requestId),
      payloadPresent: true
    }, new Uint8Array([1, 2, 3, 4])));

    const completed = await response;
    expect(completed).toMatchObject({
      status: 502,
      body: expect.any(Buffer)
    });
    expect(JSON.parse(completed.body.toString())).toMatchObject({
      error: { code: 'ResponseTooLarge' }
    });
  });

  it('forwards fixed and control unaryFrame errors with exact v2 headers and bytes', async () => {
    const fixture = await createFixture();
    for (const [index, kind] of (
      ['publicTypedError', 'internalError', 'platformError'] as const
    ).entries()) {
      const requestId = `unary-frame-fixed-${kind}`;
      const dispatch = fixture.dispatcher.dispatchBinaryFrame(
        {
          header: canonicalHeader(fixture.snapshot, requestId),
          payloadBytes: new Uint8Array([index, 255 - index])
        },
        1_000
      );
      const request = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
      expect(request.header.requestId).toBe(requestId);
      const header = {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId,
        errorKind: 'fixedService'
      } as const;
      const payloadBytes = fixedServicePayload(
        kind,
        `trace-frame-${index}`,
        `error-frame-${index}`
      );
      fixture.runtime.send(encodeRuntimeFrame(header, payloadBytes));

      const result = await dispatch;
      expect(result.header).toEqual(header);
      expect(Buffer.from(result.payloadBytes)).toEqual(Buffer.from(payloadBytes));
    }

    const requestId = 'unary-frame-control';
    const dispatch = fixture.dispatcher.dispatchBinaryFrame(
      {
        header: canonicalHeader(fixture.snapshot, requestId),
        payloadBytes: new Uint8Array()
      },
      1_000
    );
    await nextBinaryMessage(fixture.runtime);
    const header = {
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId,
      errorKind: 'control',
      error: {
        code: 'InternalError',
        message: 'The service could not complete the request.',
        status: 500,
        details: { traceId: 'control-only-trace' }
      }
    } as const;
    fixture.runtime.send(encodeRuntimeFrame(header));

    const result = await dispatch;
    expect(result.header).toEqual(header);
    expect(result.payloadBytes).toHaveLength(0);
  });

  it('uses mutually exclusive fixed and control errors for ordinary pending requests', async () => {
    const fixture = await createFixture();
    const fixedHeader = canonicalHeader(fixture.snapshot, 'ordinary-fixed');
    const fixedDispatch = fixture.dispatcher
      .dispatchBinary(
        { header: fixedHeader, payloadBytes: new Uint8Array() },
        1_000
      )
      .catch((error: unknown) => error);
    await nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame(
      {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: fixedHeader.requestId,
        errorKind: 'fixedService'
      },
      fixedServicePayload('internalError', 'trace-ordinary-fixed', 'error-ordinary-fixed')
    ));
    const fixedError = await fixedDispatch;
    expect(fixedError).toBeInstanceOf(FixedServiceResponseError);
    expect(fixedError).not.toBeInstanceOf(RuntimeResponseError);
    expect(fixedError).toMatchObject({
      serviceErrorKind: 'internalError',
      traceId: 'trace-ordinary-fixed',
      errorId: 'error-ordinary-fixed'
    });

    const controlHeader = canonicalHeader(fixture.snapshot, 'ordinary-control');
    const controlDispatch = fixture.dispatcher
      .dispatchBinary(
        { header: controlHeader, payloadBytes: new Uint8Array() },
        1_000
      )
      .catch((error: unknown) => error);
    await nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: controlHeader.requestId,
      errorKind: 'control',
      error: {
        code: 'InternalError',
        message: 'The service could not complete the request.',
        status: 500
      }
    }));
    const controlError = await controlDispatch;
    expect(controlError).toBeInstanceOf(RuntimeResponseError);
    expect(controlError).not.toBeInstanceOf(FixedServiceResponseError);
  });

  it('maps every fixed kind to one redacted HTTP 5xx fact and redacts generic 5xx details', async () => {
    const fixture = await createFixture();
    for (const [index, kind] of (
      ['publicTypedError', 'internalError', 'platformError'] as const
    ).entries()) {
      const traceId = `trace-http-${index}`;
      const errorId = `error-http-${index}`;
      const response = sendHttp(fixture.httpUrl, new Uint8Array());
      const request = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
      fixture.runtime.send(encodeRuntimeFrame(
        {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: String(request.header.requestId),
          errorKind: 'fixedService'
        },
        fixedServicePayload(kind, traceId, errorId)
      ));

      const completed = await response;
      expect(completed.status).toBe(500);
      expect(JSON.parse(completed.body.toString())).toEqual({
        error: {
          code: 'FixedServiceError',
          message: 'Service request failed',
          details: { traceId, errorId }
        }
      });
      assertNoPrivateSentinels(completed.body.toString());
    }

    const controlResponse = sendHttp(fixture.httpUrl, new Uint8Array());
    const controlRequest = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: String(controlRequest.header.requestId),
      errorKind: 'control',
      error: {
        code: 'std.service.ProtocolError',
        message: 'runtime protocol failure',
        status: 502,
        details: {
          private: PRIVATE_SENTINELS.join('|')
        }
      }
    }));
    const completedControl = await controlResponse;
    expect(completedControl.status).toBe(502);
    expect(JSON.parse(completedControl.body.toString())).toEqual({
      error: {
        code: 'std.service.ProtocolError',
        message: 'runtime protocol failure'
      }
    });
    assertNoPrivateSentinels(completedControl.body.toString());
  });

  it('fails closed before the socket for legacy, flat, unknown, stream, adapter, and HTTP mismatches', async () => {
    const fixture = await createFixture();
    const valid = canonicalHeader(fixture.snapshot, 'valid');
    const legacy = mutate(valid, (header) => {
      header.assemblyIdentity = header.routing.assemblyIdentity;
      header.assemblyGeneration = header.routing.assemblyGeneration;
      header.contractOperationId = header.routing.contractOperationId;
      delete header.routing;
    });
    const invalid: RuntimeUnaryDispatchFrameHeader[] = [
      legacy,
      mutate(valid, (header) => {
        header.assemblyIdentity = ASSEMBLY;
      }),
      mutate(valid, (header) => {
        header.unknown = true;
      }),
      mutate(valid, (header) => {
        header.mode = 'serverStream';
      }),
      mutate(valid, (header) => {
        header.httpAdapter = {
          kind: 'rawHttp',
          handler: { kind: 'serviceFunction', modulePath: 'service', symbol: 'invoke' },
          adapterArgs: []
        };
      }),
      mutate(valid, (header) => {
        delete header.httpRequest;
      }),
      mutate(valid, (header) => {
        header.gatewayEntryIdentity = `skiff-gateway-v1:sha256:${'e'.repeat(64)}`;
        header.websocketEntryId = 'entry-a';
        header.websocketAdapter = {
          kind: 'connect',
          adapterArgs: [],
          connectRequest: {
            connectionId: 'connection-a',
            url: `ws://${HOST}${PATH}`,
            query: [],
            headers: [],
            cookies: []
          }
        };
      }),
      mutate(valid, (header) => {
        header.httpRequest.method = 'GET';
      }),
      mutate(valid, (header) => {
        header.httpRequest.path = '/wrong';
      }),
      mutate(valid, (header) => {
        header.httpRequest.url = 'http://wrong.localhost/v1/invoke';
      }),
      mutate(valid, (header) => {
        header.testEffectsEnabled = true;
      }),
      mutate(valid, (header) => {
        header.testEffectDoubles = { effect: [{ response: null }] };
      })
    ];

    let ordinal = 0;
    for (const header of invalid) {
      ordinal += 1;
      header.requestId = `invalid-${ordinal}`;
      await expect(fixture.dispatcher.dispatchBinary({
        header,
        payloadBytes: new Uint8Array([ordinal])
      }, 100)).rejects.toThrow();
    }
    await nextTurn();
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });

    const timeoutHeader = canonicalHeader(fixture.snapshot, 'timeout');
    const timeoutDispatch = fixture.dispatcher.dispatchBinary({
      header: timeoutHeader,
      payloadBytes: new Uint8Array()
    }, 50);
    await nextBinaryMessage(fixture.runtime);
    const timeoutCancelPromise = nextBinaryMessage(fixture.runtime);
    await expect(timeoutDispatch).rejects.toThrow(/within 50ms/);
    const timeoutCancel = decodeRuntimeFrame(await timeoutCancelPromise);
    expect(timeoutCancel.header).toMatchObject({
      type: 'request.cancel',
      requestId: timeoutHeader.requestId
    });
    expect(timeoutCancel.payloadBytes).toHaveLength(0);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('keeps caller-abort cancel on the same request and socket with an empty payload', async () => {
    const fixture = await createFixture();
    const controller = new AbortController();
    const header = canonicalHeader(fixture.snapshot, 'caller-abort');
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000, { signal: controller.signal });
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    expect(requestFrame.header.requestId).toBe(header.requestId);
    const cancelFramePromise = nextBinaryMessage(fixture.runtime);
    controller.abort();
    await expect(dispatch).rejects.toThrow(/cancelled before completion/);
    const cancelFrame = decodeRuntimeFrame(await cancelFramePromise);
    expect(cancelFrame.header).toMatchObject({
      type: 'request.cancel',
      requestId: header.requestId
    });
    expect(cancelFrame.payloadBytes).toHaveLength(0);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('rejects response.start for unary once, cancels once, and ignores a late terminal', async () => {
    const fixture = await createFixture();
    const header = canonicalHeader(fixture.snapshot, 'unexpected-start');
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);
    const cancelFramePromise = nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.start',
      requestId: header.requestId,
      httpResponse: { status: 200, headers: [] }
    }));
    await expect(dispatch).rejects.toThrow(/response.start is only valid for serverStream/);
    const cancelFrame = decodeRuntimeFrame(await cancelFramePromise);
    expect(cancelFrame.header).toMatchObject({
      type: 'request.cancel',
      requestId: header.requestId
    });
    expect(cancelFrame.payloadBytes).toHaveLength(0);

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: header.requestId,
      payloadPresent: false
    }));
    await nextTurn();
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(fixture.runtime.readyState).toBe(WebSocket.OPEN);

    const errorHeader = canonicalHeader(fixture.snapshot, 'runtime-error');
    const errorDispatch = fixture.dispatcher.dispatchBinary({
      header: errorHeader,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);
    let unexpectedOutboundFrames = 0;
    const countOutbound = () => {
      unexpectedOutboundFrames += 1;
    };
    fixture.runtime.on('message', countOutbound);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: errorHeader.requestId,
      errorKind: 'control',
      error: { code: 'Rejected', message: 'runtime rejected unary request' }
    }));
    await expect(errorDispatch).rejects.toThrow(/runtime rejected unary request/);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: errorHeader.requestId,
      payloadPresent: false
    }));
    await nextTurn();
    fixture.runtime.off('message', countOutbound);
    expect(unexpectedOutboundFrames).toBe(0);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });
});

interface UnaryFixture {
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  gateway: AssemblyHttpGateway;
  httpUrl: string;
  runtime: WebSocket;
  snapshot: RouterActiveAssemblySnapshot;
  close(): Promise<void>;
}

const BINDING: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', host: HOST, method: 'POST', path: PATH },
  deployment: {
    serviceId: 'example/unary',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
    deploymentArtifactIdentity: `skiff-deployment-artifact-v2:sha256:${'c'.repeat(64)}`
  },
  contract: {
    serviceId: 'example/unary',
    contractVersion: '1.0.0',
    serviceProtocolIdentity: `skiff-service-protocol-v3:sha256:${'d'.repeat(64)}`
  },
  operationMode: 'unary',
  contractOperationId: OPERATION
};

async function createFixture(
  limits: { maxRequestBytes?: number; maxResponseBytes?: number } = {}
): Promise<UnaryFixture> {
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 7,
    assembly: { assemblyIdentity: ASSEMBLY },
    ingress: new RuntimeAssemblyIngressIndex([BINDING])
  });
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
  const dispatcher = new RuntimeDispatcher({ registry: assemblyRegistry, frameSender: endpoint });
  endpoint.setDispatcher(dispatcher);
  const runtimeListen = await endpoint.listen({ port: 0 });
  const gateway = new AssemblyHttpGateway({
    snapshots,
    dispatcher,
    port: 0,
    maxRequestBytes: limits.maxRequestBytes ?? 67108864,
    maxResponseBytes: limits.maxResponseBytes ?? 67108864,
    requestTimeoutMs: 1000
  });
  const httpListen = await gateway.listen();
  const runtime = await openSocket(runtimeListen.url);
  runtime.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId: RUNTIME_ID
  }));
  runtime.send(encodeAssemblyActivationFrame('runtimeToRouter', {
    type: 'register',
    environment: 'test',
    generation: 7,
    assembly: { assemblyIdentity: ASSEMBLY },
    replicaId: RUNTIME_ID
  }));
  await until(() => assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID));

  const fixture: UnaryFixture = {
    dispatcher,
    endpoint,
    gateway,
    httpUrl: httpListen.url,
    runtime,
    snapshot: snapshots.get(),
    close: async () => {
      await gateway.close();
      await endpoint.close();
    }
  };
  fixtures.push(fixture);
  return fixture;
}

function canonicalHeader(
  snapshot: RouterActiveAssemblySnapshot,
  requestId: string
): RuntimeAssemblyRequestStartFrameHeader {
  return assemblyHttpRequestHeader({
    snapshot,
    binding: BINDING,
    requestId,
    timeoutMs: 1000,
    httpRequest: requestMetadata()
  });
}

function requestMetadata() {
  return {
    method: 'POST',
    url: `http://${HOST}${PATH}`,
    path: PATH,
    query: [],
    headers: []
  };
}

function mutate(
  source: RuntimeAssemblyRequestStartFrameHeader,
  change: (header: Record<string, any>) => void
): RuntimeAssemblyRequestStartFrameHeader {
  const header = structuredClone(source) as unknown as Record<string, any>;
  change(header);
  return header as unknown as RuntimeAssemblyRequestStartFrameHeader;
}

async function sendHttp(
  baseUrl: string,
  body: Uint8Array,
  query = ''
): Promise<{ status: number; headers: Record<string, string | string[] | undefined>; body: Buffer }> {
  const base = new URL(baseUrl);
  return await new Promise((resolve, reject) => {
    const outgoing = httpRequest({
      hostname: base.hostname,
      port: base.port,
      path: `${PATH}${query}`,
      method: 'POST',
      headers: {
        host: HOST,
        'content-length': String(body.byteLength)
      }
    }, (response) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => resolve({
        status: response.statusCode ?? 0,
        headers: response.headers,
        body: Buffer.concat(chunks)
      }));
    });
    outgoing.on('error', reject);
    outgoing.end(body);
  });
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

function rawDataBuffer(data: WebSocket.RawData): Buffer {
  if (Array.isArray(data)) return Buffer.concat(data);
  if (data instanceof ArrayBuffer) return Buffer.from(new Uint8Array(data));
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
}

async function until(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return;
    await nextTurn();
  }
  throw new Error('condition was not reached');
}

async function nextTurn(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}

function fixedServicePayload(
  kind: 'publicTypedError' | 'internalError' | 'platformError',
  traceId: string,
  errorId: string
): Uint8Array {
  const privateBytes = Array.from(Buffer.from(PRIVATE_SENTINELS.join('|'), 'utf8'));
  const envelope =
    kind === 'publicTypedError'
      ? {
          kind,
          packageId: 'example.com/errors',
          stableSchemaKey: 'private-failure',
          packageSchemaTypeId: 'type:private-failure',
          encodedPayload: privateBytes,
          traceId,
          errorId
        }
      : kind === 'internalError'
        ? {
            kind,
            payload: {
              message: PRIVATE_SENTINELS.join('|'),
              traceId,
              errorId
            }
          }
        : {
            kind,
            builtinErrorIdentity: 'std.db.ConflictError',
            encodedPayload: privateBytes,
            traceId,
            errorId
          };
  return Buffer.from(JSON.stringify(envelope), 'utf8');
}

function assertNoPrivateSentinels(value: string): void {
  for (const sentinel of PRIVATE_SENTINELS) {
    expect(value).not.toContain(sentinel);
  }
}
