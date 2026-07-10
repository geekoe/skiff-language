import { describe, expect, it } from 'vitest';

import {
  decodeRuntimeFrame,
  encodeBinaryFrame,
  encodeRuntimeFrame,
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS
} from '../src/protocol/envelope.js';
import { decodeRuntimePayload, encodeRuntimePayload } from './helpers/runtimePayloadCodec.js';
import {
  runtimeFrameHeaderFixtures,
  runtimeFrameHeaderSchemas,
  validateRouterToRuntimeFrameHeader,
  validateRuntimeToRouterFrameHeader,
  type RouterToRuntimeFrameHeaderName,
  type RuntimeProtocolFrameHeaderName,
  type RuntimeToRouterFrameHeaderName
} from '../src/protocol/runtimeProtocol.js';
import {
  CONTRACT_H_REQUEST_CANCEL_SITUATIONS,
  mapInternalRequestCancelReason,
  requestCancelReasonForSituation,
  type RequestCancelReason,
  type RequestCancelSituation
} from '../src/protocol/cancelReason.js';
import type { JsonSchema } from '../src/manifest/types.js';

const runtimeFrameHeaderTypes = [
  'runtime.register',
  'runtime.capabilities',
  'runtime.health',
  'actor.put.request',
  'actor.put.response',
  'actor.put.error',
  'actor.find.request',
  'actor.find.response',
  'actor.find.error',
  'actor.remove.request',
  'actor.remove.response',
  'actor.remove.error',
  'spawn.submit.request',
  'spawn.submit.response',
  'spawn.submit.error',
  'spawn.claim.request',
  'spawn.claim.response',
  'spawn.claim.error',
  'spawn.renew.request',
  'spawn.renew.response',
  'spawn.renew.error',
  'spawn.complete.request',
  'spawn.complete.response',
  'spawn.complete.error',
  'spawn.fail.request',
  'spawn.fail.response',
  'spawn.fail.error',
  'request.start',
  'package-test.start',
  'router.control',
  'runtime.registered',
  'response.start',
  'response.end',
  'response.error',
  'response.chunk',
  'request.cancel',
  'connection.send'
] as const satisfies readonly RuntimeProtocolFrameHeaderName[];

const runtimeToRouterFrameHeaderTypes = [
  'runtime.register',
  'runtime.capabilities',
  'runtime.health',
  'actor.put.request',
  'actor.find.request',
  'actor.remove.request',
  'spawn.submit.request',
  'spawn.claim.request',
  'spawn.renew.request',
  'spawn.complete.request',
  'spawn.fail.request',
  'request.start',
  'response.start',
  'response.end',
  'response.error',
  'response.chunk',
  'request.cancel',
  'connection.send'
] as const satisfies readonly RuntimeToRouterFrameHeaderName[];

const routerToRuntimeFrameHeaderTypes = [
  'runtime.registered',
  'router.control',
  'actor.put.response',
  'actor.put.error',
  'actor.find.response',
  'actor.find.error',
  'actor.remove.response',
  'actor.remove.error',
  'spawn.submit.response',
  'spawn.submit.error',
  'spawn.claim.response',
  'spawn.claim.error',
  'spawn.renew.response',
  'spawn.renew.error',
  'spawn.complete.response',
  'spawn.complete.error',
  'spawn.fail.response',
  'spawn.fail.error',
  'request.start',
  'package-test.start',
  'request.cancel',
  'response.start',
  'response.end',
  'response.error',
  'response.chunk'
] as const satisfies readonly RouterToRuntimeFrameHeaderName[];

describe('runtime protocol fixtures and schemas', () => {
  it('maps Contract H cancel situations to stable request.cancel reasons', () => {
    const expected = {
      caller_abort: 'caller_cancel',
      client_disconnect: 'client_disconnect',
      timeout: 'timeout',
      deadline_exceeded: 'deadline_exceeded',
      backpressure: 'backpressure',
      protocol_error: 'protocol_error',
      stream_dropped: 'stream_dropped',
      runtime_disconnect: 'runtime_disconnect',
      router_shutdown: 'router_shutdown'
    } as const satisfies Record<RequestCancelSituation, RequestCancelReason>;

    expect(CONTRACT_H_REQUEST_CANCEL_SITUATIONS).toEqual(Object.keys(expected));

    for (const [situation, reason] of Object.entries(expected) as Array<
      [RequestCancelSituation, RequestCancelReason]
    >) {
      expect(requestCancelReasonForSituation(situation)).toBe(reason);

      const header = {
        ...runtimeFrameHeaderFixtures['request.cancel'],
        reason
      };
      expect(validateRouterToRuntimeFrameHeader(header)).toEqual({
        ok: true,
        envelope: header
      });
      expect(validateRuntimeToRouterFrameHeader(header)).toEqual({
        ok: true,
        envelope: header
      });
    }
  });

  it('maps internal cancel reasons while retaining the original reason', () => {
    expect(mapInternalRequestCancelReason('chunk_seq_mismatch')).toEqual({
      internalReason: 'chunk_seq_mismatch',
      wireReason: 'protocol_error'
    });
    expect(mapInternalRequestCancelReason('stream_cancelled')).toEqual({
      internalReason: 'stream_cancelled',
      wireReason: 'stream_dropped'
    });
    expect(mapInternalRequestCancelReason('unknown_internal_reason')).toEqual({
      internalReason: 'unknown_internal_reason',
      wireReason: 'caller_cancel'
    });
  });

  it('returns clear validation errors for malformed runtime frame headers', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        schemaVersion: 'skiff-runtime-frame-v1',
        type: 'response.end'
      })
    ).toEqual({
      ok: false,
      error: 'invalid response.end envelope: requestId must be a string'
    });
    expect(
      validateRuntimeToRouterFrameHeader({
        schemaVersion: 'skiff-runtime-frame-v1',
        type: 'response.error',
        requestId: 'request-1',
        error: {
          code: 'Broken'
        }
      })
    ).toEqual({
      ok: false,
      error: 'invalid response.error envelope: error.message must be a string'
    });
    expect(validateRuntimeToRouterFrameHeader({ type: 'not.real' })).toEqual({
      ok: false,
      error:
        'invalid runtime frame header envelope: type must be one of runtime.register, runtime.capabilities, runtime.health, actor.put.request, actor.find.request, actor.remove.request, spawn.submit.request, spawn.claim.request, spawn.renew.request, spawn.complete.request, spawn.fail.request, request.start, request.cancel, connection.send, response.start, response.chunk, response.end, response.error'
    });
  });

  it('accepts and rejects service-independent runtime capability frames', () => {
    expect(
      validateRuntimeToRouterFrameHeader(runtimeFrameHeaderFixtures['runtime.capabilities'])
    ).toEqual({
      ok: true,
      envelope: runtimeFrameHeaderFixtures['runtime.capabilities']
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.capabilities'],
        serviceId: 'example.com/hello'
      })
    ).toEqual({
      ok: false,
      error: 'invalid runtime.capabilities frame header envelope: serviceId is not supported'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.capabilities'],
        capabilities: []
      })
    ).toEqual({
      ok: false,
      error: 'invalid runtime.capabilities envelope: capabilities must be an object'
    });
  });

  it('accepts and rejects runtime health frames', () => {
    expect(validateRuntimeToRouterFrameHeader(runtimeFrameHeaderFixtures['runtime.health'])).toEqual({
      ok: true,
      envelope: runtimeFrameHeaderFixtures['runtime.health']
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.health'],
        counters: {
          ...runtimeFrameHeaderFixtures['runtime.health'].counters,
          spawnedTasksActive: -1
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.health envelope: counters.spawnedTasksActive must be a non-negative integer'
    });
  });

  it('rejects non-canonical runtime registration identities', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        serviceProtocolIdentity: 'skiff-protocol-v1:sha256:not-a-real-hash'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: serviceProtocolIdentity must be skiff-protocol-v1:sha256:<64 lowercase hex>'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        gatewayEntryIdentities: ['gateway-entry']
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: gatewayEntryIdentities items must be skiff-gateway-v1:sha256:<64 lowercase hex>'
    });
  });

  it('rejects runtime registrations with raw service or gateway target components', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        targets: ['service.example.com/hello.HelloApi.hello']
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: targets items must use service.example~com~~hello.<target suffix>'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        targets: ['gateway.example.com/hello.http.raw']
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: targets items must use gateway.example~com~~hello.<target suffix>'
    });
  });

  it('rejects non-canonical router request identities', () => {
    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['request.start'],
        serviceProtocolIdentity: 'skiff-protocol-v1:sha256:not-a-real-hash'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start envelope: serviceProtocolIdentity must be skiff-protocol-v1:sha256:<64 lowercase hex>'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['request.start'],
        gatewayEntryIdentity: 'gateway-entry'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start envelope: gatewayEntryIdentity must be skiff-gateway-v1:sha256:<64 lowercase hex>'
    });
  });

  it('accepts optional serviceId on router request.start frames', () => {
    const requestEnvelope = {
      ...runtimeFrameHeaderFixtures['request.start'],
      serviceId: 'example.com/hello'
    };

    expect(runtimeFrameHeaderSchemas['request.start'].properties.serviceId).toEqual({
      type: 'string'
    });
    expect(validateRouterToRuntimeFrameHeader(requestEnvelope)).toEqual({
      ok: true,
      envelope: requestEnvelope
    });
    expect(
      validateRouterToRuntimeFrameHeader({
        ...requestEnvelope,
        serviceId: 'not a publication id'
      })
    ).toEqual({
      ok: false,
      error: 'invalid request.start envelope: serviceId must be a publication id'
    });
  });

  it('accepts runtime-originated service request.start frames', () => {
    const requestEnvelope = {
      ...runtimeFrameHeaderFixtures['request.start'],
      requestId: 'service-call-1',
      caller: {
        kind: 'service',
        target: 'service.example~com~~hello.HelloApi.handle'
      },
      serviceId: 'example.com/hello',
      buildId:
        'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333'
    };

    expect(validateRuntimeToRouterFrameHeader(requestEnvelope)).toEqual({
      ok: true,
      envelope: requestEnvelope
    });
  });

  it('accepts explicit activation metadata in control and dispatch frames', () => {
    const controlEnvelope = {
      ...runtimeFrameHeaderFixtures['router.control'],
      serviceConfig: [
        {
          serviceId: 'example.com/hello',
          buildId:
            'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
          activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
          resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
          resolvedConfig: {
            dashscopeApiKey: 'secret-local',
            dashscopeModel: 'qwen-plus'
          },
          redactedResolvedConfig: {
            dashscopeApiKey: '[REDACTED]',
            dashscopeModel: 'qwen-plus'
          },
          redactionProjectionIdentity:
            'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
          configShape: {
            schemaVersion: 'skiff-config-shape-v1',
            entries: [
              {
                path: 'dashscopeApiKey',
                type: 'string',
                required: true
              },
              {
                path: 'dashscopeModel',
                type: 'string',
                required: false
              }
            ]
          },
          serviceDb: {
            mongoUrl: 'mongodb://127.0.0.1:27017/?directConnection=true',
            storageServiceId: 'example.com/hello'
          },
          packageConfigs: [
            {
              packageId: 'skiff.run/llm',
              alias: 'llm',
              resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:package-config-fixture',
              resolvedConfig: {
                dashscope: {
                  apiKey: 'package-secret'
                }
              },
              redactedResolvedConfig: {
                dashscope: {
                  apiKey: '[REDACTED]'
                }
              },
              redactionProjectionIdentity:
                'skiff-config-redaction-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555',
              configShape: {
                schemaVersion: 'skiff-config-shape-v1',
                entries: [
                  {
                    path: 'dashscope.apiKey',
                    type: 'string',
                    required: true
                  }
                ]
              }
            }
          ]
        }
      ]
    };
    const requestEnvelope = {
      ...runtimeFrameHeaderFixtures['request.start'],
      activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture'
    };

    expect(validateRouterToRuntimeFrameHeader(controlEnvelope)).toEqual({
      ok: true,
      envelope: controlEnvelope
    });
    expect(validateRouterToRuntimeFrameHeader(requestEnvelope)).toEqual({
      ok: true,
      envelope: requestEnvelope
    });
  });

  it('rejects malformed activation metadata in control and dispatch frames', () => {
    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId: 'build-plain',
            activationIdentity: 'activation-plain',
            resolvedConfigIdentity: 'config-plain',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity: 'redaction-plain',
            configShape: []
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity:
              'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            configShape: {
              schemaVersion: 'skiff-config-shape-v1',
              entries: []
            },
            serviceDb: {
              mongoUrl: '',
              storageServiceId: 'example.com/hello'
            }
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].serviceDb.mongoUrl must be a non-empty string'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity:
              'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            configShape: {
              schemaVersion: 'skiff-config-shape-v1',
              entries: []
            },
            serviceDb: {
              mongoUrl: 'mongodb://127.0.0.1:27017'
            }
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].serviceDb.storageServiceId must be a publication id'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity:
              'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            configShape: {
              schemaVersion: 'skiff-config-shape-v1',
              entries: [{ path: 'app.secret', type: 'Date', required: true }]
            }
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].configShape.entries[0].type must be string, number, bool, Json, or JsonObject'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity:
              'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            configShape: {
              schemaVersion: 'skiff-config-shape-v1',
              entries: []
            },
            serviceDb: {
              mongoUrl: 'mongodb://127.0.0.1:27017',
              storageServiceId: 'example.com/hello',
              storageNamespace: 'hello'
            }
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].serviceDb.storageNamespace is no longer supported'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity:
              'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            configShape: {
              schemaVersion: 'skiff-config-shape-v1',
              entries: []
            },
            serviceDb: {
              mongoUrl: 'mongodb://127.0.0.1:27017',
              storageServiceId: 'example.com/hello',
              storageNamespace: 'aaaaaaaaaaaaaaaaaaaa'
            }
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].serviceDb.storageNamespace is no longer supported'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['request.start'],
        activationIdentity: 'activation-plain'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start envelope: activationIdentity must be skiff-runtime-activation-v1:opaque:<opaque id>'
    });

    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceConfig: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:config-fixture',
            resolvedConfig: {},
            redactedResolvedConfig: {},
            redactionProjectionIdentity:
              'skiff-config-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            configShape: {
              schemaVersion: 'skiff-config-shape-v1',
              entries: []
            },
            packageConfigs: [
              {
                packageId: 'skiff.run/llm',
                dependencyRef: 'llm',
                resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:package-config-fixture',
                resolvedConfig: {},
                redactedResolvedConfig: {},
                redactionProjectionIdentity:
                  'skiff-config-redaction-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555',
                configShape: {
                  schemaVersion: 'skiff-config-shape-v1',
                  entries: []
                }
              }
            ]
          }
        ]
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control envelope: serviceConfig[0].packageConfigs[0].dependencyRef is no longer supported; use alias'
    });
  });

  it('rejects legacy serviceValues in control payloads', () => {
    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['router.control'],
        serviceValues: [
          {
            serviceId: 'example.com/hello',
            buildId:
              'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
            activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-fixture',
            valuesSnapshotIdentity: 'skiff-values-snapshot-v1:opaque:snapshot-fixture',
            valuesSnapshot: {},
            redactedValuesSnapshot: {},
            redactionProjectionIdentity:
              'skiff-values-redaction-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444',
            valuesPolicy: []
          }
        ]
      })
    ).toEqual({
      ok: false,
      error: 'invalid router.control envelope: serviceValues is no longer supported; use serviceConfig'
    });
  });

  it('accepts router control telemetry config', () => {
    const controlEnvelope = {
      ...runtimeFrameHeaderFixtures['router.control'],
      telemetry: {
        endpoint: 'ws://127.0.0.1:4002/telemetry',
        protocol: TELEMETRY_PROTOCOL,
        topics: [...TELEMETRY_TOPICS],
        queueMaxEvents: 10000,
        batchMaxEvents: 200,
        batchMaxBytes: 262144,
        flushIntervalMs: 1000,
        enabled: true
      }
    };

    expect(validateRouterToRuntimeFrameHeader(controlEnvelope)).toEqual({
      ok: true,
      envelope: controlEnvelope
    });
  });

  it('accepts router control file backend config', () => {
    const controlEnvelope = {
      ...runtimeFrameHeaderFixtures['router.control'],
      fileBackend: {
        local: {
          root: '/var/lib/skiff/file-blobs'
        },
        oss: {
          endpoint: 'https://oss-cn-hangzhou.aliyuncs.com',
          bucket: 'skiff-files',
          region: 'cn-hangzhou',
          accessKeyIdEnv: 'SKIFF_OSS_ACCESS_KEY_ID',
          accessKeySecretEnv: 'SKIFF_OSS_ACCESS_KEY_SECRET'
        }
      }
    };

    expect(validateRouterToRuntimeFrameHeader(controlEnvelope)).toEqual({
      ok: true,
      envelope: controlEnvelope
    });
  });

  it('validates router control artifact root overlays', () => {
    const controlEnvelope = {
      ...runtimeFrameHeaderFixtures['router.control'],
      artifactRoots: ['/var/lib/skiff/artifacts', '/tmp/skiff-test-artifacts']
    };

    expect(validateRouterToRuntimeFrameHeader(controlEnvelope)).toEqual({
      ok: true,
      envelope: controlEnvelope
    });
    expect(
      validateRouterToRuntimeFrameHeader({
        ...controlEnvelope,
        artifactRoot: '/var/lib/skiff/artifacts'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid router.control frame header: artifactRoot is not supported; use artifactRoots'
    });
    expect(
      validateRouterToRuntimeFrameHeader({
        ...controlEnvelope,
        artifactRoots: []
      })
    ).toEqual({
      ok: false,
      error: 'invalid router.control envelope: artifactRoots must be a non-empty string array'
    });
  });

  it('rejects malformed router control telemetry config', () => {
    const validTelemetry = {
      endpoint: 'ws://127.0.0.1:4002/telemetry',
      protocol: TELEMETRY_PROTOCOL,
      topics: ['log'] as const,
      queueMaxEvents: 10000,
      batchMaxEvents: 200,
      batchMaxBytes: 262144,
      flushIntervalMs: 1000,
      enabled: true
    };
    const cases = [
      {
        telemetry: { ...validTelemetry, protocol: 'skiff-telemetry-v2' },
        error:
          'invalid router.control envelope: telemetry.protocol must be one of skiff-telemetry-v1'
      },
      {
        telemetry: { ...validTelemetry, topics: ['log', 'audit'] },
        error:
          'invalid router.control envelope: telemetry.topics items must be one of log, trace, metric, health, debug'
      },
      {
        telemetry: { ...validTelemetry, topics: ['log', 'log'] },
        error: 'invalid router.control envelope: telemetry.topics must not contain duplicates'
      },
      {
        telemetry: { ...validTelemetry, topics: [] },
        error: 'invalid router.control envelope: telemetry.topics must be a non-empty array'
      },
      {
        telemetry: { ...validTelemetry, queueMaxEvents: 0 },
        error:
          'invalid router.control envelope: telemetry.queueMaxEvents must be a positive integer'
      },
      {
        telemetry: { ...validTelemetry, batchMaxEvents: -1 },
        error:
          'invalid router.control envelope: telemetry.batchMaxEvents must be a positive integer'
      },
      {
        telemetry: { ...validTelemetry, batchMaxBytes: 1.5 },
        error:
          'invalid router.control envelope: telemetry.batchMaxBytes must be a positive integer'
      },
      {
        telemetry: { ...validTelemetry, flushIntervalMs: 0 },
        error:
          'invalid router.control envelope: telemetry.flushIntervalMs must be a positive integer'
      },
      {
        telemetry: { ...validTelemetry, endpoint: undefined },
        error: 'invalid router.control envelope: telemetry.endpoint must be a string'
      }
    ];

    for (const { telemetry, error } of cases) {
      expect(
        validateRouterToRuntimeFrameHeader({
          ...runtimeFrameHeaderFixtures['router.control'],
          telemetry
        })
      ).toEqual({
        ok: false,
        error
      });
    }
  });

  it('rejects malformed router control file backend config', () => {
    const cases = [
      {
        fileBackend: {},
        error: 'invalid router.control envelope: fileBackend must configure local or oss'
      },
      {
        fileBackend: { local: { root: '' } },
        error:
          'invalid router.control envelope: fileBackend.local.root must be a non-empty string'
      },
      {
        fileBackend: {
          oss: {
            endpoint: 'https://oss-cn-hangzhou.aliyuncs.com',
            bucket: 'skiff-files',
            accessKeyIdEnv: 'SKIFF_OSS_ACCESS_KEY_ID'
          }
        },
        error:
          'invalid router.control envelope: fileBackend.oss requires accessKeySecretEnv or accessKeySecret'
      }
    ];

    for (const { fileBackend, error } of cases) {
      expect(
        validateRouterToRuntimeFrameHeader({
          ...runtimeFrameHeaderFixtures['router.control'],
          fileBackend
        })
      ).toEqual({
        ok: false,
        error
      });
    }
  });
});

describe('runtime binary frame foundations', () => {
  it('covers the runtime binary frame header set', () => {
    for (const type of runtimeFrameHeaderTypes) {
      expect(runtimeFrameHeaderSchemas[type]).toBeDefined();
      expect(runtimeFrameHeaderFixtures[type]).toBeDefined();
      expect(runtimeFrameHeaderSchemas[type].properties.type.enum).toContain(type);
      expect(runtimeFrameHeaderFixtures[type].type).toBe(type);
      expect(runtimeFrameHeaderFixtures[type]).not.toHaveProperty('payload');
      expect(runtimeFrameHeaderFixtures[type]).not.toHaveProperty('payloadBytes');
      expect(runtimeFrameHeaderFixtures[type]).not.toHaveProperty('args');
    }
  });

  it('keeps frame header fixtures valid for their transport direction', () => {
    for (const type of runtimeToRouterFrameHeaderTypes) {
      expect(validateRuntimeToRouterFrameHeader(runtimeFrameHeaderFixtures[type])).toEqual({
        ok: true,
        envelope: runtimeFrameHeaderFixtures[type]
      });
    }

    for (const type of routerToRuntimeFrameHeaderTypes) {
      expect(validateRouterToRuntimeFrameHeader(runtimeFrameHeaderFixtures[type])).toEqual({
        ok: true,
        envelope: runtimeFrameHeaderFixtures[type]
      });
    }
  });

  it('keeps spawn submit schema function-only', () => {
    const properties = runtimeFrameHeaderSchemas['spawn.submit.request'].properties;
    expect(properties.targetKind.enum).toEqual(['function']);
    expect(properties).not.toHaveProperty('actorRef');
    expect(properties).not.toHaveProperty('methodName');
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['spawn.submit.request'],
        actorRef: runtimeFrameHeaderFixtures['actor.put.response'].actorRef,
        methodName: 'receive'
      })
    ).toEqual({
      ok: false,
      error: 'invalid spawn.submit.request envelope: actorRef is not supported'
    });
  });

  it('accepts request.start serviceId for runtime lazy artifact loading', () => {
    const header = {
      ...runtimeFrameHeaderFixtures['request.start'],
      serviceId: 'example.com/hello'
    };

    expect(validateRouterToRuntimeFrameHeader(header)).toEqual({
      ok: true,
      envelope: header
    });
    expect(
      validateRouterToRuntimeFrameHeader({
        ...header,
        serviceId: 'not-a-publication-id'
      })
    ).toEqual({
      ok: false,
      error: 'invalid request.start envelope: serviceId must be a publication id'
    });
  });

  it('round-trips typed headers and opaque payload bytes', () => {
    const payload = new Uint8Array([0, 1, 2, 123, 34, 255]);
    const encoded = encodeRuntimeFrame(runtimeFrameHeaderFixtures['request.start'], payload);
    const decoded = decodeRuntimeFrame(encoded);

    expect(decoded.header).toEqual(runtimeFrameHeaderFixtures['request.start']);
    expect([...decoded.payloadBytes]).toEqual([...payload]);
    expect(validateRouterToRuntimeFrameHeader(decoded.header)).toEqual({
      ok: true,
      envelope: runtimeFrameHeaderFixtures['request.start']
    });
  });

  it('models operation calls as metadata header plus opaque operation payload bytes', () => {
    const operationPayload = Buffer.from([0xde, 0xad, 0xbe, 0xef, 0, 1, 2]);
    const decoded = decodeRuntimeFrame(
      encodeRuntimeFrame(runtimeFrameHeaderFixtures['request.start'], operationPayload)
    );

    expect(decoded.header).toEqual(runtimeFrameHeaderFixtures['request.start']);
    expect([...decoded.payloadBytes]).toEqual([...operationPayload]);
    expect(decoded.header).not.toHaveProperty('args');
    expect(decoded.header).not.toHaveProperty('payload');
    expect(JSON.stringify(decoded.header)).not.toContain('__skiffBytesBase64');
  });

  it('models HTTP ingress as request metadata header plus raw payload bytes', () => {
    const body = Buffer.from([0, 1, 2, 123, 34, 255]);
    const header = {
      ...runtimeFrameHeaderFixtures['request.start'],
      httpRequest: {
        method: 'POST',
        url: 'http://hello.local/raw/a%20b?x=1&x=2',
        path: '/raw/a b',
        query: [
          { name: 'x', value: '1' },
          { name: 'x', value: '2' }
        ],
        headers: [
          { name: 'host', value: 'hello.local' },
          { name: 'content-type', value: 'application/octet-stream' }
        ]
      }
    };

    const decoded = decodeRuntimeFrame(encodeRuntimeFrame(header, body));

    expect(decoded.header).toEqual(header);
    expect([...decoded.payloadBytes]).toEqual([...body]);
    expect(decoded.header).toHaveProperty('httpRequest');
    expect(decoded.header).not.toHaveProperty('args');
    expect((decoded.header as Record<string, unknown>).httpRequest).not.toHaveProperty('body');
    expect(JSON.stringify(decoded.header)).not.toContain('__skiffBytesBase64');
    expect(validateRouterToRuntimeFrameHeader(decoded.header)).toEqual({
      ok: true,
      envelope: header
    });
  });

  it('models HTTP egress as response metadata header plus raw payload bytes', () => {
    const body = Buffer.from([255, 0, 1, 2, 123, 34]);
    const header = {
      ...runtimeFrameHeaderFixtures['response.end'],
      httpResponse: {
        status: 202,
        headers: [
          { name: 'content-type', value: 'application/octet-stream' },
          { name: 'set-cookie', value: 'a=1; Path=/' }
        ]
      }
    };

    const decoded = decodeRuntimeFrame(encodeRuntimeFrame(header, body));

    expect(decoded.header).toEqual(header);
    expect([...decoded.payloadBytes]).toEqual([...body]);
    expect(decoded.header).toHaveProperty('httpResponse');
    expect(decoded.header).not.toHaveProperty('payload');
    expect(decoded.header).not.toHaveProperty('body');
    expect(JSON.stringify(decoded.header)).not.toContain('__skiffBytesBase64');
    expect(validateRuntimeToRouterFrameHeader(decoded.header)).toEqual({
      ok: true,
      envelope: header
    });
  });

  it('allows header-only register, control, cancel, and error frames', () => {
    const runtimeToRouterHeaderOnly = [
      'runtime.register',
      'runtime.health',
      'request.cancel',
      'response.error'
    ] as const satisfies readonly RuntimeToRouterFrameHeaderName[];
    for (const type of runtimeToRouterHeaderOnly) {
      const encoded = encodeRuntimeFrame(runtimeFrameHeaderFixtures[type]);
      const decoded = decodeRuntimeFrame(encoded);

      expect(decoded.header).toEqual(runtimeFrameHeaderFixtures[type]);
      expect(decoded.payloadBytes.byteLength).toBe(0);
      expect(validateRuntimeToRouterFrameHeader(decoded.header)).toEqual({
        ok: true,
        envelope: runtimeFrameHeaderFixtures[type]
      });
    }

    const routerToRuntimeHeaderOnly = [
      'router.control',
      'request.cancel'
    ] as const satisfies readonly RouterToRuntimeFrameHeaderName[];
    for (const type of routerToRuntimeHeaderOnly) {
      const encoded = encodeRuntimeFrame(runtimeFrameHeaderFixtures[type]);
      const decoded = decodeRuntimeFrame(encoded);

      expect(decoded.header).toEqual(runtimeFrameHeaderFixtures[type]);
      expect(decoded.payloadBytes.byteLength).toBe(0);
      expect(validateRouterToRuntimeFrameHeader(decoded.header)).toEqual({
        ok: true,
        envelope: runtimeFrameHeaderFixtures[type]
      });
    }
  });

  it('rejects legacy JSON text envelopes instead of parsing args or payload JSON', () => {
    const legacyRequestStart = JSON.stringify({
      type: 'request.start',
      requestId: 'request-fixture-1',
      args: { name: 'Ada' }
    });
    const legacyResponseChunk = JSON.stringify({
      type: 'response.chunk',
      requestId: 'request-fixture-1',
      seq: 0,
      payload: { token: 'hello' }
    });
    const legacyResponseEnd = JSON.stringify({
      type: 'response.end',
      requestId: 'request-fixture-1',
      payload: { message: 'hello' }
    });

    expect(() => decodeRuntimeFrame(legacyRequestStart)).toThrow(
      'invalid skiff binary frame: expected skiff binary frame magic'
    );
    expect(() => decodeRuntimeFrame(legacyResponseChunk)).toThrow(
      'invalid skiff binary frame: expected skiff binary frame magic'
    );
    expect(() => decodeRuntimeFrame(legacyResponseEnd)).toThrow(
      'invalid skiff binary frame: expected skiff binary frame magic'
    );
  });

  it('requires schemaVersion on binary runtime frames', () => {
    const { schemaVersion: _schemaVersion, ...requestStart } =
      runtimeFrameHeaderFixtures['request.start'];
    const { schemaVersion: _responseSchemaVersion, ...responseEnd } =
      runtimeFrameHeaderFixtures['response.end'];

    expect(() => decodeRuntimeFrame(encodeBinaryFrame(requestStart))).toThrow(
      'invalid skiff runtime frame: schemaVersion must be skiff-runtime-frame-v1'
    );
    expect(() => decodeRuntimeFrame(encodeBinaryFrame(responseEnd))).toThrow(
      'invalid skiff runtime frame: schemaVersion must be skiff-runtime-frame-v1'
    );
    expect(validateRouterToRuntimeFrameHeader(requestStart)).toEqual({
      ok: false,
      error:
        'invalid request.start frame header envelope: schemaVersion must be one of skiff-runtime-frame-v1'
    });
    expect(validateRuntimeToRouterFrameHeader(responseEnd)).toEqual({
      ok: false,
      error:
        'invalid response.end frame header envelope: schemaVersion must be one of skiff-runtime-frame-v1'
    });
  });

  it('rejects legacy payload fields in frame headers', () => {
    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['request.start'],
        args: {
          name: 'Ada'
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start frame header: args is not supported; use binary frame payload bytes'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['response.end'],
        payload: {
          message: 'hello'
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid response.end frame header: payload is not supported; use binary frame payload bytes'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['response.chunk'],
        payload: {
          token: 'hello'
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid response.chunk frame header: payload is not supported; use binary frame payload bytes'
    });
  });

  it('rejects legacy HTTP body shims inside frame headers', () => {
    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['request.start'],
        httpRequest: {
          method: 'POST',
          url: 'http://hello.local/bytes',
          path: '/bytes',
          query: [],
          headers: [],
          body: {
            __skiffBytesBase64: Buffer.from('legacy request body').toString('base64')
          }
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start frame header: httpRequest.body is not supported; use binary frame payload bytes'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['response.end'],
        httpResponse: {
          status: 200,
          headers: [],
          body: {
            __skiffBytesBase64: Buffer.from('legacy response body').toString('base64')
          }
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid response.end frame header: httpResponse.body is not supported; use binary frame payload bytes'
    });
  });

  it('rejects legacy websocket connectionPolicy shapes at the runtime protocol boundary', () => {
    const websocketConnect = {
      result: 'accept',
      businessIdentity: 'user-1',
      contextPayloadPresent: false,
      connectionPolicy: {
        maxConnections: 1,
        overflow: 'close-oldest'
      }
    };

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['response.end'],
        websocketConnect: {
          ...websocketConnect,
          connectionPolicy: {
            ...websocketConnect.connectionPolicy,
            scope: 'identity'
          }
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid response.end envelope: websocketConnect.connectionPolicy.scope is not supported'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['response.end'],
        websocketConnect: {
          ...websocketConnect,
          connectionPolicy: {
            maxConnections: 1,
            overflow: 'drop-new'
          }
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid response.end envelope: websocketConnect.connectionPolicy.overflow must be one of close-oldest, reject-new'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['response.end'],
        websocketConnect: {
          ...websocketConnect,
          connectionPolicy: {
            maxConnections: 0,
            overflow: 'close-oldest'
          }
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid response.end envelope: websocketConnect.connectionPolicy.maxConnections must be a positive integer'
    });
  });

  it('rejects malformed businessIdentity targets in connection.send frame headers', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['connection.send'],
        businessIdentity: '  '
      })
    ).toEqual({
      ok: false,
      error: 'invalid connection.send envelope: businessIdentity must be a non-empty string'
    });
  });

  it('requires exactly one connection.send target in frame headers', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['connection.send'],
        businessIdentity: 'user-1',
        connectionId: 'connection-1'
      })
    ).toEqual({
      ok: false,
      error: 'invalid connection.send envelope: exactly one of businessIdentity or connectionId must be set'
    });

    const {
      businessIdentity: _businessIdentity,
      websocketEntryId: _websocketEntryId,
      ...withoutTarget
    } = runtimeFrameHeaderFixtures['connection.send'];
    expect(validateRuntimeToRouterFrameHeader(withoutTarget)).toEqual({
      ok: false,
      error: 'invalid connection.send envelope: exactly one of businessIdentity or connectionId must be set'
    });
  });

  it('accepts only known connection.send frame payload kinds', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['connection.send'],
        payloadKind: 'text'
      })
    ).toEqual({
      ok: true,
      envelope: {
        ...runtimeFrameHeaderFixtures['connection.send'],
        payloadKind: 'text'
      }
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['connection.send'],
        payloadKind: 'json'
      })
    ).toEqual({
      ok: false,
      error: 'invalid connection.send envelope: payloadKind must be one of text, binary'
    });
  });
});

describe('runtime payload codec', () => {
  it('encodes runtime payloads with the v2 binary codec version', () => {
    const payload = encodeRuntimePayload('ok', { type: 'string' });

    expect(payload.subarray(0, 4).toString('ascii')).toBe('SKPV');
    expect(payload[4]).toBe(2);
  });

  it('encodes json object payloads without the legacy representation envelope', () => {
    const payload = encodeRuntimePayload({ name: 'Ada' }, { type: 'json' });

    expect(payload[5]).toBe(7);
    expect(payload.readUInt32LE(6)).toBe(1);
  });

  it('round trips Date schemas as epoch millisecond payloads', () => {
    const schema = {
      type: 'string',
      format: 'date-time',
      xSkiffSymbol: 'Date'
    } satisfies JsonSchema;

    const payload = encodeRuntimePayload(new Date('1970-01-01T00:00:00.000Z'), schema);

    expect(payload[5]).toBe(10);
    expect(payload.includes(Buffer.from('1970-01-01'))).toBe(false);
    expect(decodeRuntimePayload(payload, schema)).toEqual(new Date('1970-01-01T00:00:00.000Z'));
  });

  it('encodes multi-value string enums as literal union payloads', () => {
    const schema = {
      type: 'string',
      enum: ['user', 'host']
    } satisfies JsonSchema;

    const payload = encodeRuntimePayload('host', schema);

    expect(payload[5]).toBe(1);
    expect(payload[6]).toBe(4);
    expect(decodeRuntimePayload(payload, schema)).toBe('host');
  });

  it('rejects legacy bytes shims for typed bytes payloads', () => {
    const schema = {
      type: 'object',
      properties: {
        body: { type: 'json', xSkiffSymbol: 'std.bytes.bytes' }
      },
      required: ['body'],
      additionalProperties: false
    } satisfies JsonSchema;

    expect(() =>
      encodeRuntimePayload(
        {
          body: {
            __skiffBytesBase64: Buffer.from('legacy body').toString('base64')
          }
        },
        schema
      )
    ).toThrow('expected bytes at payload.body');
  });
});
