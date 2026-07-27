import { loadManifest } from '../../src/manifest/loadManifest.js';
import { publicationStorageSegment } from '../../src/publicationId.js';

export const SAMPLE_SERVICE_ID = 'skiff.run/sample';
export const DEFAULT_TEST_BUILD_ID =
  'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333';

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
    'skiff-service-protocol-v5:sha256:5555555555555555555555555555555555555555555555555555555555555555';
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
    sessionOptions?: boolean;
  } = {}
) {
  const serviceId = input.serviceId ?? SAMPLE_SERVICE_ID;
  const serviceTargetComponent = publicationStorageSegment(serviceId);
  const protocolIdentity =
    input.protocolIdentity ??
    'skiff-service-protocol-v5:sha256:5555555555555555555555555555555555555555555555555555555555555555';
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
          ...(input.sessionOptions === true
            ? [
                {
                  method: 'OPTIONS',
                  path: '/session',
                  handler: {
                    kind: 'serviceFunction' as const,
                    source: 'root.api.sessionPreflight',
                    modulePath: 'api',
                    symbol: 'sessionPreflight'
                  },
                  operation: 'SessionApi.handle',
                  operationAbiId: testOperationAbiId(sessionTarget),
                  target: sessionTarget,
                  adapter: {
                    kind: 'rawHttp' as const,
                    handler: {
                      kind: 'serviceFunction' as const,
                      modulePath: 'api',
                      symbol: 'sessionPreflight'
                    },
                    adapterArgs: [
                      { param: 'request', source: { kind: 'http.request' as const } }
                    ]
                  }
                }
              ]
            : []),
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
  return manifest;
}
