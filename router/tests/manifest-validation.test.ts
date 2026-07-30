import { afterEach, describe, expect, it } from 'vitest';

import { loadManifest as loadRuntimeManifest } from '../src/manifest/loadManifest.js';
import {
  httpRequestSchema,
  httpResponseSchema,
  httpResponseStreamEventSchema
} from './helpers/manifests.js';
import { closeTrackedResources } from './helpers/runtime.js';

afterEach(closeTrackedResources);

function loadManifest(value: unknown) {
  addDefaultOperationAbiIds(value);
  return loadRuntimeManifest(value);
}

function addDefaultOperationAbiIds(value: unknown): void {
  if (!isRecord(value) || !Array.isArray(value.operations)) {
    return;
  }
  value.operations.forEach((operation, index) => {
    if (!isRecord(operation) || typeof operation.operationAbiId === 'string') {
      return;
    }
    const target =
      typeof operation.target === 'string'
        ? operation.target
        : typeof operation.operation === 'string'
          ? operation.operation
          : `index:${index}`;
    operation.operationAbiId = testOperationAbiId(target);
  });
}

function isRecord(value: unknown): value is Record<string, any> {
  return typeof value === 'object' && value !== null;
}

function testOperationAbiId(target: string): string {
  return `operation:test:${target}`;
}

describe('router manifest validation', () => {
  it.each(['access', 'visibility', 'organizationRole'])(
    'rejects removed service metadata field %s',
    (field) => {
      const manifestValue = httpManifestValue() as any;
      manifestValue.service[field] =
        field === 'access' ? { visibility: 'internal' } : 'legacy';

      expect(() => loadManifest(manifestValue)).toThrow(
        new RegExp(`manifest\\.service does not support .*"${field}"`)
      );
    }
  );

  it('loads manifests without removed service metadata', () => {
    const loaded = loadManifest(httpManifestValue());

    expect(loaded.service).not.toHaveProperty('access');
  });

  it('accepts typed HTTP route metadata and exposes it on loaded routes', () => {
    const loaded = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/typed-http',
        revisionId: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        protocolIdentity: 'skiff-service-protocol-v5:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      },
      operations: [
        {
          operation: 'TypedHttpApi.handle',
          target: 'service.skiff~run~~typed-http.TypedHttpApi.handle',
          mode: 'unary',
          parameters: [{ name: 'request', schema: httpRequestSchema() }],
          response: httpResponseSchema()
        }
      ],
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/typed',
              operation: 'TypedHttpApi.handle',
              operationAbiId: testOperationAbiId(
                'service.skiff~run~~typed-http.TypedHttpApi.handle'
              ),
              typed: {
                body: {
                  schema: {
                    type: 'object',
                    required: ['name'],
                    properties: {
                      name: { type: 'string' }
                    },
                    additionalProperties: false
                  }
                },
                response: {
                  schema: {
                    type: 'object',
                    required: ['ok'],
                    properties: {
                      ok: { type: 'boolean' }
                    },
                    additionalProperties: false
                  }
                },
                ingressIdentity:
                  'skiff-http-ingress-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
              }
            }
          ]
        }
      }
    });

    expect(loaded.httpRouteEntries[0]?.typed).toEqual({
      body: {
        schema: {
          type: 'object',
          required: ['name'],
          properties: {
            name: { type: 'string' }
          },
          additionalProperties: false
        }
      },
      response: {
        schema: {
          type: 'object',
          required: ['ok'],
          properties: {
            ok: { type: 'boolean' }
          },
          additionalProperties: false
        }
      },
      ingressIdentity:
        'skiff-http-ingress-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
    });
    expect(loaded.httpRouteEntries[0]?.dispatchTarget).toBe(
      'service.skiff~run~~typed-http.TypedHttpApi.handle'
    );
    expect(loaded.httpRouteEntries[0]?.requestParameterName).toBe('request');
  });

  it('accepts typed HTTP adapter metadata for non-raw unary route operations', () => {
    const loaded = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/typed-http',
        revisionId: 'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
        protocolIdentity: 'skiff-service-protocol-v5:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
      },
      operations: [
        {
          operation: 'internal.todos.create',
          target: 'service.skiff~run~~typed-http.internal.todos.create',
          mode: 'unary',
          parameters: [
            { name: 'input', schema: { type: 'object' } },
            { name: 'context', schema: { type: 'object' } }
          ],
          response: { type: 'object' }
        }
      ],
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/typed',
              operation: 'internal.todos.create',
              operationAbiId: testOperationAbiId(
                'service.skiff~run~~typed-http.internal.todos.create'
              ),
              target: 'service.skiff~run~~typed-http.internal.todos.create',
              handler: {
                kind: 'serviceFunction',
                source: 'root.internal.todos.create',
                modulePath: 'internal.todos',
                symbol: 'create'
              },
              typed: {
                body: { schema: { type: 'object' } },
                response: { schema: { type: 'object' } },
                ingressIdentity:
                  'skiff-http-ingress-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                adapter: {
                  kind: 'typedJson',
                  handler: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.todos',
                    symbol: 'create'
                  },
                  pre: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.account',
                    symbol: 'pre'
                  },
                  adapterArgs: [
                    { param: 'input', source: { kind: 'http.body' } },
                    { param: 'context', source: { kind: 'http.context' } }
                  ]
                }
              }
            }
          ]
        }
      }
    });

    expect(loaded.httpRouteEntries[0]?.dispatchTarget).toBe(
      'service.skiff~run~~typed-http.internal.todos.create'
    );
    expect(loaded.httpRouteEntries[0]?.requestParameterName).toBe('input');
    expect(loaded.httpRouteEntries[0]?.typed?.adapter).toEqual({
      kind: 'typedJson',
      handler: {
        kind: 'serviceFunction',
        modulePath: 'internal.todos',
        symbol: 'create'
      },
      pre: {
        kind: 'serviceFunction',
        modulePath: 'internal.account',
        symbol: 'pre'
      },
      adapterArgs: [
        { param: 'input', source: { kind: 'http.body' } },
        { param: 'context', source: { kind: 'http.context' } }
      ]
    });
  });

  it('rejects old HTTP adapter handlerArgs manifest shape', () => {
    const manifestValue = {
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/typed-http',
        revisionId: 'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
        protocolIdentity: 'skiff-service-protocol-v5:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
      },
      operations: [
        {
          operation: 'internal.todos.create',
          target: 'service.skiff~run~~typed-http.internal.todos.create',
          mode: 'unary',
          parameters: [{ name: 'input', schema: { type: 'object' } }],
          response: { type: 'object' }
        }
      ],
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/typed',
              operation: 'internal.todos.create',
              operationAbiId: testOperationAbiId(
                'service.skiff~run~~typed-http.internal.todos.create'
              ),
              typed: {
                body: { schema: { type: 'object' } },
                response: { schema: { type: 'object' } },
                ingressIdentity:
                  'skiff-http-ingress-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                adapter: {
                  kind: 'typedJson',
                  handler: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.todos',
                    symbol: 'create'
                  },
                  handlerArgs: [{ kind: 'body' }]
                }
              }
            }
          ]
        }
      }
    };

    expect(() => loadManifest(manifestValue)).toThrow(/handlerArgs/);
  });

  it('rejects malformed typed HTTP route metadata', () => {
    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/typed-http',
          revisionId: 'abababababababababababababababababababababababababababababababab',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:abababababababababababababababababababababababababababababababab'
        },
        operations: [
          {
            operation: 'TypedHttpApi.handle',
            target: 'service.skiff~run~~typed-http.TypedHttpApi.handle',
            mode: 'unary',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: httpResponseSchema()
          }
        ],
        gateway: {
          http: {
            routes: [
              {
                method: 'POST',
                path: '/typed',
                operation: 'TypedHttpApi.handle',
                typed: {
                  body: { schema: { type: 'string' } },
                  response: { schema: { type: 'boolean' } },
                  ingressIdentity: 'skiff-http-ingress-v1:sha256:not-a-real-hash'
                }
              }
            ]
          }
        }
      })
    ).toThrow(/gateway\.http\.routes\[0\]\.typed\.ingressIdentity must be skiff-http-ingress-v1/);
  });

  it('does not infer raw HTTP validity from operation schemas', () => {
    const streamEvent = {
      ...httpResponseStreamEventSchema(),
      xSkiffSymbol: 'HttpResponseStreamEvent'
    };

    const loaded = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/raw-http-stream',
        revisionId: 'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
        protocolIdentity: 'skiff-service-protocol-v5:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
      },
      operations: [
        {
          operation: 'StreamApi.handle',
          target: 'service.skiff~run~~raw-http-stream.StreamApi.handle',
          mode: 'serverStream',
          parameters: [{ name: 'request', schema: httpRequestSchema() }],
          response: streamEvent
        }
      ],
      gateway: {
        http: {
          raw: {
            operation: 'StreamApi.handle',
            target: 'gateway.skiff~run~~raw-http-stream.http.raw'
          }
        }
      }
    });

    expect(loaded.rawHttpEntries[0]?.operation).toBe('StreamApi.handle');
  });

  it('accepts structural HTTP response stream event schemas without symbol fallback', () => {
    const streamEvent = httpResponseStreamEventSchema();
    delete (streamEvent as any).xSkiffSymbol;

    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/raw-http-stream',
          revisionId: 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'
        },
        operations: [
          {
            operation: 'StreamApi.handle',
            target: 'service.skiff~run~~raw-http-stream.StreamApi.handle',
            mode: 'serverStream',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: streamEvent
          }
        ],
        gateway: {
          http: {
            raw: {
              operation: 'StreamApi.handle',
              target: 'gateway.skiff~run~~raw-http-stream.http.raw'
            }
          }
        }
      })
    ).not.toThrow();
  });

  it('keeps raw HTTP routes without typed metadata unchanged', () => {
    const loaded = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/raw-http-route',
        revisionId: 'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
        protocolIdentity: 'skiff-service-protocol-v5:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
      },
      operations: [
        {
          operation: 'RawHttpApi.handle',
          target: 'service.skiff~run~~raw-http-route.RawHttpApi.handle',
          mode: 'unary',
          parameters: [{ name: 'request', schema: httpRequestSchema() }],
          response: httpResponseSchema()
        }
      ],
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/raw',
              operation: 'RawHttpApi.handle',
              operationAbiId: testOperationAbiId(
                'service.skiff~run~~raw-http-route.RawHttpApi.handle'
              ),
              adapter: {
                kind: 'rawHttp',
                handler: {
                  kind: 'serviceFunction',
                  modulePath: 'internal.raw_http',
                  symbol: 'handle'
                },
                adapterArgs: [{ param: 'request', source: { kind: 'http.request' } }]
              }
            }
          ]
        }
      }
    });

    expect(loaded.httpRouteEntries[0]?.typed).toBeUndefined();
    expect(loaded.httpRouteEntries[0]?.dispatchTarget).toBe(
      'service.skiff~run~~raw-http-route.RawHttpApi.handle'
    );
    expect(loaded.httpRouteEntries[0]?.operationManifest?.parameters[0]?.schema).toEqual(
      httpRequestSchema()
    );
    expect(loaded.httpRouteEntries[0]?.operationManifest?.response).toEqual(
      httpResponseSchema()
    );
  });

  it('requires explicit operationAbiId on service HTTP routes', () => {
    const operationTarget = 'service.skiff~run~~http-route-abi.HttpApi.handle';

    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/http-route-abi',
          revisionId: 'abababababababababababababababababababababababababababababababab',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:abababababababababababababababababababababababababababababababab'
        },
        operations: [
          {
            operation: 'HttpApi.handle',
            operationAbiId: testOperationAbiId(operationTarget),
            target: operationTarget,
            mode: 'unary',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: httpResponseSchema()
          }
        ],
        gateway: {
          http: {
            routes: [
              {
                method: 'POST',
                path: '/handle',
                operation: 'HttpApi.handle',
                adapter: {
                  kind: 'rawHttp',
                  handler: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.http',
                    symbol: 'handle'
                  },
                  adapterArgs: [{ param: 'request', source: { kind: 'http.request' } }]
                }
              }
            ]
          }
        }
      })
    ).toThrow(/gateway\.http\.routes\[0\]\.operationAbiId is required for service HTTP routes/);
  });

  it('rejects service HTTP routes whose operationAbiId differs from the operation', () => {
    const operationTarget = 'service.skiff~run~~http-route-abi.HttpApi.handle';

    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/http-route-abi',
          revisionId: 'cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd'
        },
        operations: [
          {
            operation: 'HttpApi.handle',
            operationAbiId: testOperationAbiId(operationTarget),
            target: operationTarget,
            mode: 'unary',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: httpResponseSchema()
          }
        ],
        gateway: {
          http: {
            routes: [
              {
                method: 'POST',
                path: '/handle',
                operation: 'HttpApi.handle',
                operationAbiId: 'operation:test:mismatched-http-route',
                adapter: {
                  kind: 'rawHttp',
                  handler: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.http',
                    symbol: 'handle'
                  },
                  adapterArgs: [{ param: 'request', source: { kind: 'http.request' } }]
                }
              }
            ]
          }
        }
      })
    ).toThrow(
      /gateway\.http\.routes\[0\]\.operationAbiId must match operation HttpApi\.handle\.operationAbiId/
    );
  });

  it('rejects old HTTP route bind/responseMode manifest shape', () => {
    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'example.com/legacy-http',
          revisionId: '1111111111111111111111111111111111111111111111111111111111111111',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:1111111111111111111111111111111111111111111111111111111111111111'
        },
        operations: [
          {
            operation: 'LegacyHttpApi.handle',
            target: 'service.example~com~~legacy-http.LegacyHttpApi.handle',
            mode: 'unary',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: httpResponseSchema()
          }
        ],
        gateway: {
          http: {
            routes: [
              {
                id: 'legacy',
                method: 'GET',
                path: '/legacy',
                operation: 'LegacyHttpApi.handle',
                bind: {},
                responseMode: 'json'
              }
            ]
          }
        }
      })
    ).toThrow(/gateway\.http\.routes\[0\] does not support/);
  });


  it('rejects non-canonical protocol identities in direct manifests', () => {
    const manifestValue = httpManifestValue();
    manifestValue.service.protocolIdentity = 'skiff-service-protocol-v5:sha256:not-a-real-hash';

    expect(() => loadManifest(manifestValue)).toThrow(
      /manifest\.service\.protocolIdentity must be skiff-service-protocol-v5/
    );
  });

  it('rejects non-canonical revision ids in direct manifests', () => {
    const manifestValue = httpManifestValue();
    manifestValue.service.revisionId = 'revision-websocket';

    expect(() => loadManifest(manifestValue)).toThrow(
      /manifest\.service\.revisionId must be <64 lowercase hex>/
    );
  });

  it('rejects direct manifests whose service id is not a publication id', () => {
    const manifestValue = httpManifestValue();
    manifestValue.service.id = 'websocket_fixture';

    expect(() => loadManifest(manifestValue)).toThrow(
      /manifest\.service\.id must be a publication id/
    );
  });

  it('rejects raw publication ids in service operation targets', () => {
    const manifestValue = httpManifestValue();
    manifestValue.operations[0]!.target =
      'service.example.com/sample.SampleHttpApi.handle';

    expect(() => loadManifest(manifestValue)).toThrow(
      /operation SampleHttpApi\.handle\.target must be service\.example~com~~sample\.<target suffix>/
    );
  });

  it('rejects raw publication ids in gateway targets', () => {
    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/sample',
          revisionId: '4444444444444444444444444444444444444444444444444444444444444444',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:4444444444444444444444444444444444444444444444444444444444444444'
        },
        operations: [
          {
            operation: 'SessionApi.handle',
            target: 'service.skiff~run~~sample.SessionApi.handle',
            mode: 'unary',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: httpResponseSchema()
          }
        ],
        gateway: {
          http: {
            routes: [
              {
                method: 'POST',
                path: '/session',
                operation: 'SessionApi.handle',
                target: 'gateway.skiff.run/sample.http.post.session',
                adapter: {
                  kind: 'rawHttp',
                  handler: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.session',
                    symbol: 'handle'
                  },
                  adapterArgs: [{ param: 'request', source: { kind: 'http.request' } }]
                }
              }
            ]
          }
        }
      })
    ).toThrow(
      /gateway\.http\.routes\[0\]\.target must be gateway\.skiff~run~~sample\.<target suffix>/
    );
  });

  it('accepts projected service and gateway targets in direct manifests', () => {
    expect(() =>
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/sample',
          revisionId: '5555555555555555555555555555555555555555555555555555555555555555',
          protocolIdentity: 'skiff-service-protocol-v5:sha256:5555555555555555555555555555555555555555555555555555555555555555'
        },
        operations: [
          {
            operation: 'SampleHttpApi.handle',
            target: 'service.skiff~run~~sample.SampleHttpApi.handle',
            mode: 'unary',
            parameters: [{ name: 'request', schema: httpRequestSchema() }],
            response: httpResponseSchema()
          }
        ],
        gateway: {
          http: {
            raw: {
              operation: 'SampleHttpApi.handle',
              target: 'gateway.skiff~run~~sample.http.raw'
            }
          }
        }
      })
    ).not.toThrow();
  });

  it('rejects raw HTTP metadata that points at an unknown operation', () => {
    expect(
      () =>
        loadManifest({
          schemaVersion: 'skiff-runtime-manifest-v1',
          service: {
            id: 'example.com/sample',
            revisionId: '3333333333333333333333333333333333333333333333333333333333333333',
            protocolIdentity: 'skiff-service-protocol-v5:sha256:2222222222222222222222222222222222222222222222222222222222222222'
          },
          operations: [
            {
              operation: 'AdminHttpApi.handle',
              target: 'service.example~com~~sample.AdminHttpApi.handle',
              mode: 'unary',
              parameters: [{ name: 'request', schema: httpRequestSchema() }],
              response: httpResponseSchema()
            }
          ],
          gateway: {
            http: {
              raw: {
                operation: 'MissingHttpApi.handle',
                target: 'gateway.example~com~~sample.http.raw'
              }
            }
          }
        })
    ).toThrow(/gateway\.http\.raw references unknown operation MissingHttpApi\.handle/);
  });

});

function httpManifestValue() {
  return {
    schemaVersion: 'skiff-runtime-manifest-v1',
    service: {
      id: 'example.com/sample',
      revisionId: '3333333333333333333333333333333333333333333333333333333333333333',
      protocolIdentity:
        'skiff-service-protocol-v5:sha256:2222222222222222222222222222222222222222222222222222222222222222'
    },
    operations: [
      {
        operation: 'SampleHttpApi.handle',
        operationAbiId:
          'operation:test:service.example~com~~sample.SampleHttpApi.handle',
        target: 'service.example~com~~sample.SampleHttpApi.handle',
        mode: 'unary',
        parameters: [{ name: 'request', schema: httpRequestSchema() }],
        response: httpResponseSchema()
      }
    ],
    gateway: {
      http: {
        raw: {
          operation: 'SampleHttpApi.handle',
          target: 'gateway.example~com~~sample.http.raw'
        }
      }
    }
  };
}
