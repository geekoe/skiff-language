import { describe, expect, it } from 'vitest';

import { RUNTIME_FRAME_SCHEMA_VERSION } from '../src/protocol/envelope.js';
import {
  runtimeFrameHeaderFixtures,
  validateRuntimeToRouterFrameHeader
} from '../src/protocol/runtimeProtocol.js';

describe('strict runtime WebSocket response variants', () => {
  it('rejects accept/reject extra fields, metadata mixing, and nested mutations', () => {
    const base = runtimeFrameHeaderFixtures['response.end'];
    const { httpResponse: _httpResponse, ...webSocketBase } = base;
    const invalid = [
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: false,
          code: 1008
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'reject',
          contextPayloadPresent: false,
          businessIdentity: 'user-a'
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: false,
          extra: true
        }
      },
      {
        ...webSocketBase,
        payloadPresent: true,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: true
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'reject',
          contextPayloadPresent: true
        }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: false,
          connectionPolicy: {
            maxConnections: 1,
            overflow: 'reject-new',
            extra: true
          }
        }
      },
      {
        ...base,
        httpResponse: { status: 200, headers: [], extra: true }
      },
      {
        ...base,
        payloadPresent: false,
        websocketConnect: { result: 'accept', contextPayloadPresent: false }
      },
      {
        ...webSocketBase,
        payloadPresent: false,
        websocketConnect: {
          result: 'accept',
          contextPayloadPresent: true,
          contextCodec: {
            operationAbiId: 'operation:connect',
            contextTypeIdentity: 'type:context'
          }
        }
      },
      {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: base.requestId
      },
      {
        ...runtimeFrameHeaderFixtures['response.start'],
        extra: true
      },
      {
        ...runtimeFrameHeaderFixtures['response.chunk'],
        extra: true
      },
      {
        ...base,
        extra: true
      },
      {
        ...runtimeFrameHeaderFixtures['response.error'],
        extra: true
      },
      {
        ...runtimeFrameHeaderFixtures['response.error'],
        error: {
          ...runtimeFrameHeaderFixtures['response.error'].error,
          extra: true
        }
      },
      {
        ...runtimeFrameHeaderFixtures['connection.send'],
        extra: true
      }
    ];
    for (const candidate of invalid) {
      expect(validateRuntimeToRouterFrameHeader(candidate)).toMatchObject({ ok: false });
    }

    expect(validateRuntimeToRouterFrameHeader({
      ...webSocketBase,
      payloadPresent: true,
      websocketConnect: {
        result: 'accept',
        contextPayloadPresent: true,
        contextCodec: {
          operationAbiId: 'operation:connect',
          contextTypeIdentity: 'type:zero-byte-context'
        }
      }
    })).toMatchObject({ ok: true });
  });
});
