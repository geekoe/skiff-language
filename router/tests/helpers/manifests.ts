import { loadManifest } from '../../src/manifest/loadManifest.js';
import { publicationStorageSegment } from '../../src/publicationId.js';

import { webSocketManifestValue } from './websocketFixtures.js';

export const SAMPLE_SERVICE_ID = 'skiff.run/sample';
export const DEFAULT_TEST_BUILD_ID =
  'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333';

export function loadWebSocketManifest() {
  return withBuildId(loadManifest(webSocketManifestValue()));
}

export function webSocketRuntimeGatewayEntryIdentities(
  manifest: ReturnType<typeof loadManifest>
): string[] {
  const entry = manifest.websocketEntry;
  if (!entry) {
    return [];
  }
  return [
    entry.connect?.gatewayEntryIdentity,
    entry.receive.gatewayEntryIdentity
  ].filter((identity): identity is string => identity !== undefined);
}

export function loadRawHttpManifest(
  input: {
    serviceId?: string;
    protocolIdentity?: string;
    stream?: boolean;
    buildId?: string;
  } = {}
) {
  const serviceId = input.serviceId ?? SAMPLE_SERVICE_ID;
  const serviceTargetComponent = publicationStorageSegment(serviceId);
  const protocolIdentity =
    input.protocolIdentity ??
    'skiff-protocol-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555';
  const stream = input.stream ?? false;
  const handleTarget = `service.${serviceTargetComponent}.SampleHttpApi.handle`;
  return withBuildId(loadManifest({
    schemaVersion: 'skiff-runtime-manifest-v1',
    service: {
      id: serviceId,
      revisionId: testRevisionId(`${serviceId}:raw`),
      protocolIdentity
    },
    operations: [
      {
        operation: 'SampleHttpApi.handle',
        operationAbiId: testOperationAbiId(handleTarget),
        target: handleTarget,
        mode: stream ? 'serverStream' : 'unary',
        parameters: [
          {
            name: 'request',
            schema: httpRequestSchema()
          }
        ],
        response: stream ? httpResponseStreamEventSchema() : httpResponseSchema()
      }
    ],
    timeout: {
      defaultMs: 2000
    },
    gateway: {
      http: {
        raw: {
          operation: 'SampleHttpApi.handle',
          target: `gateway.${serviceTargetComponent}.http.raw`
        }
      }
    }
  }), input.buildId);
}

export function loadRawHttpStreamManifest(
  input: {
    serviceId?: string;
    protocolIdentity?: string;
  } = {}
) {
  return loadRawHttpManifest({ ...input, stream: true });
}

export function loadHttpRouteManifest(
  input: {
    serviceId?: string;
    protocolIdentity?: string;
  } = {}
) {
  const serviceId = input.serviceId ?? SAMPLE_SERVICE_ID;
  const serviceTargetComponent = publicationStorageSegment(serviceId);
  const protocolIdentity =
    input.protocolIdentity ??
    'skiff-protocol-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555';
  const sessionTarget = `service.${serviceTargetComponent}.SessionApi.handle`;
  const trackTarget = `service.${serviceTargetComponent}.TrackApi.handle`;
  const rawTarget = `service.${serviceTargetComponent}.SampleHttpApi.handle`;
  return withBuildId(loadManifest({
    schemaVersion: 'skiff-runtime-manifest-v1',
    service: {
      id: serviceId,
      revisionId: testRevisionId(`${serviceId}:route`),
      protocolIdentity
    },
    operations: [
      {
        operation: 'SessionApi.handle',
        operationAbiId: testOperationAbiId(sessionTarget),
        target: sessionTarget,
        mode: 'unary',
        parameters: [
          {
            name: 'request',
            schema: httpRequestSchema()
          }
        ],
        response: httpResponseSchema()
      },
      {
        operation: 'TrackApi.handle',
        operationAbiId: testOperationAbiId(trackTarget),
        target: trackTarget,
        mode: 'unary',
        parameters: [
          {
            name: 'request',
            schema: httpRequestSchema()
          }
        ],
        response: httpResponseSchema()
      },
      {
        operation: 'SampleHttpApi.handle',
        operationAbiId: testOperationAbiId(rawTarget),
        target: rawTarget,
        mode: 'unary',
        parameters: [
          {
            name: 'request',
            schema: httpRequestSchema()
          }
        ],
        response: httpResponseSchema()
      }
    ],
    timeout: {
      defaultMs: 2000
    },
    gateway: {
      http: {
        routes: [
          {
            method: 'POST',
            path: '/session',
            handler: {
              kind: 'serviceFunction',
              source: 'root.api.session',
              modulePath: 'api',
              symbol: 'session'
            },
            operation: 'SessionApi.handle',
            operationAbiId: testOperationAbiId(sessionTarget),
            target: `service.${serviceTargetComponent}.SessionApi.handle`,
            adapter: {
              kind: 'rawHttp',
              handler: {
                kind: 'serviceFunction',
                modulePath: 'api',
                symbol: 'session'
              },
              adapterArgs: [{ param: 'request', source: { kind: 'http.request' } }]
            }
          },
          {
            method: 'POST',
            path: '/track',
            handler: {
              kind: 'serviceFunction',
              source: 'root.api.track',
              modulePath: 'api',
              symbol: 'track'
            },
            operation: 'TrackApi.handle',
            operationAbiId: testOperationAbiId(trackTarget),
            target: `service.${serviceTargetComponent}.TrackApi.handle`,
            adapter: {
              kind: 'rawHttp',
              handler: {
                kind: 'serviceFunction',
                modulePath: 'api',
                symbol: 'track'
              },
              adapterArgs: [{ param: 'request', source: { kind: 'http.request' } }]
            }
          }
        ],
        raw: {
          operation: 'SampleHttpApi.handle',
          target: `gateway.${serviceTargetComponent}.http.raw`
        }
      }
    }
  }));
}

export function httpHeaderSchema() {
  return {
    type: 'object',
    required: ['name', 'value'],
    properties: {
      name: { type: 'string' },
      value: { type: 'string' }
    },
    additionalProperties: false
  };
}

export function httpBodySchema() {
  return {
    type: 'string',
    contentEncoding: 'base64',
    xSkiffSymbol: 'std.bytes.bytes'
  };
}

export function httpRequestSchema() {
  return {
    type: 'object',
    required: ['method', 'url', 'path', 'query', 'headers', 'body'],
    properties: {
      method: { type: 'string' },
      url: { type: 'string' },
      path: { type: 'string' },
      query: { type: 'array', items: httpHeaderSchema() },
      headers: { type: 'array', items: httpHeaderSchema() },
      body: httpBodySchema()
    },
    additionalProperties: false
  };
}

export function httpResponseSchema() {
  return {
    type: 'object',
    required: ['status', 'headers', 'body'],
    properties: {
      status: { type: 'integer' },
      headers: { type: 'array', items: httpHeaderSchema() },
      body: httpBodySchema()
    },
    additionalProperties: false
  };
}

export function httpResponseStreamEventSchema() {
  return {
    type: 'object',
    xSkiffSymbol: 'std.http.HttpResponseStreamEvent',
    oneOf: [
      {
        type: 'object',
        required: ['tag', 'status', 'headers'],
        properties: {
          tag: { type: 'string', enum: ['start'] },
          status: { type: 'integer' },
          headers: { type: 'array', items: httpHeaderSchema() }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: ['tag', 'value'],
        properties: {
          tag: { type: 'string', enum: ['chunk'] },
          value: httpBodySchema()
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: ['tag'],
        properties: {
          tag: { type: 'string', enum: ['end'] }
        },
        additionalProperties: false
      }
    ]
  };
}

export function loadWebSocketManifestForService(
  serviceId: string,
  protocolIdentity: string
) {
  return loadManifest(webSocketManifestValueForService(serviceId, protocolIdentity));
}

function webSocketManifestValueForService(serviceId: string, protocolIdentity: string) {
  const value = JSON.parse(JSON.stringify(webSocketManifestValue()));
  const typeName = serviceTypeName(serviceId);
  const connectOperation = `${typeName}Connection.connect`;
  const receiveOperation = `${typeName}Connection.receive`;
  value.service.id = serviceId;
  value.service.revisionId = testRevisionId(`${serviceId}:websocket`);
  value.service.protocolIdentity = protocolIdentity;
  value.operations[0].operation = connectOperation;
  value.operations[0].target = `service.${publicationStorageSegment(serviceId)}.${typeName}Connection.connect`;
  value.operations[0].operationAbiId = testOperationAbiId(value.operations[0].target);
  value.operations[1].operation = receiveOperation;
  value.operations[1].target = `service.${publicationStorageSegment(serviceId)}.${typeName}Connection.receive`;
  value.operations[1].operationAbiId = testOperationAbiId(value.operations[1].target);
  value.gateway.websocket.connect.operation = connectOperation;
  value.gateway.websocket.connect.operationAbiId = value.operations[0].operationAbiId;
  value.gateway.websocket.receive.operation = receiveOperation;
  value.gateway.websocket.receive.operationAbiId = value.operations[1].operationAbiId;
  return value;
}

function serviceTypeName(serviceId: string): string {
  const localName = serviceLocalName(serviceId);
  return localName
    .split(/[_-]/)
    .filter((part) => part.length > 0)
    .map((part) => part[0]!.toUpperCase() + part.slice(1))
    .join('');
}

function serviceLocalName(serviceId: string): string {
  return serviceId.split('/').at(-1) ?? serviceId;
}

function testRevisionId(seed: string): string {
  let hash = 0;
  for (let index = 0; index < seed.length; index += 1) {
    hash = (hash * 31 + seed.charCodeAt(index)) >>> 0;
  }
  return hash.toString(16).padStart(8, '0').repeat(8).slice(0, 64);
}

function testOperationAbiId(target: string): string {
  return `operation:test:${target}`;
}

export function withBuildId<TManifest extends ReturnType<typeof loadManifest>>(
  manifest: TManifest,
  buildId = DEFAULT_TEST_BUILD_ID
): TManifest {
  for (const entry of manifest.httpRouteEntries) {
    entry.buildId ??= buildId;
  }
  for (const entry of manifest.rawHttpEntries) {
    entry.buildId ??= buildId;
  }
  for (const entry of manifest.websocketEntries) {
    entry.buildId ??= buildId;
  }
  if (manifest.websocketEntry) {
    manifest.websocketEntry.buildId ??= buildId;
  }
  return manifest;
}
