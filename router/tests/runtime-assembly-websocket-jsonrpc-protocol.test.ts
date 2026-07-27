import { describe, expect, it } from 'vitest';

import {
  encodeBinaryFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
} from '../src/protocol/envelope.js';
import {
  decodeRuntimeAssemblyRequestStartFrame,
  encodeRuntimeAssemblyRequestStartFrame
} from '../src/protocol/runtimeAssemblyRequestFrame.js';
import {
  decodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame,
  encodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame
} from '../src/protocol/runtimeAssemblyRequestResponseFrame.js';
import type {
  RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
} from '../src/protocol/runtimeAssemblyRequest.js';
import {
  validateRuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
} from '../src/protocol/runtimeProtocol.js';

const METHOD_GATEWAY_IDENTITY =
  'skiff-gateway-entry-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';
const PHYSICAL_WEBSOCKET_ENTRY_ID =
  'skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616';
const REQUEST_PAYLOAD = Buffer.from('{"query":"ready"}', 'utf8');

function canonicalRequest(): RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: 'request-websocket-jsonrpc-1',
    mode: 'unary',
    caller: {
      kind: 'gateway'
    },
    routing: {
      kind: 'runtimeAssembly',
      assemblyIdentity:
        'skiff-runtime-assembly-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
      assemblyGeneration: 11,
      gatewayEntryIdentity: METHOD_GATEWAY_IDENTITY,
      ingress: {
        protocol: 'webSocket',
        host: 'socket.example.com',
        method: 'status.get',
        path: '/chat'
      }
    },
    clientSession: {
      id: 'client-websocket'
    },
    deadline: {
      timeoutMs: 3000,
      expiresAt: '2030-01-01T00:00:03Z'
    },
    trace: {
      traceId: 'trace-websocket',
      spanId: 'span-websocket',
      parentSpanId: 'parent-websocket',
      sampled: true
    },
    websocketJsonRpc: {
      profile: 'jsonrpc-2.0-text',
      connectionId: 'connection-1',
      websocketEntryId: PHYSICAL_WEBSOCKET_ENTRY_ID,
      gatewayEntryIdentity: METHOD_GATEWAY_IDENTITY,
      businessIdentity: 'tenant-1'
    },
    testEffectsEnabled: true
  };
}

function responseHeader(
  outcome: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader['websocketJsonRpc']['outcome']
): RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader {
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId: 'request-websocket-jsonrpc-1',
    payloadPresent: outcome === 'success',
    websocketJsonRpc: {
      outcome
    }
  };
}

describe('runtimeAssembly websocketJsonRpc strict transport wire', () => {
  it('round-trips the canonical method-bearing request without parsing opaque params', () => {
    const header = canonicalRequest();
    expect(validateRuntimeAssemblyRequestStartFrameWireHeader(header)).toMatchObject({
      ok: true
    });

    const encoded = encodeRuntimeAssemblyRequestStartFrame(header, REQUEST_PAYLOAD);
    const decoded = decodeRuntimeAssemblyRequestStartFrame(encoded);
    expect(decoded.header).toEqual(header);
    expect(decoded.payloadBytes).toEqual(REQUEST_PAYLOAD);
    expect(decoded.header.routing.ingress.method).toBe('status.get');
    expect('websocketJsonRpc' in decoded.header).toBe(true);
    expect('websocketConnect' in decoded.header).toBe(false);

    const scalarPayload = Buffer.from('42', 'utf8');
    expect(
      decodeRuntimeAssemblyRequestStartFrame(
        encodeRuntimeAssemblyRequestStartFrame(header, scalarPayload)
      ).payloadBytes
    ).toEqual(scalarPayload);
  });

  it('keeps method null on websocketConnect and method string on websocketJsonRpc', () => {
    const jsonrpc = canonicalRequest() as unknown as Record<string, unknown>;
    const nullMethod = structuredClone(jsonrpc);
    const nullRouting = nullMethod.routing as Record<string, unknown>;
    (nullRouting.ingress as Record<string, unknown>).method = null;
    expect(validateRuntimeAssemblyRequestStartFrameWireHeader(nullMethod)).toMatchObject({
      ok: false
    });

    const connect = {
      ...canonicalRequest(),
      requestId: 'request-websocket-connect-existing',
      routing: {
        ...canonicalRequest().routing,
        gatewayEntryIdentity:
          'skiff-gateway-entry-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
        ingress: {
          ...canonicalRequest().routing.ingress,
          method: null
        }
      },
      websocketConnect: {
        connectionId: 'connection-1',
        url: 'wss://socket.example.com/chat',
        query: [],
        headers: [],
        cookies: [],
        websocketEntryId: PHYSICAL_WEBSOCKET_ENTRY_ID,
        gatewayEntryIdentity:
          'skiff-gateway-entry-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
      }
    } as unknown as Record<string, unknown>;
    delete connect.websocketJsonRpc;
    const decoded = decodeRuntimeAssemblyRequestStartFrame(
      encodeBinaryFrame(connect)
    );
    expect(decoded.header.routing.ingress.method).toBeNull();
    expect('websocketConnect' in decoded.header).toBe(true);
    expect('websocketJsonRpc' in decoded.header).toBe(false);

    const connectWithMethod = structuredClone(connect);
    const connectRouting = connectWithMethod.routing as Record<string, unknown>;
    (connectRouting.ingress as Record<string, unknown>).method = 'status.get';
    expect(
      validateRuntimeAssemblyRequestStartFrameWireHeader(connectWithMethod)
    ).toMatchObject({ ok: false });
  });

  it('rejects the shared request mutation matrix in both validator and decoder', () => {
    const cases: Array<{
      name: string;
      mutate(header: Record<string, unknown>): void;
      payload?: Uint8Array;
    }> = [
      {
        name: 'wrong mode',
        mutate: (header) => {
          header.mode = 'serverStream';
        }
      },
      {
        name: 'non-canonical request id',
        mutate: (header) => {
          header.requestId = ' request-id ';
        }
      },
      {
        name: 'wrong profile',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).profile =
            'jsonrpc-1.0';
        }
      },
      {
        name: 'identity mismatch',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).gatewayEntryIdentity =
            'skiff-gateway-entry-v2:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee';
        }
      },
      {
        name: 'unknown top-level field',
        mutate: (header) => {
          header.peerRequestId = 'must-not-enter-wire';
        }
      },
      {
        name: 'unknown nested field',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).rawSocketId =
            'must-not-enter-wire';
        }
      },
      {
        name: 'empty method',
        mutate: (header) => {
          const routing = header.routing as Record<string, unknown>;
          (routing.ingress as Record<string, unknown>).method = '';
        }
      },
      {
        name: 'oversized method',
        mutate: (header) => {
          const routing = header.routing as Record<string, unknown>;
          (routing.ingress as Record<string, unknown>).method = 'm'.repeat(257);
        }
      },
      {
        name: 'non-canonical connection id',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).connectionId =
            'peer socket id';
        }
      },
      {
        name: 'explicit null business identity',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).businessIdentity =
          null;
        }
      },
      {
        name: 'oversized business identity',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).businessIdentity =
            'b'.repeat(1025);
        }
      },
      {
        name: 'control-character business identity',
        mutate: (header) => {
          (header.websocketJsonRpc as Record<string, unknown>).businessIdentity =
            'tenant\u0085one';
        }
      },
      {
        name: 'missing payload',
        mutate: () => {},
        payload: new Uint8Array()
      },
      {
        name: 'payload above limit',
        mutate: () => {},
        payload: new Uint8Array(1024 * 1024 + 1)
      }
    ];

    for (const testCase of cases) {
      const header = structuredClone(
        canonicalRequest()
      ) as unknown as Record<string, unknown>;
      testCase.mutate(header);
      const validation = validateRuntimeAssemblyRequestStartFrameWireHeader(header);
      if (
        testCase.name !== 'missing payload' &&
        testCase.name !== 'payload above limit'
      ) {
        expect(validation, testCase.name).toMatchObject({ ok: false });
      } else {
        expect(validation, testCase.name).toMatchObject({ ok: true });
      }
      const frame = encodeBinaryFrame(
        header,
        testCase.payload ?? REQUEST_PAYLOAD
      );
      expect(
        () => decodeRuntimeAssemblyRequestStartFrame(frame),
        testCase.name
      ).toThrow();
      expect(
        () =>
          encodeRuntimeAssemblyRequestStartFrame(
            header as unknown as RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
            testCase.payload ?? REQUEST_PAYLOAD
          ),
        `${testCase.name} encoder`
      ).toThrow();
    }
  });

  it('accepts success with JSON null and the exact three payload-free failures', () => {
    const success = responseHeader('success');
    const encoded = encodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
      success,
      Buffer.from('null', 'utf8')
    );
    const decoded =
      decodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(encoded);
    expect(decoded.header).toEqual(success);
    expect(Buffer.from(decoded.payloadBytes).toString('utf8')).toBe('null');

    for (const outcome of [
      'invalidParams',
      'internalError',
      'deadlineExceeded'
    ] as const) {
      const header = responseHeader(outcome);
      const frame = encodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(header);
      expect(
        decodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(frame).header
      ).toEqual(header);
    }
  });

  it('rejects payload mismatch, unknown fields, unknown outcome, and cancelled', () => {
    const cases: Array<{
      name: string;
      header: Record<string, unknown>;
      payload: Uint8Array;
    }> = [
      {
        name: 'success missing payload',
        header: responseHeader('success') as unknown as Record<string, unknown>,
        payload: new Uint8Array()
      },
      {
        name: 'success payload above limit',
        header: responseHeader('success') as unknown as Record<string, unknown>,
        payload: new Uint8Array(1024 * 1024 + 1)
      },
      {
        name: 'error carrying payload',
        header: responseHeader('invalidParams') as unknown as Record<string, unknown>,
        payload: Buffer.from('null', 'utf8')
      },
      {
        name: 'success payloadPresent false',
        header: {
          ...responseHeader('success'),
          payloadPresent: false
        },
        payload: Buffer.from('null', 'utf8')
      },
      {
        name: 'error payloadPresent true',
        header: {
          ...responseHeader('invalidParams'),
          payloadPresent: true
        },
        payload: new Uint8Array()
      },
      {
        name: 'control-character request id',
        header: {
          ...responseHeader('internalError'),
          requestId: 'request\u0085id'
        },
        payload: new Uint8Array()
      },
      {
        name: 'non-canonical request id',
        header: {
          ...responseHeader('internalError'),
          requestId: ' request-id '
        },
        payload: new Uint8Array()
      },
      {
        name: 'unknown outcome',
        header: {
          ...responseHeader('invalidParams'),
          websocketJsonRpc: { outcome: 'unknown' }
        },
        payload: new Uint8Array()
      },
      {
        name: 'cancelled outcome',
        header: {
          ...responseHeader('invalidParams'),
          websocketJsonRpc: { outcome: 'cancelled' }
        },
        payload: new Uint8Array()
      },
      {
        name: 'unknown top-level field',
        header: {
          ...responseHeader('internalError'),
          message: 'must-not-enter-wire'
        },
        payload: new Uint8Array()
      },
      {
        name: 'unknown nested field',
        header: {
          ...responseHeader('internalError'),
          websocketJsonRpc: {
            outcome: 'internalError',
            stack: 'must-not-enter-wire'
          }
        },
        payload: new Uint8Array()
      }
    ];

    for (const testCase of cases) {
      const validation =
        validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader(
          testCase.header
        );
      if (
        testCase.name !== 'success missing payload' &&
        testCase.name !== 'success payload above limit' &&
        testCase.name !== 'error carrying payload'
      ) {
        expect(validation, testCase.name).toMatchObject({ ok: false });
      } else {
        expect(validation, testCase.name).toMatchObject({ ok: true });
      }
      expect(
        () =>
          decodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
            encodeBinaryFrame(testCase.header, testCase.payload)
          ),
        testCase.name
      ).toThrow();
      expect(
        () =>
          encodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
            testCase.header as unknown as RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
            testCase.payload
          ),
        `${testCase.name} encoder`
      ).toThrow();
    }
  });
});
