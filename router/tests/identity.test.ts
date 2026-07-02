import { describe, expect, it } from 'vitest';

import { loadManifest } from '../src/manifest/loadManifest.js';
import {
  baseWebSocketManifest,
  connectionMessageSchema,
  gatewayConnectResultSchema
} from './helpers/websocketFixtures.js';

const EXPECTED_WEBSOCKET_ENTRY_IDENTITY =
  'skiff-gateway-v1:sha256:fdd7562ac7e72970fd9a94b6edd54422b826e18093feb1b9de9c1ad54cfc8e7e';
const EXPECTED_WEBSOCKET_CONNECT_IDENTITY =
  'skiff-gateway-v1:sha256:b10fb09d16bf2cd906c9f25a5e145b3037bd58c285702564613a2d7cea3e58db';
const EXPECTED_WEBSOCKET_RECEIVE_IDENTITY =
  'skiff-gateway-v1:sha256:f24154cde6dd218102a8b62b9b666445984a2d49ad4e792692dd1e2f2d930362';

describe('gateway entry identity', () => {
  it('generates websocket entry, connect, and receive identities', () => {
    const manifest = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/hello',
        revisionId: '1111111111111111111111111111111111111111111111111111111111111111',
        protocolIdentity:
          'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000002'
      },
      operations: [
        {
          operation: 'HelloSocket.connect',
          operationAbiId: 'operation:test:service.skiff~run~~hello.HelloSocket.connect',
          target: 'service.skiff~run~~hello.HelloSocket.connect',
          mode: 'unary',
          parameters: [
            {
              name: 'session',
              schema: { type: 'any' }
            }
          ],
          response: gatewayConnectResultSchema()
        },
        {
          operation: 'HelloConnection.receive',
          operationAbiId: 'operation:test:service.skiff~run~~hello.HelloConnection.receive',
          target: 'service.skiff~run~~hello.HelloConnection.receive',
          mode: 'unary',
          parameters: [
            {
              name: 'context',
              schema: { type: 'any' }
            },
            {
              name: 'message',
              schema: connectionMessageSchema()
            },
            {
              name: 'connectionId',
              schema: { type: 'string' }
            }
          ],
          response: { type: 'null' }
        }
      ],
      gateway: {
        websocket: {
          id: 'client',
          path: '/ws',
          serviceParam: 'service',
          context: {
            type: 'object',
            required: ['userId'],
            properties: {
              userId: { type: 'string' }
            },
            additionalProperties: false
          },
          connect: {
            operation: 'HelloSocket.connect',
            operationAbiId: 'operation:test:service.skiff~run~~hello.HelloSocket.connect',
            adapterArgs: [
              { param: 'session', source: { kind: 'websocket.connectRequest' } }
            ]
          },
          receive: {
            operation: 'HelloConnection.receive',
            operationAbiId: 'operation:test:service.skiff~run~~hello.HelloConnection.receive',
            adapterArgs: [
              { param: 'context', source: { kind: 'websocket.connectionContext' } },
              { param: 'message', source: { kind: 'websocket.message' } },
              { param: 'connectionId', source: { kind: 'websocket.connectionId' } }
            ]
          }
        }
      }
    });

    const entry = manifest.websocketEntry;
    expect(entry).toBeDefined();
    expect(entry!.gatewayEntryIdentity).toBe(EXPECTED_WEBSOCKET_ENTRY_IDENTITY);
    expect(entry!.connect?.gatewayEntryIdentity).toBe(EXPECTED_WEBSOCKET_CONNECT_IDENTITY);
    expect(entry!.receive.gatewayEntryIdentity).toBe(EXPECTED_WEBSOCKET_RECEIVE_IDENTITY);
  });

  it('rejects supplied websocket gateway identities that do not match the computed identity', () => {
    expect(() =>
      loadManifest({
        ...baseWebSocketManifest(),
        gateway: {
          websocket: {
            ...baseWebSocketManifest().gateway.websocket,
            gatewayEntryIdentity:
              'skiff-gateway-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
          }
        }
      })
    ).toThrow(/gatewayEntryIdentity must match computed gateway identity/);
  });

  it('rejects supplied connect and receive gateway identities that do not match computed identities', () => {
    const wrongIdentity =
      'skiff-gateway-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

    expect(() => {
      const manifest = baseWebSocketManifest();
      return loadManifest({
        ...manifest,
        gateway: {
          websocket: {
            ...manifest.gateway.websocket,
            connect: {
              ...manifest.gateway.websocket.connect,
              gatewayEntryIdentity: wrongIdentity
            }
          }
        }
      });
    }).toThrow(/connect\.gatewayEntryIdentity must match computed gateway identity/);

    expect(() => {
      const manifest = baseWebSocketManifest();
      return loadManifest({
        ...manifest,
        gateway: {
          websocket: {
            ...manifest.gateway.websocket,
            receive: {
              ...manifest.gateway.websocket.receive,
              gatewayEntryIdentity: wrongIdentity
            }
          }
        }
      });
    }).toThrow(/receive\.gatewayEntryIdentity must match computed gateway identity/);
  });
});
