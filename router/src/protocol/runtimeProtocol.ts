import { TextDecoder } from 'node:util';

import {
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION,
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS,
  TELEMETRY_VISIBILITIES,
  isRecord,
  type RequestCancelReason,
  type ResponseErrorFrameHeader,
  type RouterToRuntimeFrameHeader,
  type RuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
  type RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
  type RuntimeFrameHeader,
  type RuntimeFrameHeaderName,
  type RuntimeToRouterFrameHeader,
  type TelemetryEvent,
  type TelemetryTopic
} from './envelope.js';
import {
  REQUEST_CANCEL_REASONS,
  REQUEST_CANCEL_REASON_BY_SITUATION
} from './cancelReason.js';
import { CONFIG_SHAPE_VALUE_TYPES, isConfigShapeValueType } from '../config/index.js';
import { isPublicationId, publicationStorageSegment } from '../publicationId.js';
import {
  hasRuntimeAssemblyRouting,
  normalizeRuntimeAssemblyRequestStartHeader,
  type RuntimeAssemblyRequestStartFrameHeader,
  type RuntimeAssemblyRequestStartFrameTransportWireHeader,
  type RuntimeAssemblyRequestStartFrameWireHeader,
  validateRuntimeAssemblyRequestStartHeader
} from './runtimeAssemblyRequest.js';
import {
  activationGeneration,
  activationToken,
  runtimeAssemblyIdentity
} from './assemblyActivationLexical.js';

export type RuntimeProtocolFrameHeaderName = RuntimeFrameHeaderName;
export type RuntimeToRouterFrameHeaderName = RuntimeToRouterFrameHeader['type'];
export type RouterToRuntimeFrameHeaderName = RouterToRuntimeFrameHeader['type'];

export interface ProtocolSchemaProperty {
  type: string | readonly string[];
  enum?: readonly (string | number | boolean | null)[];
  minLength?: number;
  pattern?: string;
  minimum?: number;
  maximum?: number;
  required?: readonly string[];
  properties?: Record<string, ProtocolSchemaProperty>;
  items?: ProtocolSchemaProperty;
  additionalProperties?: boolean;
}

export interface ProtocolEnvelopeObjectSchema {
  type: 'object';
  required: readonly string[];
  properties: Record<string, ProtocolSchemaProperty>;
  additionalProperties: boolean;
}

export interface ProtocolEnvelopeOneOfSchema {
  oneOf: readonly ProtocolEnvelopeObjectSchema[];
}

export type ProtocolEnvelopeSchema =
  | ProtocolEnvelopeObjectSchema
  | ProtocolEnvelopeOneOfSchema;

type FrameHeaderFixtureMap = {
  [Type in RuntimeProtocolFrameHeaderName]: Extract<RuntimeFrameHeader, { type: Type }>;
};

export type EnvelopeValidationResult<TEnvelope> =
  | {
      ok: true;
      envelope: TEnvelope;
    }
  | {
      ok: false;
      error: string;
    };

export interface PublicTypedServiceErrorEnvelopeView {
  readonly kind: 'publicTypedError';
  readonly packageId: string;
  readonly stableSchemaKey: string;
  readonly packageSchemaTypeId: string;
  readonly encodedPayload: readonly number[];
  readonly traceId: string;
  readonly errorId: string;
}

export interface InternalServiceErrorEnvelopeView {
  readonly kind: 'internalError';
  readonly payload: {
    readonly message: string;
    readonly traceId: string;
    readonly errorId: string;
  };
}

export interface PlatformServiceErrorEnvelopeView {
  readonly kind: 'platformError';
  readonly builtinErrorIdentity: string;
  readonly encodedPayload: readonly number[];
  readonly traceId: string;
  readonly errorId: string;
}

export type ServiceErrorEnvelopeView =
  | PublicTypedServiceErrorEnvelopeView
  | InternalServiceErrorEnvelopeView
  | PlatformServiceErrorEnvelopeView;

export type ValidatedResponseErrorFrame =
  | {
      readonly header: Extract<ResponseErrorFrameHeader, { errorKind: 'fixedService' }>;
      readonly payloadBytes: Uint8Array;
      readonly serviceError: ServiceErrorEnvelopeView;
    }
  | {
      readonly header: Extract<ResponseErrorFrameHeader, { errorKind: 'control' }>;
      readonly payloadBytes: Uint8Array;
    };

const runtimeToRouterFrameHeaderTypes = [
  'runtime.register',
  'runtime.capabilities',
  'runtime.health',
  'actor.getOrCreate.request',
  'actor.replace.request',
  'actor.find.request',
  'actor.remove.request',
  'spawn.submit.request',
  'request.start',
  'request.cancel',
  'connection.send',
  'connection.request',
  'connection.request.cancel',
  'response.start',
  'response.chunk',
  'response.end',
  'response.error'
] as const satisfies readonly RuntimeToRouterFrameHeaderName[];

const routerToRuntimeFrameHeaderTypes = [
  'router.bootstrap',
  'router.control',
  'runtime.registered',
  'actor.getOrCreate.response',
  'actor.getOrCreate.error',
  'actor.replace.response',
  'actor.replace.error',
  'actor.find.response',
  'actor.find.error',
  'actor.remove.response',
  'actor.remove.error',
  'spawn.submit.response',
  'spawn.submit.error',
  'request.start',
  'package-test.start',
  'request.cancel',
  'connection.response',
  'response.start',
  'response.chunk',
  'response.end',
  'response.error'
] as const satisfies readonly RouterToRuntimeFrameHeaderName[];

const SERVICE_PROTOCOL_IDENTITY_PATTERN =
  /^skiff-service-protocol-v5:sha256:[0-9a-f]{64}$/;
const GATEWAY_IDENTITY_PATTERN = /^skiff-gateway-v1:sha256:[0-9a-f]{64}$/;
const BUILD_ID_PATTERN = /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/;
const PACKAGE_TEST_BUILD_ID_PATTERN = /^skiff-package-test-build-v1:sha256:[0-9a-f]{64}$/;
const PACKAGE_BUILD_ID_PATTERN =
  /^skiff-package-build-v10:sha256:[0-9a-f]{64}$/;
const PACKAGE_TEST_ENTRYPOINT_ID_PATTERN = /^skiff-package-test-entrypoint-v1:sha256:[0-9a-f]{64}$/;
const PACKAGE_TEST_ACTIVATION_ID_PATTERN = /^skiff-package-test-run-v1:[A-Za-z0-9._:~-]+$/;
const ACTIVATION_IDENTITY_PATTERN = /^skiff-runtime-activation-v1:opaque:[A-Za-z0-9._:-]+$/;
const RESOLVED_CONFIG_IDENTITY_PATTERN =
  /^skiff-config-resolved-v1:opaque:[A-Za-z0-9._:-]+$/;
const CONFIG_REDACTION_IDENTITY_PATTERN =
  /^skiff-config-redaction-v1:sha256:[0-9a-f]{64}$/;
const REVISION_ID_PATTERN = /^[0-9a-f]{64}$/;
const ACTOR_ID_HASH_PATTERN = /^sha256:[0-9a-f]{64}$/;
const BASE64_PATTERN = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const INTERNAL_CANCELLATION_RESERVED_ERROR_CODE = ['Cancel', 'Error'].join('');
const PLATFORM_SERVICE_ERROR_IDENTITIES = [
  'TimeoutError',
  'config.DecodeError',
  'std.bytes.DecodeError',
  'std.number.DecodeError',
  'std.json.DecodeError',
  'std.db.ConflictError',
  'std.db.DecodeError',
  'std.file.FileError',
  'std.time.DecodeError',
  'std.service.ProviderUnavailableError',
  'std.service.ProtocolError',
  'std.http.HttpError'
] as const;
const TELEMETRY_EVENT_SOURCES = ['gateway', 'router', 'runtime', 'provider', 'test'] as const;
const TELEMETRY_EVENT_LEVELS = ['debug', 'info', 'warn', 'error'] as const;
const TELEMETRY_EVENT_STRING_FIELDS = [
  'serviceId',
  'revisionId',
  'buildId',
  'activationIdentity',
  'runtimeId',
  'providerId',
  'providerRevision',
  'providerCapability',
  'providerTarget',
  'requestId',
  'clientRequestId',
  'traceId',
  'errorId',
  'spanId',
  'parentSpanId',
  'target',
  'name',
  'message'
] as const;
const TELEMETRY_EVENT_OBJECT_FIELDS = ['attrs', 'error', 'dropped'] as const;
const TELEMETRY_EVENT_FIELDS = new Set<string>([
  'topic',
  'ts',
  'source',
  'visibility',
  ...TELEMETRY_EVENT_STRING_FIELDS,
  'level',
  ...TELEMETRY_EVENT_OBJECT_FIELDS,
  'durationMs'
]);
const RFC3339_TIMESTAMP_PATTERN =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/;

const configShapeProtocolSchema = {
  type: 'object',
  required: ['schemaVersion', 'entries'],
  properties: {
    schemaVersion: { type: 'string', enum: ['skiff-config-shape-v1'] },
    entries: {
      type: 'array',
      items: {
        type: 'object',
        required: ['path', 'type', 'required'],
        properties: {
          path: { type: 'string' },
          type: { type: 'string', enum: CONFIG_SHAPE_VALUE_TYPES },
          required: { type: 'boolean' }
        },
        additionalProperties: false
      }
    }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const cancelReasons = REQUEST_CANCEL_REASONS satisfies readonly RequestCancelReason[];

const spawnTargetKinds = ['function'] as const;
const dispatchModes = ['unary', 'serverStream'] as const;
const websocketAdapterSourceKinds = [
  'websocket.connectRequest',
  'websocket.receiveEvent',
  'websocket.connection',
  'websocket.connectionContext',
  'websocket.message',
  'websocket.messageBody',
  'websocket.connectionId',
  'websocket.businessIdentity'
] as const;
const websocketPayloadSegmentKinds = ['websocket.context', 'websocket.message'] as const;

const runtimeCapabilitiesProtocolSchema = {
  type: 'object',
  properties: {
    dispatchModes: { type: 'array', items: { type: 'string', enum: dispatchModes } },
    packageTestDispatch: { type: 'boolean' },
    requestCancel: { type: 'boolean' },
    runtimeProgram: { type: 'boolean' }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const runtimeRegisterProperties = {
  type: { type: 'string', enum: ['runtime.register'] },
  runtimeId: { type: 'string' },
  serviceId: { type: 'string' },
  version: { type: 'string' },
  revisionId: { type: 'string' },
  activationIdentity: { type: 'string' },
  buildId: { type: 'string' },
  serviceProtocolIdentity: { type: 'string' },
  targets: { type: 'array', items: { type: 'string' } },
  runtimeVersion: { type: 'string' },
  codeRevisionId: { type: 'string' },
  artifactIdentity: { type: 'string' },
  gatewayEntryIdentities: { type: 'array', items: { type: 'string' } },
  capabilities: runtimeCapabilitiesProtocolSchema
} as const satisfies Record<string, ProtocolSchemaProperty>;

const runtimeCapabilitiesProperties = {
  type: { type: 'string', enum: ['runtime.capabilities'] },
  runtimeId: { type: 'string' },
  capabilities: runtimeCapabilitiesProtocolSchema
} as const satisfies Record<string, ProtocolSchemaProperty>;

const runtimeHealthCountersProperties = {
  outboundRequestsPending: { type: 'integer' },
  outboundStreamLeasesActive: { type: 'integer' },
  streamRuntimeStreamsActive: { type: 'integer' },
  flagBackedCancelWaitersActive: { type: 'integer' },
  spawnedTasksActive: { type: 'integer' }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const runtimeHealthCountersProtocolSchema = {
  type: 'object',
  required: Object.keys(runtimeHealthCountersProperties),
  properties: runtimeHealthCountersProperties,
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const runtimeHealthProperties = {
  type: { type: 'string', enum: ['runtime.health'] },
  runtimeId: { type: 'string' },
  observedAt: { type: 'string' },
  counters: runtimeHealthCountersProtocolSchema
} as const satisfies Record<string, ProtocolSchemaProperty>;

const runtimeRegisteredProperties = {
  type: { type: 'string', enum: ['runtime.registered'] },
  runtimeId: { type: 'string' }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const routerBootstrapProperties = {
  type: { type: 'string', enum: ['router.bootstrap'] },
  artifactsPath: { type: 'string' },
  serviceDb: {
    type: 'object',
    required: ['mongoUrl'],
    properties: {
      mongoUrl: { type: 'string' }
    },
    additionalProperties: false
  },
  http: {
    type: 'object',
    required: ['maxResponseBytes'],
    properties: {
      maxResponseBytes: { type: 'integer' }
    },
    additionalProperties: false
  },
  activation: {
    type: 'object',
    required: ['environment', 'generation', 'assembly', 'configSnapshot'],
    properties: {
      environment: { type: 'string' },
      generation: { type: 'integer' },
      assembly: {
        type: 'object',
        required: ['assemblyIdentity'],
        properties: {
          assemblyIdentity: { type: 'string' }
        },
        additionalProperties: false
      },
      configSnapshot: {
        type: 'object',
        required: ['snapshotId'],
        properties: {
          snapshotId: { type: 'string' }
        },
        additionalProperties: false
      }
    },
    additionalProperties: false
  }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const routerControlProperties = {
  type: { type: 'string', enum: ['router.control'] },
  artifactRoots: { type: 'array', items: { type: 'string' } },
  devReload: { type: 'boolean' },
  mode: { type: 'string', enum: ['dev', 'release'] },
  generation: { type: 'string' },
  fingerprint: { type: 'string' },
  telemetry: {
    type: 'object',
    required: [
      'endpoint',
      'protocol',
      'topics',
      'queueMaxEvents',
      'batchMaxEvents',
      'batchMaxBytes',
      'flushIntervalMs',
      'enabled'
    ],
    properties: {
      endpoint: { type: 'string' },
      protocol: { type: 'string', enum: [TELEMETRY_PROTOCOL] },
      topics: { type: 'array', items: { type: 'string', enum: TELEMETRY_TOPICS } },
      queueMaxEvents: { type: 'integer' },
      batchMaxEvents: { type: 'integer' },
      batchMaxBytes: { type: 'integer' },
      flushIntervalMs: { type: 'integer' },
      enabled: { type: 'boolean' }
    },
    additionalProperties: false
  },
  fileBackend: {
    type: 'object',
    properties: {
      local: {
        type: 'object',
        required: ['root'],
        properties: {
          root: { type: 'string' }
        },
        additionalProperties: false
      },
      oss: {
        type: 'object',
        required: ['endpoint', 'bucket'],
        properties: {
          endpoint: { type: 'string' },
          bucket: { type: 'string' },
          region: { type: 'string' },
          accessKeyId: { type: 'string' },
          accessKeySecret: { type: 'string' },
          accessKeyIdEnv: { type: 'string' },
          accessKeySecretEnv: { type: 'string' }
        },
        additionalProperties: false
      }
    },
    additionalProperties: false
  },
  serviceConfig: {
    type: 'array',
    items: {
      type: 'object',
      required: [
        'serviceId',
        'buildId',
        'activationIdentity',
        'resolvedConfigIdentity',
        'resolvedConfig',
        'redactedResolvedConfig',
        'redactionProjectionIdentity'
      ],
      properties: {
        serviceId: { type: 'string' },
        buildId: { type: 'string' },
        activationIdentity: { type: 'string' },
        resolvedConfigIdentity: { type: 'string' },
        resolvedConfig: { type: 'object', additionalProperties: true },
        redactedResolvedConfig: { type: 'object', additionalProperties: true },
        redactionProjectionIdentity: { type: 'string' },
        configShape: configShapeProtocolSchema,
        serviceDb: {
          type: 'object',
          required: ['mongoUrl', 'storageServiceId'],
          properties: {
            mongoUrl: { type: 'string' },
            storageServiceId: { type: 'string' }
          },
          additionalProperties: false
        },
        packageConfigs: {
          type: 'array',
          items: {
            type: 'object',
            required: [
              'packageId',
              'alias',
              'resolvedConfigIdentity',
              'resolvedConfig',
              'redactedResolvedConfig',
              'redactionProjectionIdentity'
            ],
            properties: {
              packageId: { type: 'string' },
              packageSlot: { type: 'integer' },
              alias: { type: 'string' },
              resolvedConfigIdentity: { type: 'string' },
              resolvedConfig: { type: 'object', additionalProperties: true },
              redactedResolvedConfig: { type: 'object', additionalProperties: true },
              redactionProjectionIdentity: { type: 'string' },
              configShape: configShapeProtocolSchema
            },
            additionalProperties: false
          }
        }
      },
      additionalProperties: false
    }
  }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const actorKeyProperties = {
  serviceId: { type: 'string' },
  actorTypeIdentity: { type: 'string' },
  actorIdTypeIdentity: { type: 'string' },
  actorIdEncodingVersion: { type: 'string' },
  canonicalActorIdKeyBytesBase64: { type: 'string' },
  actorIdHash: { type: 'string' }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const actorKeySchema = {
  type: 'object',
  required: [
    'serviceId',
    'actorTypeIdentity',
    'actorIdTypeIdentity',
    'actorIdEncodingVersion',
    'canonicalActorIdKeyBytesBase64'
  ],
  properties: actorKeyProperties,
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const actorDeclarationOwnerSchema = {
  type: 'object',
  required: ['unit', 'file', 'actorSymbol'],
  properties: {
    unit: {
      type: 'object',
      required: ['kind'],
      properties: {
        kind: { type: 'string', enum: ['service', 'package'] },
        value: { type: 'integer', minimum: 0 }
      },
      additionalProperties: false
    },
    file: {
      type: 'object',
      required: ['kind', 'value'],
      properties: {
        kind: { type: 'string', enum: ['loadedFileIndex', 'fileIrIdentity'] },
        value: { type: ['string', 'integer'] }
      },
      additionalProperties: false
    },
    actorSymbol: { type: 'string', minLength: 1 }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const actorMethodDeadlineSchema = {
  type: 'object',
  required: ['timeoutMs', 'expiresAt'],
  properties: {
    timeoutMs: { type: 'integer', minimum: 1 },
    expiresAt: { type: 'string', minLength: 1 }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const actorRefSchema = {
  type: 'object',
  required: [
    'serviceId',
    'actorTypeIdentity',
    'actorIdTypeIdentity',
    'actorIdEncodingVersion',
    'canonicalActorIdKeyBytesBase64',
    'actorIdHash'
  ],
  properties: {
    ...actorKeyProperties,
    epoch: { type: 'integer' }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const activationIdentitySchema = {
  type: 'object',
  required: [
    'assemblyIdentity',
    'generation',
    'runtimeReplicaId',
    'deploymentRevision'
  ],
  properties: {
    assemblyIdentity: { type: 'string' },
    generation: { type: 'integer' },
    runtimeReplicaId: { type: 'string' },
    deploymentRevision: { type: 'string' }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const runtimeRpcRequestBaseProperties = {
  rpcId: { type: 'string' },
  runtimeId: { type: 'string' },
  activationIdentity: activationIdentitySchema
} as const satisfies Record<string, ProtocolSchemaProperty>;

const runtimeRpcResponseBaseProperties = {
  rpcId: { type: 'string' }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const runtimeControlErrorProperties = {
  rpcId: { type: 'string' },
  error: {
    type: 'object',
    required: ['code', 'message'],
    properties: {
      code: { type: 'string' },
      message: { type: 'string' },
      status: { type: 'integer' },
      details: { type: 'any' }
    },
    additionalProperties: true
  }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const requestStartFrameProperties = {
  type: { type: 'string', enum: ['request.start'] },
  requestId: { type: 'string' },
  mode: { type: 'string', enum: ['unary', 'serverStream'] },
  caller: {
    type: 'object',
    required: ['kind', 'target'],
    properties: {
      kind: { type: 'string', enum: ['gateway', 'service'] },
      target: { type: 'string' }
    },
    additionalProperties: false
  },
  target: { type: 'string' },
  serviceId: { type: 'string' },
  version: { type: 'string' },
  buildId: { type: 'string' },
  serviceProtocolIdentity: { type: 'string' },
  routing: {
    type: 'object',
    required: [
      'kind',
      'assemblyIdentity',
      'assemblyGeneration',
      'deployment',
      'gatewayEntryIdentity',
      'ingress'
    ],
    properties: {
      kind: { type: 'string', enum: ['runtimeAssembly'] },
      assemblyIdentity: { type: 'string' },
      assemblyGeneration: { type: 'integer' },
      deployment: {
        type: 'object',
        required: [
          'serviceId',
          'contractVersion',
          'deploymentRevision',
          'deploymentArtifactIdentity'
        ],
        properties: {
          serviceId: { type: 'string', minLength: 1 },
          contractVersion: { type: 'string', minLength: 1 },
          deploymentRevision: { type: 'string', minLength: 1 },
          deploymentArtifactIdentity: {
            type: 'string',
            pattern: '^skiff-deployment-artifact-v4:sha256:[0-9a-f]{64}$'
          }
        },
        additionalProperties: false
      },
      gatewayEntryIdentity: {
        type: 'string',
        pattern: '^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$'
      },
      ingress: {
        type: 'object',
        required: ['protocol', 'method', 'path'],
        properties: {
          protocol: { type: 'string', enum: ['http'] },
          method: { type: 'string' },
          path: { type: 'string' }
        },
        additionalProperties: false
      }
    },
    additionalProperties: false
  },
  activationIdentity: { type: 'string' },
  gatewayEntryIdentity: { type: 'string' },
  businessIdentity: { type: 'string' },
  websocketEntryId: { type: 'string' },
  clientSession: {
    type: 'object',
    required: ['id'],
    properties: {
      id: { type: 'string' }
    },
    additionalProperties: false
  },
  deadline: {
    type: 'object',
    required: ['timeoutMs', 'expiresAt'],
    properties: {
      timeoutMs: { type: 'number' },
      expiresAt: { type: 'string' }
    },
    additionalProperties: false
  },
  trace: {
    type: 'object',
    required: ['traceId', 'spanId'],
    properties: {
      traceId: { type: 'string' },
      spanId: { type: 'string' },
      parentSpanId: { type: 'string' },
      sampled: { type: 'boolean' }
    },
    additionalProperties: false
  },
  websocketAdapter: {
    type: 'object',
    required: ['kind', 'adapterArgs'],
    properties: {
      kind: { type: 'string', enum: ['connect', 'receive'] },
      adapterArgs: {
        type: 'array',
        items: {
          type: 'object',
          required: ['param', 'source'],
          properties: {
            param: { type: 'string' },
            source: {
              type: 'object',
              required: ['kind'],
              properties: {
                kind: { type: 'string', enum: websocketAdapterSourceKinds }
              },
              additionalProperties: false
            }
          },
          additionalProperties: false
        }
      },
      contextExpectation: {
        type: 'object',
        required: ['kind'],
        properties: {
          kind: { type: 'string', enum: ['null', 'typed'] },
          connectOperationAbiId: { type: 'string' },
          contextTypeIdentity: { type: 'string' }
        },
        additionalProperties: false
      },
      connectRequest: {
        type: 'object',
        required: ['connectionId', 'url', 'query', 'headers', 'cookies'],
        properties: {
          connectionId: { type: 'string' },
          url: { type: 'string' },
          query: {
            type: 'array',
            items: {
              type: 'object',
              required: ['name', 'value'],
              properties: {
                name: { type: 'string' },
                value: { type: 'string' }
              },
              additionalProperties: false
            }
          },
          headers: {
            type: 'array',
            items: {
              type: 'object',
              required: ['name', 'value'],
              properties: {
                name: { type: 'string' },
                value: { type: 'string' }
              },
              additionalProperties: false
            }
          },
          cookies: {
            type: 'array',
            items: {
              type: 'object',
              required: ['name', 'value'],
              properties: {
                name: { type: 'string' },
                value: { type: 'string' }
              },
              additionalProperties: false
            }
          },
          version: { type: 'string' }
        },
        additionalProperties: false
      },
      receiveEvent: {
        type: 'object',
        required: ['connectionId', 'message', 'payloadSegments'],
        properties: {
          connectionId: { type: 'string' },
          businessIdentity: { type: 'string' },
          message: {
            type: 'object',
            required: ['tag', 'encoding'],
            properties: {
              tag: { type: 'string', enum: ['text', 'binary'] },
              encoding: { type: 'string', enum: ['utf8', 'binary'] }
            },
            additionalProperties: false
          },
          payloadSegments: {
            type: 'array',
            items: {
              type: 'object',
              required: ['kind', 'offset', 'length'],
              properties: {
                kind: { type: 'string', enum: websocketPayloadSegmentKinds },
                offset: { type: 'integer' },
                length: { type: 'integer' }
              },
              additionalProperties: false
            }
          },
          contextCodec: {
            type: 'object',
            required: ['operationAbiId', 'contextTypeIdentity'],
            properties: {
              operationAbiId: { type: 'string' },
              contextTypeIdentity: { type: 'string' }
            },
            additionalProperties: false
          }
        },
        additionalProperties: false
      }
    },
    additionalProperties: false
  }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const packageTestStartFrameProperties = {
  type: { type: 'string', enum: ['package-test.start'] },
  requestId: { type: 'string' },
  caller: {
    type: 'object',
    required: ['kind', 'target'],
    properties: {
      kind: { type: 'string', enum: ['gateway'] },
      target: { type: 'string' }
    },
    additionalProperties: false
  },
  packageId: { type: 'string' },
  packageVersion: { type: 'string' },
  testBuildIdentity: { type: 'string' },
  entrypointId: { type: 'string' },
  activationId: { type: 'string' },
  deadline: requestStartFrameProperties.deadline,
  trace: requestStartFrameProperties.trace,
  testEffectsEnabled: { type: 'boolean' },
  testEffectDoubles: {
    type: 'object',
    additionalProperties: true
  }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const responseErrorNonBlankStringProperty = {
  type: 'string',
  minLength: 1,
  pattern: '\\S'
} as const satisfies ProtocolSchemaProperty;

const responseErrorCommonProperties = {
  schemaVersion: {
    type: 'string',
    enum: [RESPONSE_ERROR_FRAME_SCHEMA_VERSION]
  },
  type: { type: 'string', enum: ['response.error'] },
  requestId: responseErrorNonBlankStringProperty
} as const satisfies Record<string, ProtocolSchemaProperty>;

const responseErrorPayloadProperty = {
  type: 'object',
  required: ['code', 'message'],
  properties: {
    code: responseErrorNonBlankStringProperty,
    message: responseErrorNonBlankStringProperty,
    status: {
      type: 'integer',
      minimum: 400,
      maximum: 599
    },
    details: { type: 'any' }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const fixedServiceResponseErrorSchema = {
  type: 'object',
  required: ['schemaVersion', 'type', 'requestId', 'errorKind'],
  properties: {
    ...responseErrorCommonProperties,
    errorKind: { type: 'string', enum: ['fixedService'] }
  },
  additionalProperties: false
} as const satisfies ProtocolEnvelopeObjectSchema;

const controlResponseErrorSchema = {
  type: 'object',
  required: ['schemaVersion', 'type', 'requestId', 'errorKind', 'error'],
  properties: {
    ...responseErrorCommonProperties,
    errorKind: { type: 'string', enum: ['control'] },
    error: responseErrorPayloadProperty
  },
  additionalProperties: false
} as const satisfies ProtocolEnvelopeObjectSchema;

const responseErrorSchema = {
  oneOf: [fixedServiceResponseErrorSchema, controlResponseErrorSchema]
} as const satisfies ProtocolEnvelopeOneOfSchema;

const requestCancelProperties = {
  type: { type: 'string', enum: ['request.cancel'] },
  requestId: { type: 'string' },
  reason: { type: 'string', enum: cancelReasons }
} as const satisfies Record<string, ProtocolSchemaProperty>;

const connectionRequestDeadlineProperty = {
  type: 'object',
  required: ['timeoutMs', 'expiresAt'],
  properties: {
    timeoutMs: { type: 'integer', minimum: 1 },
    expiresAt: { type: 'string', minLength: 20 }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

const connectionRemoteErrorProperty = {
  type: 'object',
  required: ['code', 'message', 'dataPresent'],
  properties: {
    code: {
      type: 'integer',
      minimum: Number.MIN_SAFE_INTEGER,
      maximum: Number.MAX_SAFE_INTEGER
    },
    message: { type: 'string', minLength: 1 },
    dataPresent: { type: 'boolean' }
  },
  additionalProperties: false
} as const satisfies ProtocolSchemaProperty;

export const runtimeFrameHeaderSchemas = {
  'runtime.register': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'runtimeId',
      'serviceId',
      'revisionId',
      'buildId',
      'serviceProtocolIdentity',
      'targets'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...runtimeRegisterProperties
    },
    additionalProperties: false
  },
  'runtime.capabilities': {
    type: 'object',
    required: ['schemaVersion', 'type', 'runtimeId', 'capabilities'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...runtimeCapabilitiesProperties
    },
    additionalProperties: false
  },
  'runtime.health': {
    type: 'object',
    required: ['schemaVersion', 'type', 'runtimeId', 'observedAt', 'counters'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...runtimeHealthProperties
    },
    additionalProperties: false
  },
  'runtime.registered': {
    type: 'object',
    required: ['schemaVersion', 'type', 'runtimeId'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...runtimeRegisteredProperties
    },
    additionalProperties: false
  },
  'router.bootstrap': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'artifactsPath',
      'serviceDb',
      'http',
      'activation'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...routerBootstrapProperties
    },
    additionalProperties: false
  },
  'router.control': {
    type: 'object',
    required: ['schemaVersion', 'type', 'artifactRoots'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...routerControlProperties
    },
    additionalProperties: false
  },
  'actor.getOrCreate.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'rpcId',
      'runtimeId',
      'activationIdentity',
      'actorKey',
      'actorAbiIdentity',
      'actorImplementationIdentity',
      'bootstrapEncodingVersion',
      'declarationOwner'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.getOrCreate.request'] },
      ...runtimeRpcRequestBaseProperties,
      actorKey: actorKeySchema,
      actorAbiIdentity: { type: 'string' },
      actorImplementationIdentity: { type: 'string' },
      bootstrapEncodingVersion: { type: 'string' },
      declarationOwner: actorDeclarationOwnerSchema,
      deadline: actorMethodDeadlineSchema
    },
    additionalProperties: false
  },
  'actor.getOrCreate.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'actorRef'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.getOrCreate.response'] },
      ...runtimeRpcResponseBaseProperties,
      actorRef: actorRefSchema
    },
    additionalProperties: false
  },
  'actor.getOrCreate.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.getOrCreate.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'actor.replace.request': {
    type: 'object',
    required: [
      'schemaVersion', 'type', 'rpcId', 'runtimeId', 'activationIdentity', 'actorKey',
      'actorAbiIdentity', 'actorImplementationIdentity', 'bootstrapEncodingVersion',
      'declarationOwner'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.replace.request'] },
      ...runtimeRpcRequestBaseProperties,
      actorKey: actorKeySchema,
      actorAbiIdentity: { type: 'string' },
      actorImplementationIdentity: { type: 'string' },
      bootstrapEncodingVersion: { type: 'string' },
      declarationOwner: actorDeclarationOwnerSchema,
      deadline: actorMethodDeadlineSchema
    },
    additionalProperties: false
  },
  'actor.replace.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'actorRef'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.replace.response'] },
      ...runtimeRpcResponseBaseProperties,
      actorRef: actorRefSchema
    },
    additionalProperties: false
  },
  'actor.replace.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.replace.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'actor.find.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'rpcId',
      'runtimeId',
      'activationIdentity',
      'actorKey'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.find.request'] },
      ...runtimeRpcRequestBaseProperties,
      actorKey: actorKeySchema
    },
    additionalProperties: false
  },
  'actor.find.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'found'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.find.response'] },
      ...runtimeRpcResponseBaseProperties,
      found: { type: 'boolean' },
      actorRef: actorRefSchema
    },
    additionalProperties: false
  },
  'actor.find.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.find.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'actor.remove.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'rpcId',
      'runtimeId',
      'activationIdentity',
      'actorKey'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.remove.request'] },
      ...runtimeRpcRequestBaseProperties,
      actorKey: actorKeySchema
    },
    additionalProperties: false
  },
  'actor.remove.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'removed'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.remove.response'] },
      ...runtimeRpcResponseBaseProperties,
      removed: { type: 'boolean' }
    },
    additionalProperties: false
  },
  'actor.remove.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.remove.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'spawn.submit.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'rpcId',
      'runtimeId',
      'activationIdentity',
      'targetKind',
      'serviceId',
      'serviceVersion',
      'serviceProtocolIdentity',
      'target',
      'callerRequestId'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.submit.request'] },
      ...runtimeRpcRequestBaseProperties,
      targetKind: { type: 'string', enum: spawnTargetKinds },
      serviceId: { type: 'string' },
      serviceVersion: { type: 'string' },
      serviceProtocolIdentity: { type: 'string' },
      target: { type: 'string' },
      spawnId: { type: 'string' },
      buildId: { type: 'string' },
      callerRequestId: { type: 'string' },
      traceId: { type: 'string' },
      callerTarget: { type: 'string' }
    },
    additionalProperties: false
  },
  'spawn.submit.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'spawnId', 'requestId', 'status'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.submit.response'] },
      ...runtimeRpcResponseBaseProperties,
      spawnId: { type: 'string' },
      requestId: { type: 'string' },
      status: { type: 'string', enum: ['submitted'] }
    },
    additionalProperties: false
  },
  'spawn.submit.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.submit.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'request.start': {
    oneOf: [
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'mode',
          'caller',
          'trace'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['request.start'] },
          requestId: { type: 'string' },
          mode: { type: 'string', enum: ['unary', 'serverStream'] },
          caller: requestStartFrameProperties.caller,
          target: { type: 'string' },
          operationAbiId: { type: 'string' },
          selector: { type: 'string' },
          serviceId: { type: 'string' },
          version: { type: 'string' },
          buildId: { type: 'string' },
          serviceProtocolIdentity: { type: 'string' },
          activationIdentity: { type: 'string' },
          gatewayEntryIdentity: { type: 'string' },
          businessIdentity: { type: 'string' },
          websocketEntryId: { type: 'string' },
          clientSession: requestStartFrameProperties.clientSession,
          deadline: requestStartFrameProperties.deadline,
          trace: requestStartFrameProperties.trace,
          testEffectsEnabled: { type: 'boolean' },
          testEffectDoubles: {
            type: 'object',
            additionalProperties: true
          },
          httpRequest: {
            type: 'object',
            required: ['method', 'url', 'path', 'query', 'headers'],
            properties: {
              method: { type: 'string' },
              url: { type: 'string' },
              path: { type: 'string' },
              query: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              },
              headers: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              }
            },
            additionalProperties: false
          },
          httpAdapter: {
            type: 'object',
            required: ['kind', 'handler'],
            properties: {
              kind: { type: 'string', enum: ['typedJson', 'rawHttp'] },
              handler: { type: 'object', additionalProperties: true },
              guard: { type: 'object', additionalProperties: true },
              pre: { type: 'object', additionalProperties: true },
              adapterArgs: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['param', 'source'],
                  properties: {
                    param: { type: 'string' },
                    source: {
                      type: 'object',
                      required: ['kind'],
                      properties: {
                        kind: { type: 'string', enum: ['http.request', 'http.body', 'http.context'] }
                      },
                      additionalProperties: false
                    }
                  },
                  additionalProperties: false
                }
              }
            },
            additionalProperties: false
          },
          websocketAdapter: requestStartFrameProperties.websocketAdapter
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'mode',
          'caller',
          'routing',
          'trace',
          'httpRequest'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['request.start'] },
          requestId: { type: 'string' },
          mode: { type: 'string', enum: ['unary', 'serverStream'] },
          caller: {
            type: 'object',
            required: ['kind'],
            properties: {
              kind: { type: 'string', enum: ['gateway'] }
            },
            additionalProperties: false
          },
          routing: requestStartFrameProperties.routing,
          clientSession: requestStartFrameProperties.clientSession,
          deadline: requestStartFrameProperties.deadline,
          trace: requestStartFrameProperties.trace,
          httpRequest: {
            type: 'object',
            required: ['method', 'url', 'path', 'query', 'headers'],
            properties: {
              method: { type: 'string' },
              url: { type: 'string' },
              path: { type: 'string' },
              query: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              },
              headers: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              }
            },
            additionalProperties: false
          },
          testEffectsEnabled: { type: 'boolean' },
          testCaseCapability: { type: 'string' }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'mode',
          'caller',
          'routing',
          'trace',
          'websocketConnect'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['request.start'] },
          requestId: { type: 'string' },
          mode: { type: 'string', enum: ['unary'] },
          caller: {
            type: 'object',
            required: ['kind'],
            properties: {
              kind: { type: 'string', enum: ['gateway'] }
            },
            additionalProperties: false
          },
          routing: {
            type: 'object',
            required: [
              'kind',
              'assemblyIdentity',
              'assemblyGeneration',
              'deployment',
              'gatewayEntryIdentity',
              'ingress'
            ],
            properties: {
              kind: { type: 'string', enum: ['runtimeAssembly'] },
              assemblyIdentity: {
                type: 'string',
                pattern: '^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$'
              },
              assemblyGeneration: {
                type: 'integer',
                minimum: 0,
                maximum: Number.MAX_SAFE_INTEGER
              },
              deployment: requestStartFrameProperties.routing.properties.deployment,
              gatewayEntryIdentity: {
                type: 'string',
                pattern: '^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$'
              },
              ingress: {
                type: 'object',
                required: ['protocol', 'method', 'path'],
                properties: {
                  protocol: { type: 'string', enum: ['webSocket'] },
                  method: { type: 'null' },
                  path: { type: 'string', pattern: '^/' }
                },
                additionalProperties: false
              }
            },
            additionalProperties: false
          },
          clientSession: requestStartFrameProperties.clientSession,
          deadline: requestStartFrameProperties.deadline,
          trace: requestStartFrameProperties.trace,
          websocketConnect: {
            type: 'object',
            required: [
              'connectionId',
              'url',
              'query',
              'headers',
              'cookies',
              'websocketEntryId',
              'gatewayEntryIdentity'
            ],
            properties: {
              connectionId: {
                type: 'string',
                pattern: '^(?=.{1,255}$)[A-Za-z0-9._:~-]+$'
              },
              url: { type: 'string' },
              query: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              },
              headers: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              },
              cookies: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              },
              version: { type: 'string' },
              websocketEntryId: {
                type: 'string',
                pattern: '^skiff-websocket-entry-v1:sha256:[0-9a-f]{64}$'
              },
              gatewayEntryIdentity: {
                type: 'string',
                pattern: '^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$'
              }
            },
            additionalProperties: false
          },
          testEffectsEnabled: { type: 'boolean', enum: [false] }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'mode',
          'caller',
          'routing',
          'trace',
          'websocketJsonRpc'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['request.start'] },
          requestId: { type: 'string', minLength: 1 },
          mode: { type: 'string', enum: ['unary'] },
          caller: {
            type: 'object',
            required: ['kind'],
            properties: {
              kind: { type: 'string', enum: ['gateway'] }
            },
            additionalProperties: false
          },
          routing: {
            type: 'object',
            required: [
              'kind',
              'assemblyIdentity',
              'assemblyGeneration',
              'deployment',
              'gatewayEntryIdentity',
              'ingress'
            ],
            properties: {
              kind: { type: 'string', enum: ['runtimeAssembly'] },
              assemblyIdentity: {
                type: 'string',
                pattern: '^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$'
              },
              assemblyGeneration: {
                type: 'integer',
                minimum: 0,
                maximum: Number.MAX_SAFE_INTEGER
              },
              deployment: requestStartFrameProperties.routing.properties.deployment,
              gatewayEntryIdentity: {
                type: 'string',
                pattern: '^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$'
              },
              ingress: {
                type: 'object',
                required: ['protocol', 'method', 'path'],
                properties: {
                  protocol: { type: 'string', enum: ['webSocket'] },
                  method: { type: 'string', minLength: 1 },
                  path: { type: 'string', pattern: '^/' }
                },
                additionalProperties: false
              }
            },
            additionalProperties: false
          },
          clientSession: requestStartFrameProperties.clientSession,
          deadline: requestStartFrameProperties.deadline,
          trace: requestStartFrameProperties.trace,
          websocketJsonRpc: {
            type: 'object',
            required: [
              'profile',
              'connectionId',
              'websocketEntryId',
              'gatewayEntryIdentity'
            ],
            properties: {
              profile: { type: 'string', enum: ['jsonrpc-2.0-text'] },
              connectionId: {
                type: 'string',
                pattern: '^(?=.{1,255}$)[A-Za-z0-9._:~-]+$'
              },
              websocketEntryId: {
                type: 'string',
                pattern: '^skiff-websocket-entry-v1:sha256:[0-9a-f]{64}$'
              },
              gatewayEntryIdentity: {
                type: 'string',
                pattern: '^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$'
              },
              businessIdentity: { type: 'string', minLength: 1 }
            },
            additionalProperties: false
          },
          testEffectsEnabled: { type: 'boolean', enum: [false] }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'mode',
          'caller',
          'routing',
          'invocation',
          'trace',
          'testEffectsEnabled'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['request.start'] },
          requestId: { type: 'string' },
          mode: { type: 'string', enum: ['unary'] },
          caller: {
            type: 'object',
            required: ['kind'],
            properties: {
              kind: { type: 'string', enum: ['service'] }
            },
            additionalProperties: false
          },
          routing: {
            type: 'object',
            required: [
              'kind',
              'assemblyIdentity',
              'assemblyGeneration',
              'deployment'
            ],
            properties: {
              kind: { type: 'string', enum: ['runtimeAssembly'] },
              assemblyIdentity: {
                type: 'string',
                pattern: '^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$'
              },
              assemblyGeneration: {
                type: 'integer',
                minimum: 0,
                maximum: Number.MAX_SAFE_INTEGER
              },
              deployment: requestStartFrameProperties.routing.properties.deployment
            },
            additionalProperties: false
          },
          invocation: {
            type: 'object',
            required: ['kind', 'targetKind', 'target'],
            properties: {
              kind: { type: 'string', enum: ['spawn'] },
              targetKind: { type: 'string', enum: ['function'] },
              target: { type: 'string', minLength: 1 }
            },
            additionalProperties: false
          },
          deadline: requestStartFrameProperties.deadline,
          trace: requestStartFrameProperties.trace,
          testEffectsEnabled: { type: 'boolean' },
          testCaseCapability: { type: 'string' }
        },
        additionalProperties: false
      }
    ]
  },
  'package-test.start': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'requestId',
      'caller',
      'packageId',
      'packageVersion',
      'testBuildIdentity',
      'entrypointId',
      'activationId',
      'trace'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...packageTestStartFrameProperties
    },
    additionalProperties: false
  },
  'response.chunk': {
    type: 'object',
    required: ['schemaVersion', 'type', 'requestId', 'seq'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['response.chunk'] },
      requestId: { type: 'string' },
      seq: { type: 'integer' }
    },
    additionalProperties: false
  },
  'response.start': {
    type: 'object',
    required: ['schemaVersion', 'type', 'requestId', 'httpResponse'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['response.start'] },
      requestId: { type: 'string' },
      httpResponse: {
        type: 'object',
        required: ['status', 'headers'],
        properties: {
          status: { type: 'integer' },
          headers: {
            type: 'array',
            items: {
              type: 'object',
              required: ['name', 'value'],
              properties: {
                name: { type: 'string' },
                value: { type: 'string' }
              },
              additionalProperties: false
            }
          }
        },
        additionalProperties: false
      }
    },
    additionalProperties: false
  },
  'response.end': {
    oneOf: [
      {
        type: 'object',
        required: ['schemaVersion', 'type', 'requestId', 'payloadPresent'],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['response.end'] },
          requestId: { type: 'string' },
          payloadPresent: { type: 'boolean' }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'payloadPresent',
          'httpResponse'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['response.end'] },
          requestId: { type: 'string' },
          payloadPresent: { type: 'boolean' },
          httpResponse: {
            type: 'object',
            required: ['status', 'headers'],
            properties: {
              status: { type: 'integer', minimum: 100, maximum: 599 },
              headers: {
                type: 'array',
                items: {
                  type: 'object',
                  required: ['name', 'value'],
                  properties: {
                    name: { type: 'string' },
                    value: { type: 'string' }
                  },
                  additionalProperties: false
                }
              }
            },
            additionalProperties: false
          }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'payloadPresent',
          'websocketConnect'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['response.end'] },
          requestId: { type: 'string' },
          payloadPresent: { type: 'boolean', enum: [false] },
          websocketConnect: {
            type: 'object',
            required: ['result'],
            properties: {
              result: { type: 'string', enum: ['accept'] },
              businessIdentity: { type: 'string' },
              connectionPolicy: {
                type: 'object',
                required: ['maxConnections', 'overflow'],
                properties: {
                  maxConnections: {
                    type: 'integer',
                    minimum: 1,
                    maximum: 4_294_967_295
                  },
                  overflow: {
                    type: 'string',
                    enum: ['close-oldest', 'reject-new']
                  },
                  closeCode: { type: 'integer', minimum: 0, maximum: 65_535 },
                  closeReason: { type: 'string' }
                },
                additionalProperties: false
              }
            },
            additionalProperties: false
          }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'payloadPresent',
          'websocketConnect'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['response.end'] },
          requestId: { type: 'string' },
          payloadPresent: { type: 'boolean', enum: [false] },
          websocketConnect: {
            type: 'object',
            required: ['result', 'code', 'reason'],
            properties: {
              result: { type: 'string', enum: ['reject'] },
              code: { type: 'integer', minimum: 0, maximum: 65_535 },
              reason: { type: 'string' }
            },
            additionalProperties: false
          }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'payloadPresent',
          'websocketJsonRpc'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['response.end'] },
          requestId: { type: 'string', minLength: 1 },
          payloadPresent: { type: 'boolean', enum: [true] },
          websocketJsonRpc: {
            type: 'object',
            required: ['outcome'],
            properties: {
              outcome: { type: 'string', enum: ['success'] }
            },
            additionalProperties: false
          }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: [
          'schemaVersion',
          'type',
          'requestId',
          'payloadPresent',
          'websocketJsonRpc'
        ],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['response.end'] },
          requestId: { type: 'string', minLength: 1 },
          payloadPresent: { type: 'boolean', enum: [false] },
          websocketJsonRpc: {
            type: 'object',
            required: ['outcome'],
            properties: {
              outcome: {
                type: 'string',
                enum: ['invalidParams', 'internalError', 'deadlineExceeded']
              }
            },
            additionalProperties: false
          }
        },
        additionalProperties: false
      }
    ]
  },
  'response.error': responseErrorSchema,
  'request.cancel': {
    type: 'object',
    required: ['schemaVersion', 'type', 'requestId', 'reason'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...requestCancelProperties
    },
    additionalProperties: false
  },
  'connection.send': {
    type: 'object',
    required: ['schemaVersion', 'type', 'serviceId'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['connection.send'] },
      serviceId: { type: 'string' },
      websocketEntryId: { type: 'string' },
      businessIdentity: { type: 'string' },
      connectionId: { type: 'string' },
      payloadKind: { type: 'string', enum: ['text', 'binary'] }
    },
    additionalProperties: false
  },
  'connection.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'requestId',
      'serviceId',
      'websocketEntryId',
      'connectionId',
      'profile',
      'method'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['connection.request'] },
      requestId: { type: 'string', minLength: 1 },
      serviceId: { type: 'string', minLength: 1 },
      websocketEntryId: {
        type: 'string',
        pattern: '^skiff-websocket-entry-v1:sha256:[0-9a-f]{64}$'
      },
      connectionId: { type: 'string', minLength: 1 },
      profile: { type: 'string', enum: ['jsonrpc-2.0-text'] },
      method: { type: 'string', minLength: 1 },
      deadline: connectionRequestDeadlineProperty
    },
    additionalProperties: false
  },
  'connection.request.cancel': {
    type: 'object',
    required: ['schemaVersion', 'type', 'requestId', 'reason'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['connection.request.cancel'] },
      requestId: { type: 'string', minLength: 1 },
      reason: { type: 'string', enum: cancelReasons }
    },
    additionalProperties: false
  },
  'connection.response': {
    oneOf: [
      {
        type: 'object',
        required: ['schemaVersion', 'type', 'requestId', 'outcome'],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['connection.response'] },
          requestId: { type: 'string', minLength: 1 },
          outcome: {
            type: 'string',
            enum: [
              'success',
              'deadlineExceeded',
              'connectionUnavailable',
              'transportUnavailable',
              'protocolError',
              'resourceLimit'
            ]
          }
        },
        additionalProperties: false
      },
      {
        type: 'object',
        required: ['schemaVersion', 'type', 'requestId', 'outcome', 'remote'],
        properties: {
          schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
          type: { type: 'string', enum: ['connection.response'] },
          requestId: { type: 'string', minLength: 1 },
          outcome: { type: 'string', enum: ['remote'] },
          remote: connectionRemoteErrorProperty
        },
        additionalProperties: false
      }
    ]
  }
} as const satisfies Record<RuntimeProtocolFrameHeaderName, ProtocolEnvelopeSchema>;

const runtimeRegisterTargetFixture = 'service.example~com~~hello.HelloApi.hello' as const;
const spawnTargetFixture = `function:${runtimeRegisterTargetFixture}` as const;
const serviceProtocolIdentityFixture =
  'skiff-service-protocol-v5:sha256:1111111111111111111111111111111111111111111111111111111111111111' as const;

const runtimeRegisterFixture = {
  type: 'runtime.register',
  runtimeId: 'runtime-fixture-1',
  serviceId: 'example.com/hello',
  revisionId: '1111111111111111111111111111111111111111111111111111111111111111',
  buildId:
    'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
  serviceProtocolIdentity: serviceProtocolIdentityFixture,
  targets: [runtimeRegisterTargetFixture] as string[],
  runtimeVersion: 'fixture-runtime-1',
  codeRevisionId: 'code-fixture-1',
  artifactIdentity: 'artifact-fixture-1',
  gatewayEntryIdentities: [
    'skiff-gateway-v1:sha256:2222222222222222222222222222222222222222222222222222222222222222'
  ] as string[],
  capabilities: {
    dispatchModes: ['unary'],
    packageTestDispatch: true,
    requestCancel: true,
    runtimeProgram: true
  }
} as const;

const runtimeRegisteredFixture = {
  type: 'runtime.registered',
  runtimeId: 'runtime-fixture-1'
} as const;

const runtimeCapabilitiesFixture = {
  type: 'runtime.capabilities',
  runtimeId: 'runtime-fixture-1',
  capabilities: {
    packageTestDispatch: true,
    requestCancel: true
  }
} as const;

const runtimeHealthFixture = {
  type: 'runtime.health',
  runtimeId: 'runtime-fixture-1',
  observedAt: '2026-07-10T00:00:00.000Z',
  counters: {
    outboundRequestsPending: 0,
    outboundStreamLeasesActive: 0,
    streamRuntimeStreamsActive: 0,
    flagBackedCancelWaitersActive: 0,
    spawnedTasksActive: 0
  }
} as const;

const routerControlFixture = {
  type: 'router.control',
  artifactRoots: ['/var/lib/skiff/artifacts'],
  devReload: true,
  generation: 'fixture-generation-1',
  fingerprint: 'sha256:fixture'
} as const;

const requestStartFrameFixture = {
  type: 'request.start',
  requestId: 'request-fixture-1',
  mode: 'unary',
  caller: {
    kind: 'gateway',
    target: 'gateway.example~com~~hello.http.raw'
  },
  target: 'service.example~com~~hello.HelloApi.hello',
  serviceId: 'example.com/hello',
  buildId:
    'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
  serviceProtocolIdentity: serviceProtocolIdentityFixture,
  deadline: {
    timeoutMs: 2000,
    expiresAt: '2026-01-01T00:00:02.000Z'
  },
  trace: {
    traceId: 'trace-fixture-1',
    spanId: 'span-fixture-1',
    sampled: true
  }
} as const;

const packageTestStartFrameFixture = {
  type: 'package-test.start',
  requestId: 'package-test-request-fixture-1',
  caller: {
    kind: 'gateway',
    target: '__skiff.test-dispatch'
  },
  packageId: 'example.com/hello',
  packageVersion: '0.1.0',
  testBuildIdentity:
    'skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  entrypointId:
    'skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
  activationId: 'skiff-package-test-run-v1:example.com~hello:aaaaaaaa:run-fixture:1',
  deadline: {
    timeoutMs: 2000,
    expiresAt: '2026-01-01T00:00:02.000Z'
  },
  trace: {
    traceId: 'trace-package-test-fixture-1',
    spanId: 'span-package-test-fixture-1',
    sampled: true
  }
} as const;

const responseErrorFixture = {
  type: 'response.error',
  requestId: 'request-fixture-1',
  errorKind: 'control',
  error: {
    code: 'FixtureError',
    message: 'fixture runtime error',
    details: {
      retryable: false
    }
  }
} as const;

const requestCancelFixture = {
  type: 'request.cancel',
  requestId: 'request-fixture-1',
  reason: REQUEST_CANCEL_REASON_BY_SITUATION.timeout
} as const;

const connectionSendFixture = {
  type: 'connection.send',
  serviceId: 'example.com/hello',
  websocketEntryId: 'client',
  businessIdentity: 'user-fixture-1'
} as const;

const connectionRequestFixture = {
  type: 'connection.request',
  requestId: 'connection-request-fixture-1',
  serviceId: 'example.com/hello',
  websocketEntryId:
    'skiff-websocket-entry-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  connectionId: 'connection-fixture-1',
  profile: 'jsonrpc-2.0-text',
  method: 'chat.send'
} as const;

const actorKeyFixture = {
  serviceId: 'example.com/hello',
  actorTypeIdentity: 'actor.example.ThreadActor',
  actorIdTypeIdentity: 'type.example.ThreadId',
  actorIdEncodingVersion: 'json-v1',
  canonicalActorIdKeyBytesBase64: 'InRocmVhZC0xIg==',
  actorIdHash:
    'sha256:605d0edc19c41397f6f049dad0d7b3bbcc28a8a7dddbf4ebb8eb9f8b6e766b38'
} as const;

const actorRefFixture = {
  ...actorKeyFixture,
  epoch: 1
} as const;

const actorControlActivationIdentityFixture = {
  assemblyIdentity:
    'skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  generation: 7,
  runtimeReplicaId: 'runtime-replica-7',
  deploymentRevision: 'deployment-revision-7'
} as const;

const spawnFixture = {
  runtimeId: runtimeRegisterFixture.runtimeId,
  workerId: 'spawn-worker-fixture-1',
  serviceId: runtimeRegisterFixture.serviceId,
  serviceVersion: '0.1.0',
  serviceProtocolIdentity: serviceProtocolIdentityFixture,
  buildId:
    'skiff-package-build-v10:sha256:3333333333333333333333333333333333333333333333333333333333333333',
  target: spawnTargetFixture,
  spawnCompatibilityKey: `${'0.1.0'}:${serviceProtocolIdentityFixture}:${spawnTargetFixture}`,
  spawnId: 'spawn-fixture-1',
  itemId: 'spawn-item-fixture-1',
  leaseId: 'spawn-lease-fixture-1',
  spawnExecutionId: 'spawn-exec-fixture-1',
  runtimeRequestId: 'spawn-request-fixture-1'
} as const;

const actorDeclarationOwnerFixture = {
  unit: { kind: 'service' },
  file: { kind: 'fileIrIdentity', value: 'file:actor-fixture' },
  actorSymbol: 'Counter'
} as const;

export const runtimeFrameHeaderFixtures = {
  'runtime.register': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...runtimeRegisterFixture
  },
  'runtime.capabilities': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...runtimeCapabilitiesFixture
  },
  'runtime.health': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...runtimeHealthFixture
  },
  'runtime.registered': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...runtimeRegisteredFixture
  },
  'router.bootstrap': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'router.bootstrap',
    artifactsPath: '/opt/skiff/artifacts',
    serviceDb: {
      mongoUrl: 'mongodb://mongo.internal:27017/skiff?replicaSet=rs0'
    },
    activation: {
      environment: 'prod',
      generation: 7,
      assembly: {
        assemblyIdentity:
          `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`
      },
      configSnapshot: {
        snapshotId:
          'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      }
    },
    http: {
      maxResponseBytes: 67108864
    }
  },
  'router.control': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...routerControlFixture
  },
  'actor.getOrCreate.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.getOrCreate.request',
    rpcId: 'actor-put-rpc-fixture-1',
    runtimeId: runtimeRegisterFixture.runtimeId,
    activationIdentity: actorControlActivationIdentityFixture,
    actorKey: actorKeyFixture,
    actorAbiIdentity: 'skiff-actor-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    actorImplementationIdentity: 'skiff-actor-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    declarationOwner: actorDeclarationOwnerFixture,
    deadline: { timeoutMs: 30_000, expiresAt: '2099-01-01T00:00:00.000Z' }
  },
  'actor.getOrCreate.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.getOrCreate.response',
    rpcId: 'actor-put-rpc-fixture-1',
    actorRef: actorRefFixture
  },
  'actor.getOrCreate.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.getOrCreate.error',
    rpcId: 'actor-put-rpc-fixture-1',
    error: {
      code: 'ActorGetOrCreateFixtureError',
      message: 'fixture actor getOrCreate failed'
    }
  },
  'actor.replace.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.replace.request',
    rpcId: 'actor-replace-rpc-fixture-1',
    runtimeId: runtimeRegisterFixture.runtimeId,
    activationIdentity: actorControlActivationIdentityFixture,
    actorKey: actorKeyFixture,
    actorAbiIdentity: 'skiff-actor-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    actorImplementationIdentity: 'skiff-actor-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
    bootstrapEncodingVersion: 'skiff-canonical-v1',
    declarationOwner: actorDeclarationOwnerFixture,
    deadline: { timeoutMs: 30_000, expiresAt: '2099-01-01T00:00:00.000Z' }
  },
  'actor.replace.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.replace.response',
    rpcId: 'actor-replace-rpc-fixture-1',
    actorRef: actorRefFixture
  },
  'actor.replace.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.replace.error',
    rpcId: 'actor-replace-rpc-fixture-1',
    error: { code: 'ActorReplaceFixtureError', message: 'fixture actor replace failed' }
  },
  'actor.find.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.find.request',
    rpcId: 'actor-find-rpc-fixture-1',
    runtimeId: runtimeRegisterFixture.runtimeId,
    activationIdentity: actorControlActivationIdentityFixture,
    actorKey: actorKeyFixture
  },
  'actor.find.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.find.response',
    rpcId: 'actor-find-rpc-fixture-1',
    found: true,
    actorRef: actorRefFixture
  },
  'actor.find.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.find.error',
    rpcId: 'actor-find-rpc-fixture-1',
    error: {
      code: 'ActorFindFixtureError',
      message: 'fixture actor find failed'
    }
  },
  'actor.remove.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.remove.request',
    rpcId: 'actor-remove-rpc-fixture-1',
    runtimeId: runtimeRegisterFixture.runtimeId,
    activationIdentity: actorControlActivationIdentityFixture,
    actorKey: actorKeyFixture
  },
  'actor.remove.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.remove.response',
    rpcId: 'actor-remove-rpc-fixture-1',
    removed: true
  },
  'actor.remove.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.remove.error',
    rpcId: 'actor-remove-rpc-fixture-1',
    error: {
      code: 'ActorRemoveFixtureError',
      message: 'fixture actor remove failed'
    }
  },
  'spawn.submit.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.request',
    rpcId: 'spawn-submit-rpc-fixture-1',
    runtimeId: spawnFixture.runtimeId,
    activationIdentity: actorControlActivationIdentityFixture,
    targetKind: 'function',
    serviceId: spawnFixture.serviceId,
    serviceVersion: spawnFixture.serviceVersion,
    serviceProtocolIdentity: spawnFixture.serviceProtocolIdentity,
    target: spawnFixture.target,
    spawnId: spawnFixture.spawnId,
    buildId: spawnFixture.buildId,
    callerRequestId: 'caller-request-fixture-1',
    traceId: 'trace-fixture-1',
    callerTarget: runtimeRegisterTargetFixture
  },
  'spawn.submit.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.response',
    rpcId: 'spawn-submit-rpc-fixture-1',
    spawnId: spawnFixture.spawnId,
    requestId: spawnFixture.runtimeRequestId,
    status: 'submitted'
  },
  'spawn.submit.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.error',
    rpcId: 'spawn-submit-rpc-fixture-1',
    error: {
      code: 'SpawnSubmitFixtureError',
      message: 'fixture spawn submit failed'
    }
  },
  'request.start': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: requestStartFrameFixture.requestId,
    mode: requestStartFrameFixture.mode,
    caller: requestStartFrameFixture.caller,
    target: requestStartFrameFixture.target,
    operationAbiId: 'operation:fixture',
    selector: 'operation:operation:fixture',
    serviceId: requestStartFrameFixture.serviceId,
    buildId: requestStartFrameFixture.buildId,
    serviceProtocolIdentity: requestStartFrameFixture.serviceProtocolIdentity,
    deadline: requestStartFrameFixture.deadline,
    trace: requestStartFrameFixture.trace,
    httpRequest: {
      method: 'POST',
      url: 'http://hello.local/hello?name=Ada',
      path: '/hello',
      query: [{ name: 'name', value: 'Ada' }],
      headers: [{ name: 'content-type', value: 'application/octet-stream' }]
    }
  },
  'package-test.start': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...packageTestStartFrameFixture
  },
  'response.chunk': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.chunk',
    requestId: requestStartFrameFixture.requestId,
    seq: 0
  },
  'response.start': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.start',
    requestId: requestStartFrameFixture.requestId,
    httpResponse: {
      status: 200,
      headers: [{ name: 'content-type', value: 'application/octet-stream' }]
    }
  },
  'response.end': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'response.end',
    requestId: requestStartFrameFixture.requestId,
    payloadPresent: true,
    httpResponse: {
      status: 200,
      headers: [{ name: 'content-type', value: 'application/octet-stream' }]
    }
  },
  'response.error': {
    schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
    ...responseErrorFixture
  },
  'request.cancel': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...requestCancelFixture
  },
  'connection.send': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.send',
    serviceId: connectionSendFixture.serviceId,
    websocketEntryId: connectionSendFixture.websocketEntryId,
    businessIdentity: connectionSendFixture.businessIdentity,
    payloadKind: 'binary'
  },
  'connection.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...connectionRequestFixture
  },
  'connection.request.cancel': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.request.cancel',
    requestId: connectionRequestFixture.requestId,
    reason: 'caller_cancel'
  },
  'connection.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'connection.response',
    requestId: connectionRequestFixture.requestId,
    outcome: 'success'
  }
} as const satisfies FrameHeaderFixtureMap;

export function validateRuntimeToRouterFrameHeader(
  value: unknown
): EnvelopeValidationResult<RuntimeToRouterFrameHeader> {
  const typeResult = validateEnvelopeType(value, runtimeToRouterFrameHeaderTypes, 'runtime frame header');
  if (!typeResult.ok) {
    return typeResult;
  }

  const { envelope, type } = typeResult;
  const error =
    validateFrameHeaderBase(envelope, type) ??
    (type === 'runtime.register'
      ? validateRuntimeRegister(envelope)
      : type === 'runtime.capabilities'
        ? validateRuntimeCapabilities(envelope)
      : type === 'runtime.health'
        ? validateRuntimeHealth(envelope)
      : type === 'actor.getOrCreate.request'
        ? validateActorBootstrapRequest(envelope, type)
      : type === 'actor.replace.request'
        ? validateActorBootstrapRequest(envelope, type)
      : type === 'actor.find.request'
        ? validateActorFindRequest(envelope)
      : type === 'actor.remove.request'
        ? validateActorRemoveRequest(envelope)
      : type === 'spawn.submit.request'
        ? validateSpawnSubmitRequest(envelope)
      : type === 'request.start'
        ? validateRequestStartFrameHeader(envelope, false)
      : type === 'request.cancel'
        ? validateRequestCancel(envelope)
        : type === 'connection.send'
          ? validateConnectionSendFrameHeader(envelope)
        : type === 'connection.request'
          ? validateConnectionRequestFrameHeader(envelope)
        : type === 'connection.request.cancel'
          ? validateConnectionRequestCancelFrameHeader(envelope)
          : type === 'response.start'
            ? validateResponseStartFrameHeader(envelope)
          : type === 'response.chunk'
            ? validateResponseChunkFrameHeader(envelope)
            : type === 'response.end'
              ? validateResponseEndFrameHeader(envelope)
              : validateResponseError(envelope));
  if (error) {
    return {
      ok: false,
      error
    };
  }
  return {
    ok: true,
    envelope: envelope as unknown as RuntimeToRouterFrameHeader
  };
}

export function validateRouterToRuntimeFrameHeader(
  value: unknown
): EnvelopeValidationResult<RouterToRuntimeFrameHeader> {
  const typeResult = validateEnvelopeType(value, routerToRuntimeFrameHeaderTypes, 'router frame header');
  if (!typeResult.ok) {
    return typeResult;
  }

  const { envelope, type } = typeResult;
  const error =
    validateFrameHeaderBase(envelope, type) ??
    (type === 'router.bootstrap'
      ? validateRouterBootstrap(envelope)
      : type === 'router.control'
      ? validateRouterControl(envelope)
      : type === 'runtime.registered'
        ? validateRuntimeRegistered(envelope)
      : type === 'actor.getOrCreate.response'
        ? validateActorBootstrapResponse(envelope, type)
      : type === 'actor.getOrCreate.error'
        ? validateRuntimeControlError(envelope, type)
      : type === 'actor.replace.response'
        ? validateActorBootstrapResponse(envelope, type)
      : type === 'actor.replace.error'
        ? validateRuntimeControlError(envelope, type)
      : type === 'actor.find.response'
        ? validateActorFindResponse(envelope)
      : type === 'actor.find.error'
        ? validateRuntimeControlError(envelope, 'actor.find.error')
      : type === 'actor.remove.response'
        ? validateActorRemoveResponse(envelope)
      : type === 'actor.remove.error'
        ? validateRuntimeControlError(envelope, 'actor.remove.error')
      : type === 'spawn.submit.response'
        ? validateSpawnSubmitResponse(envelope)
      : type === 'spawn.submit.error'
        ? validateRuntimeControlError(envelope, 'spawn.submit.error')
      : type === 'request.start'
        ? validateRequestStartFrameHeader(envelope, true)
      : type === 'package-test.start'
        ? validatePackageTestStartFrameHeader(envelope)
      : type === 'request.cancel'
        ? validateRequestCancel(envelope)
      : type === 'connection.response'
        ? validateConnectionResponseFrameHeader(envelope)
      : type === 'response.start'
        ? validateResponseStartFrameHeader(envelope)
      : type === 'response.chunk'
        ? validateResponseChunkFrameHeader(envelope)
      : type === 'response.end'
        ? validateResponseEndFrameHeader(envelope)
        : validateResponseError(envelope));
  if (error) {
    return {
      ok: false,
      error
    };
  }
  return {
    ok: true,
    envelope: envelope as unknown as RouterToRuntimeFrameHeader
  };
}

export function validateResponseErrorFrame(
  header: unknown,
  payloadBytes: unknown
): EnvelopeValidationResult<ValidatedResponseErrorFrame> {
  if (!isRecord(header) || header.type !== 'response.error') {
    return {
      ok: false,
      error: 'invalid response.error frame: header must be a response.error object'
    };
  }
  const headerError =
    validateFrameHeaderBase(header, 'response.error') ?? validateResponseError(header);
  if (headerError) {
    return { ok: false, error: headerError };
  }
  if (!(payloadBytes instanceof Uint8Array)) {
    return {
      ok: false,
      error: 'invalid response.error frame: payload must be a Uint8Array'
    };
  }
  if (header.errorKind === 'control') {
    if (payloadBytes.byteLength !== 0) {
      return {
        ok: false,
        error: 'invalid response.error control frame: payload must be empty'
      };
    }
    return {
      ok: true,
      envelope: {
        header: header as unknown as Extract<ResponseErrorFrameHeader, { errorKind: 'control' }>,
        payloadBytes
      }
    };
  }
  if (payloadBytes.byteLength === 0) {
    return {
      ok: false,
      error: 'invalid response.error fixedService frame: payload must be non-empty'
    };
  }
  const serviceError = decodeServiceErrorEnvelope(payloadBytes);
  if (!serviceError.ok) {
    return serviceError;
  }
  return {
    ok: true,
    envelope: {
      header: header as unknown as Extract<
        ResponseErrorFrameHeader,
        { errorKind: 'fixedService' }
      >,
      payloadBytes,
      serviceError: serviceError.envelope
    }
  };
}

export function validateTelemetryEvent(
  value: unknown
): EnvelopeValidationResult<TelemetryEvent> {
  if (!isRecord(value)) {
    return { ok: false, error: 'invalid telemetry event: event must be an object' };
  }
  const unknown = Object.keys(value).find((field) => !TELEMETRY_EVENT_FIELDS.has(field));
  if (unknown !== undefined) {
    return { ok: false, error: `invalid telemetry event: ${unknown} is not supported` };
  }
  if (
    typeof value.topic !== 'string' ||
    !isAllowedType(value.topic, TELEMETRY_TOPICS)
  ) {
    return { ok: false, error: 'invalid telemetry event: topic is not supported' };
  }
  if (
    typeof value.ts !== 'string' ||
    !RFC3339_TIMESTAMP_PATTERN.test(value.ts) ||
    !Number.isFinite(Date.parse(value.ts))
  ) {
    return { ok: false, error: 'invalid telemetry event: ts must be an RFC3339 timestamp' };
  }
  if (
    typeof value.source !== 'string' ||
    !isAllowedType(value.source, TELEMETRY_EVENT_SOURCES)
  ) {
    return { ok: false, error: 'invalid telemetry event: source is not supported' };
  }
  if (
    typeof value.visibility !== 'string' ||
    !isAllowedType(value.visibility, TELEMETRY_VISIBILITIES)
  ) {
    return { ok: false, error: 'invalid telemetry event: visibility is not supported' };
  }
  for (const field of TELEMETRY_EVENT_STRING_FIELDS) {
    if (value[field] !== undefined && typeof value[field] !== 'string') {
      return { ok: false, error: `invalid telemetry event: ${field} must be a string` };
    }
  }
  if (
    value.errorId !== undefined &&
    (typeof value.errorId !== 'string' || value.errorId.trim().length === 0)
  ) {
    return { ok: false, error: 'invalid telemetry event: errorId must be non-empty' };
  }
  if (value.visibility === 'restricted') {
    if (typeof value.traceId !== 'string' || value.traceId.trim().length === 0) {
      return {
        ok: false,
        error: 'invalid telemetry event: restricted event requires traceId'
      };
    }
    if (typeof value.errorId !== 'string' || value.errorId.trim().length === 0) {
      return {
        ok: false,
        error: 'invalid telemetry event: restricted event requires errorId'
      };
    }
  }
  if (
    value.level !== undefined &&
    (typeof value.level !== 'string' ||
      !isAllowedType(value.level, TELEMETRY_EVENT_LEVELS))
  ) {
    return { ok: false, error: 'invalid telemetry event: level is not supported' };
  }
  for (const field of TELEMETRY_EVENT_OBJECT_FIELDS) {
    if (value[field] !== undefined && !isRecord(value[field])) {
      return { ok: false, error: `invalid telemetry event: ${field} must be an object` };
    }
  }
  if (
    value.durationMs !== undefined &&
    (typeof value.durationMs !== 'number' ||
      !Number.isFinite(value.durationMs) ||
      value.durationMs < 0)
  ) {
    return { ok: false, error: 'invalid telemetry event: durationMs must be non-negative' };
  }
  if (value.topic === 'log' && value.level === undefined) {
    return { ok: false, error: 'invalid telemetry event: log event requires level' };
  }
  if (value.topic === 'trace' && value.name === undefined && value.target === undefined) {
    return { ok: false, error: 'invalid telemetry event: trace event requires name or target' };
  }
  return { ok: true, envelope: value as unknown as TelemetryEvent };
}

export function validateRuntimeAssemblyRequestStartFrameHeader(
  value: unknown
): EnvelopeValidationResult<RuntimeAssemblyRequestStartFrameHeader> {
  const result = validateRuntimeAssemblyRequestStartFrameWireHeader(value);
  if (!result.ok) return result;
  if (
    !('ingress' in result.envelope.routing) ||
    result.envelope.routing.ingress.protocol !== 'http'
  ) {
    return {
      ok: false,
      error:
        'invalid request.start runtimeAssembly envelope: HTTP consumer does not execute websocketConnect'
    };
  }
  return {
    ok: true,
    envelope: result.envelope as RuntimeAssemblyRequestStartFrameHeader
  };
}

export function validateRuntimeAssemblyRequestStartFrameWireHeader(
  value: unknown
): EnvelopeValidationResult<RuntimeAssemblyRequestStartFrameTransportWireHeader> {
  const typeResult = validateEnvelopeType(value, ['request.start'], 'runtimeAssembly frame header');
  if (!typeResult.ok) {
    return typeResult;
  }
  const { envelope } = typeResult;
  if (!hasRuntimeAssemblyRouting(envelope)) {
    return {
      ok: false,
      error: 'invalid request.start runtimeAssembly envelope: routing is required'
    };
  }
  const error =
    validateFrameHeaderBase(envelope, 'request.start') ??
    validateRequestStartFrameHeader(envelope, true);
  return error === null
    ? {
        ok: true,
        envelope: normalizeRuntimeAssemblyRequestStartHeader(envelope)
      }
    : { ok: false, error };
}

export function validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader(
  value: unknown
): EnvelopeValidationResult<RuntimeAssemblyWebSocketConnectResponseEndFrameHeader> {
  const typeResult = validateEnvelopeType(
    value,
    ['response.end'],
    'runtimeAssembly websocketConnect response frame header'
  );
  if (!typeResult.ok) {
    return typeResult;
  }
  const { envelope } = typeResult;
  const error =
    validateFrameHeaderBase(envelope, 'response.end') ??
    validateResponseEndFrameHeader(envelope) ??
    (envelope.websocketConnect === undefined
      ? 'invalid response.end runtimeAssembly envelope: websocketConnect is required'
      : null);
  return error === null
    ? {
        ok: true,
        envelope:
          envelope as unknown as RuntimeAssemblyWebSocketConnectResponseEndFrameHeader
      }
    : { ok: false, error };
}

export function validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader(
  value: unknown
): EnvelopeValidationResult<RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader> {
  const typeResult = validateEnvelopeType(
    value,
    ['response.end'],
    'runtimeAssembly websocketJsonRpc response frame header'
  );
  if (!typeResult.ok) {
    return typeResult;
  }
  const { envelope } = typeResult;
  const error =
    validateFrameHeaderBase(envelope, 'response.end') ??
    validateResponseEndFrameHeader(envelope) ??
    (envelope.websocketJsonRpc === undefined
      ? 'invalid response.end runtimeAssembly envelope: websocketJsonRpc is required'
      : null);
  return error === null
    ? {
        ok: true,
        envelope:
          envelope as unknown as RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader
      }
    : { ok: false, error };
}

function validateEnvelopeType<const TType extends string>(
  value: unknown,
  allowedTypes: readonly TType[],
  side: string
):
  | {
      ok: true;
      envelope: Record<string, unknown>;
      type: TType;
    }
  | {
      ok: false;
      error: string;
    } {
  if (!isRecord(value)) {
    return {
      ok: false,
      error: `invalid ${side} envelope: envelope must be an object`
    };
  }
  if (typeof value.type !== 'string' || !isAllowedType(value.type, allowedTypes)) {
    return {
      ok: false,
      error: `invalid ${side} envelope: type must be one of ${allowedTypes.join(', ')}`
    };
  }
  return {
    ok: true,
    envelope: value,
    type: value.type
  };
}

function validateRuntimeRegister(envelope: Record<string, unknown>): string | null {
  return (
    rejectUnsupportedFrameHeaderFields(envelope, 'runtime.register', [
      'schemaVersion',
      'type',
      'runtimeId',
      'serviceId',
      'version',
      'revisionId',
      'activationIdentity',
      'buildId',
      'serviceProtocolIdentity',
      'targets',
      'runtimeVersion',
      'codeRevisionId',
      'artifactIdentity',
      'gatewayEntryIdentities',
      'capabilities'
    ]) ??
    requireString(envelope, 'runtime.register', 'runtimeId') ??
    requirePublicationId(envelope, 'runtime.register', 'serviceId') ??
    requireStringPattern(
      envelope,
      'runtime.register',
      'revisionId',
      REVISION_ID_PATTERN,
      '<64 lowercase hex>'
    ) ??
    requireStringPattern(
      envelope,
      'runtime.register',
      'buildId',
      BUILD_ID_PATTERN,
      'skiff-service-build-v1:sha256:<64 lowercase hex>'
    ) ??
    requireString(envelope, 'runtime.register', 'serviceProtocolIdentity') ??
    requirePattern(
      envelope,
      'runtime.register',
      'serviceProtocolIdentity',
      SERVICE_PROTOCOL_IDENTITY_PATTERN,
      'skiff-service-protocol-v5:sha256:<64 lowercase hex>'
    ) ??
    requireNonEmptyStringArray(envelope, 'runtime.register', 'targets') ??
    optionalString(envelope, 'runtime.register', 'runtimeVersion') ??
    optionalString(envelope, 'runtime.register', 'codeRevisionId') ??
    optionalString(envelope, 'runtime.register', 'artifactIdentity') ??
    optionalStringPattern(
      envelope,
      'runtime.register',
      'activationIdentity',
      ACTIVATION_IDENTITY_PATTERN,
      'skiff-runtime-activation-v1:opaque:<opaque id>'
    ) ??
    validateRuntimeRegisterTargets(envelope) ??
    optionalStringArray(envelope, 'runtime.register', 'gatewayEntryIdentities') ??
    optionalStringArrayPattern(
      envelope,
      'runtime.register',
      'gatewayEntryIdentities',
      GATEWAY_IDENTITY_PATTERN,
      'skiff-gateway-v1:sha256:<64 lowercase hex>'
    ) ??
    validateRuntimeCapabilitiesMetadata(envelope.capabilities, 'runtime.register', 'capabilities')
  );
}

function validateRuntimeCapabilities(envelope: Record<string, unknown>): string | null {
  return (
    rejectUnsupportedFrameHeaderFields(envelope, 'runtime.capabilities', [
      'schemaVersion',
      'type',
      'runtimeId',
      'capabilities'
    ]) ??
    requireString(envelope, 'runtime.capabilities', 'runtimeId') ??
    validateRuntimeCapabilitiesMetadata(envelope.capabilities, 'runtime.capabilities', 'capabilities', true)
  );
}

function validateRuntimeHealth(envelope: Record<string, unknown>): string | null {
  const counterFields = Object.keys(runtimeHealthCountersProperties);
  return (
    rejectUnsupportedFrameHeaderFields(envelope, 'runtime.health', [
      'schemaVersion',
      'type',
      'runtimeId',
      'observedAt',
      'counters'
    ]) ??
    requireString(envelope, 'runtime.health', 'runtimeId') ??
    requireString(envelope, 'runtime.health', 'observedAt') ??
    requireObject(envelope, 'runtime.health', 'counters') ??
    validateRuntimeHealthCounters(envelope.counters, counterFields)
  );
}

function validateRuntimeHealthCounters(
  counters: unknown,
  counterFields: readonly string[]
): string | null {
  if (!isRecord(counters)) {
    return 'invalid runtime.health envelope: counters must be an object';
  }
  const unsupported = Object.keys(counters).find((key) => !counterFields.includes(key));
  if (unsupported !== undefined) {
    return `invalid runtime.health envelope: counters.${unsupported} is not supported`;
  }
  for (const field of counterFields) {
    const value = counters[field];
    if (!Number.isInteger(value) || Number(value) < 0) {
      return `invalid runtime.health envelope: counters.${field} must be a non-negative integer`;
    }
  }
  return null;
}

function validateRuntimeCapabilitiesMetadata(
  value: unknown,
  envelopeType: string,
  field: string,
  required = false
): string | null {
  if (value === undefined) {
    return required ? `invalid ${envelopeType} envelope: ${field} must be an object` : null;
  }
  if (!isRecord(value)) {
    return `invalid ${envelopeType} envelope: ${field} must be an object`;
  }
  const supported = ['dispatchModes', 'packageTestDispatch', 'requestCancel', 'runtimeProgram'];
  const unsupported = Object.keys(value).find((key) => !supported.includes(key));
  if (unsupported !== undefined) {
    return `invalid ${envelopeType} envelope: ${field}.${unsupported} is not supported`;
  }
  if (value.dispatchModes !== undefined) {
    if (!Array.isArray(value.dispatchModes)) {
      return `invalid ${envelopeType} envelope: ${field}.dispatchModes must be an array`;
    }
    for (const item of value.dispatchModes) {
      if (typeof item !== 'string' || !dispatchModes.includes(item as (typeof dispatchModes)[number])) {
        return `invalid ${envelopeType} envelope: ${field}.dispatchModes items must be one of ${dispatchModes.join(', ')}`;
      }
    }
  }
  for (const booleanField of ['packageTestDispatch', 'requestCancel', 'runtimeProgram']) {
    if (value[booleanField] !== undefined && typeof value[booleanField] !== 'boolean') {
      return `invalid ${envelopeType} envelope: ${field}.${booleanField} must be a boolean`;
    }
  }
  return null;
}

function validateRuntimeRegisterTargets(envelope: Record<string, unknown>): string | null {
  const serviceId = getPathValue(envelope, 'serviceId');
  const targets = getPathValue(envelope, 'targets');
  if (typeof serviceId !== 'string' || !isPublicationId(serviceId) || !Array.isArray(targets)) {
    return null;
  }

  const expectedServiceComponent = publicationStorageSegment(serviceId);
  for (const target of targets) {
    if (typeof target !== 'string') {
      continue;
    }
    if (!target.startsWith('service.') && !target.startsWith('gateway.')) {
      continue;
    }

    const [namespace, serviceComponent, ...suffix] = target.split('.');
    const expectedPrefix = `${namespace}.${expectedServiceComponent}`;
    if (
      serviceComponent !== expectedServiceComponent ||
      serviceComponent.includes('/') ||
      suffix.length === 0 ||
      suffix.some((component) => component.length === 0 || component.includes('/'))
    ) {
      return `invalid runtime.register envelope: targets items must use ${expectedPrefix}.<target suffix>`;
    }
  }

  return null;
}

function validateRuntimeRegistered(envelope: Record<string, unknown>): string | null {
  return requireString(envelope, 'runtime.registered', 'runtimeId');
}

function validateRuntimeRpcBase(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  return (
    rejectHeaderPayloadFields(envelope, envelopeType) ??
    requireString(envelope, envelopeType, 'rpcId')
  );
}

function protocolEnvelopeSchemaPropertyNames(
  schema: ProtocolEnvelopeSchema
): string[] {
  const branches = 'oneOf' in schema ? schema.oneOf : [schema];
  return [...new Set(branches.flatMap((branch) => Object.keys(branch.properties)))];
}

function validateRuntimeRpcRequestBase(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  const schema =
    runtimeFrameHeaderSchemas[envelopeType as RuntimeProtocolFrameHeaderName];
  return (
    rejectUnsupportedFrameHeaderFields(
      envelope,
      envelopeType,
      protocolEnvelopeSchemaPropertyNames(schema)
    ) ??
    validateRuntimeRpcBase(envelope, envelopeType) ??
    requireString(envelope, envelopeType, 'runtimeId') ??
    validateControlActivationIdentity(
      envelope,
      envelopeType,
      'activationIdentity'
    )
  );
}

function validateControlActivationIdentity(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  if (!isRecord(value)) {
    return `invalid ${envelopeType} envelope: ${field} must be an object`;
  }
  const unknownField = rejectUnsupportedObjectFields(
    value,
    envelopeType,
    field,
    [
      'assemblyIdentity',
      'generation',
      'runtimeReplicaId',
      'deploymentRevision'
    ]
  );
  if (unknownField !== null) {
    return unknownField;
  }
  try {
    runtimeAssemblyIdentity(value.assemblyIdentity);
    activationGeneration(value.generation, `${field}.generation`);
    activationToken(value.runtimeReplicaId, `${field}.runtimeReplicaId`);
    activationToken(value.deploymentRevision, `${field}.deploymentRevision`);
    return null;
  } catch (error) {
    return `invalid ${envelopeType} envelope: ${
      error instanceof Error ? error.message : String(error)
    }`;
  }
}

function validateActorBootstrapRequest(
  envelope: Record<string, unknown>,
  type: 'actor.getOrCreate.request' | 'actor.replace.request'
): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, type) ??
    validateActorKey(envelope, type, 'actorKey', false) ??
    requireString(envelope, type, 'actorAbiIdentity') ??
    requireString(envelope, type, 'actorImplementationIdentity') ??
    requireString(envelope, type, 'bootstrapEncodingVersion') ??
    validateActorDeclarationOwner(envelope, type) ??
    validateDeadline(envelope, type)
  );
}

function validateActorDeclarationOwner(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  const owner = getPathValue(envelope, 'declarationOwner');
  if (!isRecord(owner)) {
    return `invalid ${envelopeType} envelope: declarationOwner must be an object`;
  }
  return (
    rejectUnsupportedObjectFields(owner, envelopeType, 'declarationOwner', [
      'unit',
      'file',
      'actorSymbol',
    ]) ??
    requireString(envelope, envelopeType, 'declarationOwner.actorSymbol') ??
    validateActorDeclarationUnit(owner, envelopeType) ??
    validateActorDeclarationFile(owner, envelopeType)
  );
}

function validateActorDeclarationUnit(
  owner: Record<string, unknown>,
  envelopeType: string
): string | null {
  const unit = owner.unit;
  if (!isRecord(unit)) {
    return `invalid ${envelopeType} envelope: declarationOwner.unit must be an object`;
  }
  return (
    rejectUnsupportedObjectFields(unit, envelopeType, 'declarationOwner.unit', [
      'kind',
      'value',
    ]) ??
    requireEnum(owner, envelopeType, 'unit.kind', ['service', 'package'])
  );
}

function validateActorDeclarationFile(
  owner: Record<string, unknown>,
  envelopeType: string
): string | null {
  const file = owner.file;
  if (!isRecord(file)) {
    return `invalid ${envelopeType} envelope: declarationOwner.file must be an object`;
  }
  return (
    rejectUnsupportedObjectFields(file, envelopeType, 'declarationOwner.file', [
      'kind',
      'value',
    ]) ??
    requireEnum(owner, envelopeType, 'file.kind', [
      'loadedFileIndex',
      'fileIrIdentity',
    ])
  );
}

function validateActorBootstrapResponse(
  envelope: Record<string, unknown>,
  type: 'actor.getOrCreate.response' | 'actor.replace.response'
): string | null {
  return (
    validateRuntimeRpcBase(envelope, type) ??
    validateActorKey(envelope, type, 'actorRef', true) ??
    optionalPositiveInteger(envelope, type, 'actorRef.epoch')
  );
}

function validateActorFindRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'actor.find.request') ??
    validateActorKey(envelope, 'actor.find.request', 'actorKey', false)
  );
}

function validateActorFindResponse(envelope: Record<string, unknown>): string | null {
  const baseError =
    validateRuntimeRpcBase(envelope, 'actor.find.response') ??
    requireBoolean(envelope, 'actor.find.response', 'found');
  if (baseError) {
    return baseError;
  }
  if (envelope.actorRef === undefined) {
    return envelope.found === true
      ? 'invalid actor.find.response envelope: actorRef must be an object when found is true'
      : null;
  }
  return (
    validateActorKey(envelope, 'actor.find.response', 'actorRef', true) ??
    optionalPositiveInteger(envelope, 'actor.find.response', 'actorRef.epoch')
  );
}

function validateActorRemoveRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'actor.remove.request') ??
    validateActorKey(envelope, 'actor.remove.request', 'actorKey', false)
  );
}

function validateActorRemoveResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'actor.remove.response') ??
    requireBoolean(envelope, 'actor.remove.response', 'removed')
  );
}

function validateSpawnSubmitRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'spawn.submit.request') ??
    requireEnum(envelope, 'spawn.submit.request', 'targetKind', spawnTargetKinds) ??
    requirePublicationId(envelope, 'spawn.submit.request', 'serviceId') ??
    requireString(envelope, 'spawn.submit.request', 'serviceVersion') ??
    requireStringPattern(
      envelope,
      'spawn.submit.request',
      'serviceProtocolIdentity',
      SERVICE_PROTOCOL_IDENTITY_PATTERN,
      'skiff-service-protocol-v5:sha256:<64 lowercase hex>'
    ) ??
    requireString(envelope, 'spawn.submit.request', 'target') ??
    forbiddenField(envelope, 'spawn.submit.request', 'actorRef') ??
    forbiddenField(envelope, 'spawn.submit.request', 'methodName') ??
    optionalString(envelope, 'spawn.submit.request', 'spawnId') ??
    optionalStringPattern(
      envelope,
      'spawn.submit.request',
      'buildId',
      PACKAGE_BUILD_ID_PATTERN,
      'skiff-package-build-v10:sha256:<64 lowercase hex>'
    ) ??
    requireString(envelope, 'spawn.submit.request', 'callerRequestId') ??
    optionalString(envelope, 'spawn.submit.request', 'traceId') ??
    optionalString(envelope, 'spawn.submit.request', 'callerTarget')
  );
}

function validateSpawnSubmitResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'spawn.submit.response') ??
    requireString(envelope, 'spawn.submit.response', 'spawnId') ??
    requireString(envelope, 'spawn.submit.response', 'requestId') ??
    requireEnum(envelope, 'spawn.submit.response', 'status', ['submitted'])
  );
}

function validateRuntimeControlError(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  return validateRuntimeRpcBase(envelope, envelopeType) ?? validateErrorPayload(envelope, envelopeType);
}

function validateActorKey(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  requireHash: boolean
): string | null {
  const value = getPathValue(envelope, field);
  if (!isRecord(value)) {
    return `invalid ${envelopeType} envelope: ${field} must be an object`;
  }
  return (
    requirePublicationId(envelope, envelopeType, `${field}.serviceId`) ??
    requireString(envelope, envelopeType, `${field}.actorTypeIdentity`) ??
    requireString(envelope, envelopeType, `${field}.actorIdTypeIdentity`) ??
    requireString(envelope, envelopeType, `${field}.actorIdEncodingVersion`) ??
    validateBase64String(envelope, envelopeType, `${field}.canonicalActorIdKeyBytesBase64`) ??
    (requireHash
      ? requireStringPattern(
          envelope,
          envelopeType,
          `${field}.actorIdHash`,
          ACTOR_ID_HASH_PATTERN,
          'sha256:<64 lowercase hex>'
        )
      : optionalStringPattern(
          envelope,
          envelopeType,
          `${field}.actorIdHash`,
          ACTOR_ID_HASH_PATTERN,
          'sha256:<64 lowercase hex>'
        ))
  );
}

function optionalActorRef(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  if (getPathValue(envelope, field) === undefined) {
    return null;
  }
  return (
    validateActorKey(envelope, envelopeType, field, true) ??
    optionalPositiveInteger(envelope, envelopeType, `${field}.epoch`)
  );
}

function validateRouterControl(envelope: Record<string, unknown>): string | null {
  return (
    rejectRouterControlLegacyArtifactRoot(envelope) ??
    validateRouterControlArtifactRoots(envelope) ??
    optionalBoolean(envelope, 'router.control', 'devReload') ??
    optionalString(envelope, 'router.control', 'mode') ??
    optionalString(envelope, 'router.control', 'generation') ??
    optionalString(envelope, 'router.control', 'fingerprint') ??
    validateTelemetryControl(envelope) ??
    validateFileBackendControl(envelope) ??
    validateServiceConfig(envelope)
  );
}

function validateRouterBootstrap(envelope: Record<string, unknown>): string | null {
  const fieldsError = rejectUnsupportedFrameHeaderFields(envelope, 'router.bootstrap', [
    'schemaVersion',
    'type',
    'artifactsPath',
    'serviceDb',
    'http',
    'activation'
  ]);
  if (fieldsError !== null) {
    return fieldsError;
  }
  if (!isNormalizedAbsoluteArtifactsPath(envelope.artifactsPath)) {
    return 'invalid router.bootstrap envelope: artifactsPath must be an absolute normalized path';
  }
  if (!isRecord(envelope.serviceDb)) {
    return 'invalid router.bootstrap envelope: serviceDb must be an object';
  }
  const serviceDbFieldsError = rejectUnsupportedObjectFields(
    envelope.serviceDb,
    'router.bootstrap',
    'serviceDb',
    ['mongoUrl']
  );
  if (serviceDbFieldsError !== null) {
    return serviceDbFieldsError;
  }
  const serviceDbError = typeof envelope.serviceDb.mongoUrl === 'string' &&
    envelope.serviceDb.mongoUrl.trim().length > 0
    ? null
    : 'invalid router.bootstrap envelope: serviceDb.mongoUrl must be a non-empty string';
  if (serviceDbError !== null) {
    return serviceDbError;
  }
  if (!isRecord(envelope.activation)) {
    return 'invalid router.bootstrap envelope: activation must be an object';
  }
  const activationFieldsError = rejectUnsupportedObjectFields(
    envelope.activation,
    'router.bootstrap',
    'activation',
    ['environment', 'generation', 'assembly', 'configSnapshot']
  );
  if (activationFieldsError !== null) {
    return activationFieldsError;
  }
  if (
    typeof envelope.activation.environment !== 'string' ||
    envelope.activation.environment.length === 0 ||
    envelope.activation.environment.length > 200 ||
    envelope.activation.environment === '.' ||
    envelope.activation.environment === '..' ||
    !/^[A-Za-z0-9._-]+$/.test(envelope.activation.environment)
  ) {
    return 'invalid router.bootstrap envelope: activation.environment must be 1-200 ASCII letters, digits, dot, dash, or underscore and must not be . or ..';
  }
  if (
    typeof envelope.activation.generation !== 'number' ||
    !Number.isSafeInteger(envelope.activation.generation) ||
    envelope.activation.generation < 0
  ) {
    return 'invalid router.bootstrap envelope: activation.generation must be a non-negative safe integer';
  }
  if (!isRecord(envelope.activation.assembly)) {
    return 'invalid router.bootstrap envelope: activation.assembly must be an object';
  }
  const activationAssemblyFieldsError = rejectUnsupportedObjectFields(
    envelope.activation.assembly,
    'router.bootstrap',
    'activation.assembly',
    ['assemblyIdentity']
  );
  if (activationAssemblyFieldsError !== null) {
    return activationAssemblyFieldsError;
  }
  if (
    typeof envelope.activation.assembly.assemblyIdentity !== 'string' ||
    !/^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$/.test(
      envelope.activation.assembly.assemblyIdentity
    )
  ) {
    return 'invalid router.bootstrap envelope: activation.assembly.assemblyIdentity must be a canonical RuntimeAssembly identity';
  }
  if (!isRecord(envelope.activation.configSnapshot)) {
    return 'invalid router.bootstrap envelope: activation.configSnapshot must be an object';
  }
  const activationConfigSnapshotFieldsError = rejectUnsupportedObjectFields(
    envelope.activation.configSnapshot,
    'router.bootstrap',
    'activation.configSnapshot',
    ['snapshotId']
  );
  if (activationConfigSnapshotFieldsError !== null) {
    return activationConfigSnapshotFieldsError;
  }
  if (
    typeof envelope.activation.configSnapshot.snapshotId !== 'string' ||
    !/^skiff-runtime-config-snapshot-v1:[0-9a-f]{32}$/.test(
      envelope.activation.configSnapshot.snapshotId
    )
  ) {
    return 'invalid router.bootstrap envelope: activation.configSnapshot.snapshotId must be a canonical opaque RuntimeConfigSnapshot identity';
  }
  if (!isRecord(envelope.http)) {
    return 'invalid router.bootstrap envelope: http must be an object';
  }
  const httpFieldsError = rejectUnsupportedObjectFields(
    envelope.http,
    'router.bootstrap',
    'http',
    ['maxResponseBytes']
  );
  if (httpFieldsError !== null) {
    return httpFieldsError;
  }
  return typeof envelope.http.maxResponseBytes === 'number' &&
    Number.isSafeInteger(envelope.http.maxResponseBytes) &&
    envelope.http.maxResponseBytes > 0
    ? null
    : 'invalid router.bootstrap envelope: http.maxResponseBytes must be a positive safe integer';
}

function isNormalizedAbsoluteArtifactsPath(value: unknown): value is string {
  if (typeof value !== 'string' || !value.startsWith('/')) {
    return false;
  }
  if (value !== '/' && value.endsWith('/')) {
    return false;
  }
  if (value === '/') {
    return true;
  }
  const components = value.split('/').slice(1);
  return components.every(
    (component) => component.length > 0 && component !== '.' && component !== '..'
  );
}

function rejectRouterControlLegacyArtifactRoot(envelope: Record<string, unknown>): string | null {
  return Object.prototype.hasOwnProperty.call(envelope, 'artifactRoot')
    ? 'invalid router.control frame header: artifactRoot is not supported; use artifactRoots'
    : null;
}

function validateRouterControlArtifactRoots(envelope: Record<string, unknown>): string | null {
  const value = envelope.artifactRoots;
  if (!Array.isArray(value) || value.length === 0) {
    return 'invalid router.control envelope: artifactRoots must be a non-empty string array';
  }
  for (let index = 0; index < value.length; index += 1) {
    if (typeof value[index] !== 'string' || value[index].length === 0) {
      return `invalid router.control envelope: artifactRoots[${index}] must be a non-empty string`;
    }
  }
  return null;
}

function validateTelemetryControl(envelope: Record<string, unknown>): string | null {
  if (envelope.telemetry === undefined) {
    return null;
  }
  if (!isRecord(envelope.telemetry)) {
    return 'invalid router.control envelope: telemetry must be an object';
  }
  const telemetry = envelope.telemetry;
  return (
    requireTelemetryString(telemetry, 'endpoint') ??
    requireTelemetryEnum(telemetry, 'protocol', [TELEMETRY_PROTOCOL]) ??
    validateTelemetryTopics(telemetry) ??
    requireTelemetryPositiveInteger(telemetry, 'queueMaxEvents') ??
    requireTelemetryPositiveInteger(telemetry, 'batchMaxEvents') ??
    requireTelemetryPositiveInteger(telemetry, 'batchMaxBytes') ??
    requireTelemetryPositiveInteger(telemetry, 'flushIntervalMs') ??
    requireTelemetryBoolean(telemetry, 'enabled')
  );
}

function validateTelemetryTopics(telemetry: Record<string, unknown>): string | null {
  const value = telemetry.topics;
  if (!Array.isArray(value) || value.length === 0) {
    return 'invalid router.control envelope: telemetry.topics must be a non-empty array';
  }
  const seen = new Set<TelemetryTopic>();
  for (const topic of value) {
    if (typeof topic !== 'string' || !isAllowedType(topic, TELEMETRY_TOPICS)) {
      return `invalid router.control envelope: telemetry.topics items must be one of ${TELEMETRY_TOPICS.join(', ')}`;
    }
    if (seen.has(topic)) {
      return 'invalid router.control envelope: telemetry.topics must not contain duplicates';
    }
    seen.add(topic);
  }
  return null;
}

function requireTelemetryString(
  telemetry: Record<string, unknown>,
  field: string
): string | null {
  return typeof telemetry[field] === 'string'
    ? null
    : `invalid router.control envelope: telemetry.${field} must be a string`;
}

function requireTelemetryBoolean(
  telemetry: Record<string, unknown>,
  field: string
): string | null {
  return typeof telemetry[field] === 'boolean'
    ? null
    : `invalid router.control envelope: telemetry.${field} must be a boolean`;
}

function requireTelemetryPositiveInteger(
  telemetry: Record<string, unknown>,
  field: string
): string | null {
  return Number.isInteger(telemetry[field]) && Number(telemetry[field]) > 0
    ? null
    : `invalid router.control envelope: telemetry.${field} must be a positive integer`;
}

function requireTelemetryEnum<const TValue extends string>(
  telemetry: Record<string, unknown>,
  field: string,
  allowedValues: readonly TValue[]
): string | null {
  const value = telemetry[field];
  return typeof value === 'string' && isAllowedType(value, allowedValues)
    ? null
    : `invalid router.control envelope: telemetry.${field} must be one of ${allowedValues.join(', ')}`;
}

function validateFileBackendControl(envelope: Record<string, unknown>): string | null {
  if (envelope.fileBackend === undefined) {
    return null;
  }
  if (!isRecord(envelope.fileBackend)) {
    return 'invalid router.control envelope: fileBackend must be an object';
  }
  const fileBackend = envelope.fileBackend;
  if (fileBackend.local === undefined && fileBackend.oss === undefined) {
    return 'invalid router.control envelope: fileBackend must configure local or oss';
  }
  return validateFileBackendLocal(fileBackend) ?? validateFileBackendOss(fileBackend);
}

function validateFileBackendLocal(fileBackend: Record<string, unknown>): string | null {
  if (fileBackend.local === undefined) {
    return null;
  }
  if (!isRecord(fileBackend.local)) {
    return 'invalid router.control envelope: fileBackend.local must be an object';
  }
  return requireFileBackendString(fileBackend.local, 'local.root');
}

function validateFileBackendOss(fileBackend: Record<string, unknown>): string | null {
  if (fileBackend.oss === undefined) {
    return null;
  }
  if (!isRecord(fileBackend.oss)) {
    return 'invalid router.control envelope: fileBackend.oss must be an object';
  }
  const oss = fileBackend.oss;
  return (
    requireFileBackendString(oss, 'oss.endpoint') ??
    requireFileBackendString(oss, 'oss.bucket') ??
    optionalFileBackendString(oss, 'oss.region') ??
    optionalFileBackendString(oss, 'oss.accessKeyId') ??
    optionalFileBackendString(oss, 'oss.accessKeySecret') ??
    optionalFileBackendString(oss, 'oss.accessKeyIdEnv') ??
    optionalFileBackendString(oss, 'oss.accessKeySecretEnv') ??
    validateFileBackendOssCredentials(oss)
  );
}

function validateFileBackendOssCredentials(oss: Record<string, unknown>): string | null {
  if (oss.accessKeyId === undefined && oss.accessKeyIdEnv === undefined) {
    return 'invalid router.control envelope: fileBackend.oss requires accessKeyIdEnv or accessKeyId';
  }
  if (oss.accessKeySecret === undefined && oss.accessKeySecretEnv === undefined) {
    return 'invalid router.control envelope: fileBackend.oss requires accessKeySecretEnv or accessKeySecret';
  }
  return null;
}

function requireFileBackendString(
  fileBackend: Record<string, unknown>,
  field: string
): string | null {
  const key = fieldLeaf(field);
  return typeof fileBackend[key] === 'string' && fileBackend[key].length > 0
    ? null
    : `invalid router.control envelope: fileBackend.${field} must be a non-empty string`;
}

function optionalFileBackendString(
  fileBackend: Record<string, unknown>,
  field: string
): string | null {
  const key = fieldLeaf(field);
  return fileBackend[key] === undefined ||
    (typeof fileBackend[key] === 'string' && fileBackend[key].length > 0)
    ? null
    : `invalid router.control envelope: fileBackend.${field} must be a non-empty string`;
}

function fieldLeaf(field: string): string {
  const dot = field.lastIndexOf('.');
  return dot === -1 ? field : field.slice(dot + 1);
}

function validateRequestStartFrameHeader(
  envelope: Record<string, unknown>,
  allowRuntimeAssemblyRouting: boolean
): string | null {
  if (hasRuntimeAssemblyRouting(envelope)) {
    return (
      rejectHeaderPayloadFields(envelope, 'request.start') ??
      validateRequestRoutingVariant(envelope, allowRuntimeAssemblyRouting)
    );
  }
  const baseError =
    rejectHeaderPayloadFields(envelope, 'request.start') ??
    requireString(envelope, 'request.start', 'requestId') ??
    requireEnum(envelope, 'request.start', 'mode', ['unary', 'serverStream']) ??
    validateCaller(envelope);
  if (baseError !== null) return baseError;

  return (
    optionalStringPattern(
      envelope,
      'request.start',
      'activationIdentity',
      ACTIVATION_IDENTITY_PATTERN,
      'skiff-runtime-activation-v1:opaque:<opaque id>'
    ) ??
    optionalString(envelope, 'request.start', 'gatewayEntryIdentity') ??
    optionalStringPattern(
      envelope,
      'request.start',
      'gatewayEntryIdentity',
      GATEWAY_IDENTITY_PATTERN,
      'skiff-gateway-v1:sha256:<64 lowercase hex>'
    ) ??
    forbiddenField(envelope, 'request.start', 'identity') ??
    optionalString(envelope, 'request.start', 'businessIdentity') ??
    optionalString(envelope, 'request.start', 'websocketEntryId') ??
    validateRuntimeClientSession(envelope.clientSession) ??
    validateDeadline(envelope) ??
    validateTrace(envelope) ??
    validateHttpRequestFrameMetadata(envelope) ??
    validateHttpAdapterFrameMetadata(envelope) ??
    validateWebSocketAdapterFrameMetadata(envelope) ??
    optionalBoolean(envelope, 'request.start', 'testEffectsEnabled') ??
    validateTestEffectDoubles(envelope, 'request.start') ??
    validateRequestRoutingVariant(envelope, allowRuntimeAssemblyRouting)
  );
}

function validateRequestRoutingVariant(
  envelope: Record<string, unknown>,
  allowRuntimeAssemblyRouting: boolean
): string | null {
  if (hasRuntimeAssemblyRouting(envelope)) {
    if (!allowRuntimeAssemblyRouting) {
      return 'invalid runtime-to-router request.start envelope: runtimeAssembly routing is not supported';
    }
    return validateRuntimeAssemblyRequestStartHeader(envelope);
  }
  return validateLegacyRequestRouting(envelope);
}

function validateLegacyRequestRouting(envelope: Record<string, unknown>): string | null {
  return (
    rejectUnsupportedLegacyAssemblyRouting(envelope) ??
    requireString(envelope, 'request.start', 'target') ??
    requireString(envelope, 'request.start', 'operationAbiId') ??
    optionalString(envelope, 'request.start', 'selector') ??
    optionalPublicationId(envelope, 'request.start', 'serviceId') ??
    requireStringPattern(
      envelope,
      'request.start',
      'buildId',
      BUILD_ID_PATTERN,
      'skiff-service-build-v1:sha256:<64 lowercase hex>'
    ) ??
    requireString(envelope, 'request.start', 'serviceProtocolIdentity') ??
    validateRequestProtocolIdentity(envelope)
  );
}

function rejectUnsupportedLegacyAssemblyRouting(
  envelope: Record<string, unknown>
): string | null {
  for (const field of [
    'assemblyIdentity',
    'assemblyGeneration',
    'contractOperationId',
    'ingress'
  ]) {
    if (Object.prototype.hasOwnProperty.call(envelope, field)) {
      return `invalid request.start legacy envelope: ${field} is not supported; use routing`;
    }
  }
  return null;
}

function validateRequestProtocolIdentity(envelope: Record<string, unknown>): string | null {
  return requirePattern(
    envelope,
    'request.start',
    'serviceProtocolIdentity',
    SERVICE_PROTOCOL_IDENTITY_PATTERN,
    'skiff-service-protocol-v5:sha256:<64 lowercase hex>'
  );
}

function validatePackageTestStartFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'package-test.start') ??
    requireString(envelope, 'package-test.start', 'requestId') ??
    validatePackageTestCaller(envelope) ??
    requirePublicationId(envelope, 'package-test.start', 'packageId') ??
    requireString(envelope, 'package-test.start', 'packageVersion') ??
    requireStringPattern(
      envelope,
      'package-test.start',
      'testBuildIdentity',
      PACKAGE_TEST_BUILD_ID_PATTERN,
      'skiff-package-test-build-v1:sha256:<64 lowercase hex>'
    ) ??
    requireStringPattern(
      envelope,
      'package-test.start',
      'entrypointId',
      PACKAGE_TEST_ENTRYPOINT_ID_PATTERN,
      'skiff-package-test-entrypoint-v1:sha256:<64 lowercase hex>'
    ) ??
    requireStringPattern(
      envelope,
      'package-test.start',
      'activationId',
      PACKAGE_TEST_ACTIVATION_ID_PATTERN,
      'skiff-package-test-run-v1:<opaque id>'
    ) ??
    validateDeadline(envelope, 'package-test.start') ??
    validateTrace(envelope, 'package-test.start') ??
    optionalBoolean(envelope, 'package-test.start', 'testEffectsEnabled') ??
    validateTestEffectDoubles(envelope, 'package-test.start')
  );
}

function validateTestEffectDoubles(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  if (envelope.testEffectDoubles === undefined) {
    return null;
  }
  if (!isRecord(envelope.testEffectDoubles)) {
    return `invalid ${envelopeType} envelope: testEffectDoubles must be an object`;
  }
  for (const [target, sequence] of Object.entries(envelope.testEffectDoubles)) {
    if (!Array.isArray(sequence) || sequence.length === 0) {
      return `invalid ${envelopeType} envelope: testEffectDoubles.${target} must be a non-empty array`;
    }
    for (const [index, step] of sequence.entries()) {
      if (!isRecord(step)) {
        return `invalid ${envelopeType} envelope: testEffectDoubles.${target}[${index}] must be an object`;
      }
      if (!Object.prototype.hasOwnProperty.call(step, 'response')) {
        return `invalid ${envelopeType} envelope: testEffectDoubles.${target}[${index}].response is required`;
      }
      const unsupported = Object.keys(step).filter(
        (key) => key !== 'expectRequest' && key !== 'response'
      );
      if (unsupported.length > 0) {
        return `invalid ${envelopeType} envelope: testEffectDoubles.${target}[${index}] does not support ${unsupported.join(', ')}`;
      }
    }
  }
  return null;
}

function validateServiceConfig(envelope: Record<string, unknown>): string | null {
  if (envelope.serviceValues !== undefined) {
    return 'invalid router.control envelope: serviceValues is no longer supported; use serviceConfig';
  }
  if (envelope.serviceEnv !== undefined) {
    return 'invalid router.control envelope: serviceEnv is no longer supported; use serviceConfig';
  }
  const value = envelope.serviceConfig;
  if (value === undefined) {
    return null;
  }
  if (!Array.isArray(value)) {
    return 'invalid router.control envelope: serviceConfig must be an array';
  }
  for (let index = 0; index < value.length; index += 1) {
    const item = value[index];
    const label = `serviceConfig[${index}]`;
    if (!isRecord(item)) {
      return `invalid router.control envelope: ${label} must be an object`;
    }
    for (const field of [
      'valuesSnapshotIdentity',
      'valuesSnapshot',
      'redactedValuesSnapshot',
      'valuesPolicy',
      'resolvedEnvIdentity',
      'resolvedEnv',
      'redactedResolvedEnv',
      'envShape'
    ]) {
      if (Object.prototype.hasOwnProperty.call(item, field)) {
        return `invalid router.control envelope: ${label}.${field} is no longer supported`;
      }
    }
    if (typeof item.serviceId !== 'string' || !isPublicationId(item.serviceId)) {
      return `invalid router.control envelope: ${label}.serviceId must be a publication id`;
    }
    if (
      typeof item.buildId !== 'string' ||
      !BUILD_ID_PATTERN.test(item.buildId)
    ) {
      return `invalid router.control envelope: ${label}.buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>`;
    }
    if (
      typeof item.activationIdentity !== 'string' ||
      !ACTIVATION_IDENTITY_PATTERN.test(item.activationIdentity)
    ) {
      return `invalid router.control envelope: ${label}.activationIdentity must be skiff-runtime-activation-v1:opaque:<opaque id>`;
    }
    if (
      typeof item.resolvedConfigIdentity !== 'string' ||
      !RESOLVED_CONFIG_IDENTITY_PATTERN.test(item.resolvedConfigIdentity)
    ) {
      return `invalid router.control envelope: ${label}.resolvedConfigIdentity must be skiff-config-resolved-v1:opaque:<opaque id>`;
    }
    if (!isRecord(item.resolvedConfig)) {
      return `invalid router.control envelope: ${label}.resolvedConfig must be an object`;
    }
    if (!isRecord(item.redactedResolvedConfig)) {
      return `invalid router.control envelope: ${label}.redactedResolvedConfig must be an object`;
    }
    if (
      typeof item.redactionProjectionIdentity !== 'string' ||
      !CONFIG_REDACTION_IDENTITY_PATTERN.test(item.redactionProjectionIdentity)
    ) {
      return `invalid router.control envelope: ${label}.redactionProjectionIdentity must be skiff-config-redaction-v1:sha256:<64 lowercase hex>`;
    }
    const configShapeError = validateConfigShape(item.configShape, `${label}.configShape`);
    if (configShapeError) {
      return configShapeError;
    }
    if (item.serviceDb !== undefined) {
      if (!isRecord(item.serviceDb)) {
        return `invalid router.control envelope: ${label}.serviceDb must be an object`;
      }
      if (typeof item.serviceDb.mongoUrl !== 'string' || item.serviceDb.mongoUrl.trim().length === 0) {
        return `invalid router.control envelope: ${label}.serviceDb.mongoUrl must be a non-empty string`;
      }
      if (
        typeof item.serviceDb.storageServiceId !== 'string' ||
        !isPublicationId(item.serviceDb.storageServiceId)
      ) {
        return `invalid router.control envelope: ${label}.serviceDb.storageServiceId must be a publication id`;
      }
      if (Object.prototype.hasOwnProperty.call(item.serviceDb, 'storageNamespace')) {
        return `invalid router.control envelope: ${label}.serviceDb.storageNamespace is no longer supported`;
      }
    }
    const packageConfigError = validatePackageConfigs(item, label);
    if (packageConfigError) {
      return packageConfigError;
    }
  }
  return null;
}

function validatePackageConfigs(item: Record<string, unknown>, serviceLabel: string): string | null {
  const value = item.packageConfigs;
  if (value === undefined) {
    return null;
  }
  if (!Array.isArray(value)) {
    return `invalid router.control envelope: ${serviceLabel}.packageConfigs must be an array`;
  }
  for (let index = 0; index < value.length; index += 1) {
    const packageConfig = value[index];
    const label = `${serviceLabel}.packageConfigs[${index}]`;
    if (!isRecord(packageConfig)) {
      return `invalid router.control envelope: ${label} must be an object`;
    }
    for (const field of [
      'valuesSnapshotIdentity',
      'valuesSnapshot',
      'redactedValuesSnapshot',
      'valuesPolicy',
      'resolvedEnvIdentity',
      'resolvedEnv',
      'redactedResolvedEnv',
      'envShape'
    ]) {
      if (Object.prototype.hasOwnProperty.call(packageConfig, field)) {
        return `invalid router.control envelope: ${label}.${field} is no longer supported`;
      }
    }
    if (typeof packageConfig.packageId !== 'string' || !isPublicationId(packageConfig.packageId)) {
      return `invalid router.control envelope: ${label}.packageId must be a publication id`;
    }
    if (
      packageConfig.packageSlot !== undefined &&
      (!Number.isInteger(packageConfig.packageSlot) || Number(packageConfig.packageSlot) < 0)
    ) {
      return `invalid router.control envelope: ${label}.packageSlot must be a non-negative integer`;
    }
    if (Object.prototype.hasOwnProperty.call(packageConfig, 'dependencyRef')) {
      return `invalid router.control envelope: ${label}.dependencyRef is no longer supported; use alias`;
    }
    if (typeof packageConfig.alias !== 'string') {
      return `invalid router.control envelope: ${label}.alias must be a string`;
    }
    if (
      typeof packageConfig.resolvedConfigIdentity !== 'string' ||
      !RESOLVED_CONFIG_IDENTITY_PATTERN.test(packageConfig.resolvedConfigIdentity)
    ) {
      return `invalid router.control envelope: ${label}.resolvedConfigIdentity must be skiff-config-resolved-v1:opaque:<opaque id>`;
    }
    if (!isRecord(packageConfig.resolvedConfig)) {
      return `invalid router.control envelope: ${label}.resolvedConfig must be an object`;
    }
    if (!isRecord(packageConfig.redactedResolvedConfig)) {
      return `invalid router.control envelope: ${label}.redactedResolvedConfig must be an object`;
    }
    if (
      typeof packageConfig.redactionProjectionIdentity !== 'string' ||
      !CONFIG_REDACTION_IDENTITY_PATTERN.test(packageConfig.redactionProjectionIdentity)
    ) {
      return `invalid router.control envelope: ${label}.redactionProjectionIdentity must be skiff-config-redaction-v1:sha256:<64 lowercase hex>`;
    }
    const configShapeError = validateConfigShape(packageConfig.configShape, `${label}.configShape`);
    if (configShapeError) {
      return configShapeError;
    }
  }
  return null;
}

function validateConfigShape(value: unknown, label: string): string | null {
  if (value === undefined) {
    return null;
  }
  if (!isRecord(value)) {
    return `invalid router.control envelope: ${label} must be an object`;
  }
  if (value.schemaVersion !== 'skiff-config-shape-v1') {
    return `invalid router.control envelope: ${label}.schemaVersion must be skiff-config-shape-v1`;
  }
  if (!Array.isArray(value.entries)) {
    return `invalid router.control envelope: ${label}.entries must be an array`;
  }
  const unsupportedShapeFields = Object.keys(value).filter(
    (key) => key !== 'schemaVersion' && key !== 'entries'
  );
  if (unsupportedShapeFields.length > 0) {
    return `invalid router.control envelope: ${label} does not support ${unsupportedShapeFields.join(', ')}`;
  }
  for (let index = 0; index < value.entries.length; index += 1) {
    const entry = value.entries[index];
    const entryLabel = `${label}.entries[${index}]`;
    if (!isRecord(entry)) {
      return `invalid router.control envelope: ${entryLabel} must be an object`;
    }
    if (typeof entry.path !== 'string') {
      return `invalid router.control envelope: ${entryLabel}.path must be a string`;
    }
    if (typeof entry.type !== 'string' || !isConfigShapeValueType(entry.type)) {
      return `invalid router.control envelope: ${entryLabel}.type must be string, number, bool, Json, or JsonObject`;
    }
    if (typeof entry.required !== 'boolean') {
      return `invalid router.control envelope: ${entryLabel}.required must be a boolean`;
    }
    const unsupportedEntryFields = Object.keys(entry).filter(
      (key) => key !== 'path' && key !== 'type' && key !== 'required'
    );
    if (unsupportedEntryFields.length > 0) {
      return `invalid router.control envelope: ${entryLabel} does not support ${unsupportedEntryFields.join(', ')}`;
    }
  }
  return null;
}

function validateResponseChunkFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'response.chunk') ??
    rejectUnsupportedFrameHeaderFields(envelope, 'response.chunk', [
      'schemaVersion',
      'type',
      'requestId',
      'seq'
    ]) ??
    requireString(envelope, 'response.chunk', 'requestId') ??
    requireInteger(envelope, 'response.chunk', 'seq')
  );
}

function validateResponseStartFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'response.start') ??
    rejectUnsupportedFrameHeaderFields(envelope, 'response.start', [
      'schemaVersion',
      'type',
      'requestId',
      'httpResponse'
    ]) ??
    requireString(envelope, 'response.start', 'requestId') ??
    (envelope.httpResponse === undefined
      ? 'invalid response.start envelope: httpResponse is required'
      : null) ??
    validateHttpResponseFrameMetadata(envelope, 'response.start')
  );
}

function validateResponseEndFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'response.end') ??
    rejectUnsupportedFrameHeaderFields(envelope, 'response.end', [
      'schemaVersion',
      'type',
      'requestId',
      'payloadPresent',
      'httpResponse',
      'websocketConnect',
      'websocketJsonRpc'
    ]) ??
    requireString(envelope, 'response.end', 'requestId') ??
    requireBoolean(envelope, 'response.end', 'payloadPresent') ??
    validateHttpResponseFrameMetadata(envelope, 'response.end') ??
    validateWebSocketConnectResponseFrameMetadata(envelope) ??
    validateWebSocketJsonRpcResponseFrameMetadata(envelope) ??
    validateResponseEndVariant(envelope)
  );
}

function validateResponseEndVariant(envelope: Record<string, unknown>): string | null {
  const metadataCount = [
    envelope.httpResponse,
    envelope.websocketConnect,
    envelope.websocketJsonRpc
  ].filter((value) => value !== undefined).length;
  if (metadataCount > 1) {
    return 'invalid response.end envelope: response metadata variants cannot be mixed';
  }
  if (envelope.websocketConnect !== undefined && envelope.payloadPresent !== false) {
    return 'invalid response.end envelope: websocketConnect payloadPresent must be false';
  }
  return null;
}

function validateHttpRequestFrameMetadata(envelope: Record<string, unknown>): string | null {
  if (envelope.httpRequest === undefined) {
    return null;
  }
  if (!isRecord(envelope.httpRequest)) {
    return 'invalid request.start envelope: httpRequest must be an object';
  }
  if (Object.prototype.hasOwnProperty.call(envelope.httpRequest, 'body')) {
    return 'invalid request.start frame header: httpRequest.body is not supported; use binary frame payload bytes';
  }
  return (
    requireString(envelope, 'request.start', 'httpRequest.method') ??
    requireString(envelope, 'request.start', 'httpRequest.url') ??
    requireString(envelope, 'request.start', 'httpRequest.path') ??
    validateNameValueArray(envelope.httpRequest.query, 'request.start', 'httpRequest.query') ??
    validateNameValueArray(envelope.httpRequest.headers, 'request.start', 'httpRequest.headers')
  );
}

function validateHttpAdapterFrameMetadata(envelope: Record<string, unknown>): string | null {
  if (envelope.httpAdapter === undefined) {
    return null;
  }
  if (!isRecord(envelope.httpAdapter)) {
    return 'invalid request.start envelope: httpAdapter must be an object';
  }
  if (Object.prototype.hasOwnProperty.call(envelope.httpAdapter, 'handlerArgs')) {
    return 'invalid request.start envelope: httpAdapter.handlerArgs is not supported; use adapterArgs';
  }
  return (
    requireEnum(envelope, 'request.start', 'httpAdapter.kind', ['typedJson', 'rawHttp']) ??
    requireObject(envelope, 'request.start', 'httpAdapter.handler') ??
    optionalObject(envelope, 'request.start', 'httpAdapter.guard') ??
    optionalObject(envelope, 'request.start', 'httpAdapter.pre') ??
    validateGatewayAdapterArgs(
      envelope.httpAdapter.adapterArgs,
      'request.start',
      'httpAdapter.adapterArgs',
      ['http.request', 'http.body', 'http.context']
    )
  );
}

function validateWebSocketAdapterFrameMetadata(envelope: Record<string, unknown>): string | null {
  if (envelope.websocketAdapter === undefined) {
    return null;
  }
  if (!isRecord(envelope.websocketAdapter)) {
    return 'invalid request.start envelope: websocketAdapter must be an object';
  }
  const kindError = requireEnum(envelope, 'request.start', 'websocketAdapter.kind', [
    'connect',
    'receive'
  ]);
  if (kindError) {
    return kindError;
  }
  const adapterArgsError = validateGatewayAdapterArgs(
    envelope.websocketAdapter.adapterArgs,
    'request.start',
    'websocketAdapter.adapterArgs',
    websocketAdapterSourceKinds
  );
  if (adapterArgsError) {
    return adapterArgsError;
  }
  const contextExpectationError = validateOptionalWebSocketContextExpectation(
    envelope.websocketAdapter.contextExpectation,
    'websocketAdapter.contextExpectation'
  );
  if (contextExpectationError) {
    return contextExpectationError;
  }
  const websocketContextError = envelope.assemblyIdentity === undefined
    ? requireString(envelope, 'request.start', 'websocketEntryId') ??
      requireString(envelope, 'request.start', 'gatewayEntryIdentity')
    : null;
  if (websocketContextError) {
    return websocketContextError;
  }
  if (envelope.websocketAdapter.kind === 'connect') {
    if (Object.prototype.hasOwnProperty.call(envelope.websocketAdapter, 'receiveEvent')) {
      return 'invalid request.start envelope: websocketAdapter.receiveEvent is not supported for connect';
    }
    return validateWebSocketConnectRequestMetadata(envelope.websocketAdapter.connectRequest);
  }
  if (Object.prototype.hasOwnProperty.call(envelope.websocketAdapter, 'connectRequest')) {
    return 'invalid request.start envelope: websocketAdapter.connectRequest is not supported for receive';
  }
  return validateWebSocketReceiveEventMetadata(envelope.websocketAdapter.receiveEvent);
}

function validateOptionalWebSocketContextExpectation(value: unknown, field: string): string | null {
  if (value === undefined) {
    return null;
  }
  if (!isRecord(value)) {
    return `invalid request.start envelope: ${field} must be an object`;
  }
  const kindError = requireEnum(value, 'request.start', `${field}.kind`, [
    'null',
    'typed'
  ]);
  if (kindError) {
    return kindError;
  }
  if (value.kind === 'null') {
    if (
      Object.prototype.hasOwnProperty.call(value, 'connectOperationAbiId') ||
      Object.prototype.hasOwnProperty.call(value, 'contextTypeIdentity')
    ) {
      return `invalid request.start envelope: ${field} null expectation must not include typed fields`;
    }
    return null;
  }
  return (
    requireString(value, 'request.start', 'connectOperationAbiId') ??
    requireString(value, 'request.start', 'contextTypeIdentity')
  );
}

function validateWebSocketConnectRequestMetadata(value: unknown): string | null {
  if (!isRecord(value)) {
    return 'invalid request.start envelope: websocketAdapter.connectRequest must be an object';
  }
  return (
    requireString(value, 'request.start', 'connectionId') ??
    requireString(value, 'request.start', 'url') ??
    validateNameValueArray(value.query, 'request.start', 'websocketAdapter.connectRequest.query') ??
    validateNameValueArray(value.headers, 'request.start', 'websocketAdapter.connectRequest.headers') ??
    validateNameValueArray(value.cookies, 'request.start', 'websocketAdapter.connectRequest.cookies') ??
    optionalString(value, 'request.start', 'version')
  );
}

function validateWebSocketReceiveEventMetadata(value: unknown): string | null {
  if (!isRecord(value)) {
    return 'invalid request.start envelope: websocketAdapter.receiveEvent must be an object';
  }
  return (
    requireString(value, 'request.start', 'connectionId') ??
    optionalString(value, 'request.start', 'businessIdentity') ??
    requireObject(value, 'request.start', 'message') ??
    requireEnum(value, 'request.start', 'message.tag', [
      'text',
      'binary'
    ]) ??
    requireEnum(value, 'request.start', 'message.encoding', [
      'utf8',
      'binary'
    ]) ??
    validateWebSocketPayloadSegments(value.payloadSegments) ??
    validateOptionalContextCodec(value.contextCodec, 'websocketAdapter.receiveEvent.contextCodec')
  );
}

function validateGatewayAdapterArgs(
  value: unknown,
  envelopeType: string,
  field: string,
  allowedKinds: readonly string[]
): string | null {
  if (value === undefined) {
    return null;
  }
  if (!Array.isArray(value)) {
    return `invalid ${envelopeType} envelope: ${field} must be an array`;
  }
  const params = new Set<string>();
  for (const [index, item] of value.entries()) {
    const label = `${field}[${index}]`;
    if (!isRecord(item)) {
      return `invalid ${envelopeType} envelope: ${label} must be an object`;
    }
    if (typeof item.param !== 'string' || item.param.trim().length === 0) {
      return `invalid ${envelopeType} envelope: ${label}.param must be a non-empty string`;
    }
    if (params.has(item.param)) {
      return `invalid ${envelopeType} envelope: ${field} has duplicate param ${item.param}`;
    }
    params.add(item.param);
    if (!isRecord(item.source)) {
      return `invalid ${envelopeType} envelope: ${label}.source must be an object`;
    }
    if (
      typeof item.source.kind !== 'string' ||
      !allowedKinds.includes(item.source.kind)
    ) {
      return `invalid ${envelopeType} envelope: ${label}.source.kind must be one of ${allowedKinds.join(', ')}`;
    }
  }
  return null;
}

function validateWebSocketPayloadSegments(value: unknown): string | null {
  if (!Array.isArray(value)) {
    return 'invalid request.start envelope: websocketAdapter.receiveEvent.payloadSegments must be an array';
  }
  for (const [index, item] of value.entries()) {
    const label = `websocketAdapter.receiveEvent.payloadSegments[${index}]`;
    if (!isRecord(item)) {
      return `invalid request.start envelope: ${label} must be an object`;
    }
    if (
      typeof item.kind !== 'string' ||
      !websocketPayloadSegmentKinds.includes(item.kind as (typeof websocketPayloadSegmentKinds)[number])
    ) {
      return `invalid request.start envelope: ${label}.kind must be one of ${websocketPayloadSegmentKinds.join(', ')}`;
    }
    if (!Number.isInteger(item.offset) || Number(item.offset) < 0) {
      return `invalid request.start envelope: ${label}.offset must be a non-negative integer`;
    }
    if (!Number.isInteger(item.length) || Number(item.length) < 0) {
      return `invalid request.start envelope: ${label}.length must be a non-negative integer`;
    }
  }
  return null;
}

function validateOptionalContextCodec(value: unknown, field: string): string | null {
  if (value === undefined) {
    return null;
  }
  if (!isRecord(value)) {
    return `invalid request.start envelope: ${field} must be an object`;
  }
  return (
    requireString(value, 'request.start', 'operationAbiId') ??
    requireString(value, 'request.start', 'contextTypeIdentity')
  );
}

function validateWebSocketConnectResponseFrameMetadata(
  envelope: Record<string, unknown>
): string | null {
  if (envelope.websocketConnect === undefined) {
    return null;
  }
  if (!isRecord(envelope.websocketConnect)) {
    return 'invalid response.end envelope: websocketConnect must be an object';
  }
  const metadata = envelope.websocketConnect;
  const resultError = requireEnum(
    envelope,
    'response.end',
    'websocketConnect.result',
    ['accept', 'reject']
  );
  if (resultError) {
    return resultError;
  }
  if (metadata.result === 'accept') {
    const acceptError =
      rejectUnsupportedObjectFields(metadata, 'response.end', 'websocketConnect accept', [
        'result',
        'businessIdentity',
        'connectionPolicy'
      ]) ??
      optionalString(envelope, 'response.end', 'websocketConnect.businessIdentity') ??
      validateWebSocketConnectionPolicy(metadata.connectionPolicy);
    if (acceptError) {
      return acceptError;
    }
  } else {
    const rejectError =
      rejectUnsupportedObjectFields(metadata, 'response.end', 'websocketConnect reject', [
        'result',
        'code',
        'reason'
      ]) ??
      requireUnsigned16Integer(envelope, 'response.end', 'websocketConnect.code') ??
      requireString(envelope, 'response.end', 'websocketConnect.reason');
    if (rejectError) {
      return rejectError;
    }
  }
  return null;
}

function validateWebSocketJsonRpcResponseFrameMetadata(
  envelope: Record<string, unknown>
): string | null {
  if (envelope.websocketJsonRpc === undefined) {
    return null;
  }
  if (!isRecord(envelope.websocketJsonRpc)) {
    return 'invalid response.end envelope: websocketJsonRpc must be an object';
  }
  const metadata = envelope.websocketJsonRpc;
  const fieldError =
    rejectUnsupportedObjectFields(
      metadata,
      'response.end',
      'websocketJsonRpc',
      ['outcome']
    ) ??
    requireEnum(envelope, 'response.end', 'websocketJsonRpc.outcome', [
      'success',
      'invalidParams',
      'internalError',
      'deadlineExceeded'
    ]) ??
    requireRuntimeAssemblyWebSocketJsonRpcCanonicalBoundedString(
      envelope,
      'response.end',
      'requestId',
      1024
    );
  if (fieldError !== null) {
    return fieldError;
  }
  const expectedPayloadPresent = metadata.outcome === 'success';
  return envelope.payloadPresent === expectedPayloadPresent
    ? null
    : 'invalid response.end envelope: websocketJsonRpc payloadPresent must match outcome';
}

function validateWebSocketConnectionPolicy(value: unknown): string | null {
  if (value === undefined) {
    return null;
  }
  if (!isRecord(value)) {
    return 'invalid response.end envelope: websocketConnect.connectionPolicy must be an object';
  }
  if (Object.prototype.hasOwnProperty.call(value, 'scope')) {
    return 'invalid response.end envelope: websocketConnect.connectionPolicy.scope is not supported';
  }
  const unsupported = rejectUnsupportedObjectFields(value, 'response.end', 'websocketConnect.connectionPolicy', [
    'maxConnections',
    'overflow',
    'closeCode',
    'closeReason'
  ]);
  if (unsupported !== null) {
    return unsupported;
  }
  const policy = value as Record<string, unknown>;
  if (
    !Number.isInteger(policy.maxConnections) ||
    Number(policy.maxConnections) <= 0 ||
    Number(policy.maxConnections) > 4_294_967_295
  ) {
    return 'invalid response.end envelope: websocketConnect.connectionPolicy.maxConnections must be an unsigned non-zero 32-bit integer';
  }
  if (policy.overflow !== 'close-oldest' && policy.overflow !== 'reject-new') {
    return 'invalid response.end envelope: websocketConnect.connectionPolicy.overflow must be one of close-oldest, reject-new';
  }
  return (
    optionalUnsigned16Integer(value, 'response.end', 'closeCode') ??
    optionalString(value, 'response.end', 'closeReason')
  );
}

function validateHttpResponseFrameMetadata(
  envelope: Record<string, unknown>,
  envelopeType: 'response.start' | 'response.end'
): string | null {
  if (envelope.httpResponse === undefined) {
    if (envelopeType === 'response.start') {
      return 'invalid response.start envelope: httpResponse must be an object';
    }
    return null;
  }
  if (!isRecord(envelope.httpResponse)) {
    return `invalid ${envelopeType} envelope: httpResponse must be an object`;
  }
  if (Object.prototype.hasOwnProperty.call(envelope.httpResponse, 'body')) {
    return `invalid ${envelopeType} frame header: httpResponse.body is not supported; use binary frame payload bytes`;
  }
  const unsupported = rejectUnsupportedObjectFields(envelope.httpResponse, envelopeType, 'httpResponse', [
    'status',
    'headers'
  ]);
  if (unsupported !== null) {
    return unsupported;
  }
  const status = envelope.httpResponse.status;
  if (!Number.isInteger(status) || Number(status) < 100 || Number(status) > 599) {
    return `invalid ${envelopeType} envelope: httpResponse.status must be an integer between 100 and 599`;
  }
  return validateNameValueArray(
    envelope.httpResponse.headers,
    envelopeType,
    'httpResponse.headers'
  );
}

function validateNameValueArray(
  value: unknown,
  envelopeType: string,
  field: string
): string | null {
  if (!Array.isArray(value)) {
    return `invalid ${envelopeType} envelope: ${field} must be an array`;
  }
  for (const [index, item] of value.entries()) {
    if (!isRecord(item)) {
      return `invalid ${envelopeType} envelope: ${field}[${index}] must be an object`;
    }
    if (typeof item.name !== 'string') {
      return `invalid ${envelopeType} envelope: ${field}[${index}].name must be a string`;
    }
    if (typeof item.value !== 'string') {
      return `invalid ${envelopeType} envelope: ${field}[${index}].value must be a string`;
    }
    const unsupported = rejectUnsupportedObjectFields(item, envelopeType, `${field}[${index}]`, [
      'name',
      'value'
    ]);
    if (unsupported !== null) {
      return unsupported;
    }
  }
  return null;
}

function validateResponseError(envelope: Record<string, unknown>): string | null {
  const commonError =
    rejectHeaderPayloadFields(envelope, 'response.error') ??
    requireNonBlankString(envelope, 'response.error', 'requestId') ??
    requireEnum(envelope, 'response.error', 'errorKind', ['fixedService', 'control']);
  if (commonError) {
    return commonError;
  }
  if (envelope.errorKind === 'fixedService') {
    return rejectUnsupportedFrameHeaderFields(envelope, 'response.error', [
      'schemaVersion',
      'type',
      'requestId',
      'errorKind'
    ]);
  }
  return (
    rejectUnsupportedFrameHeaderFields(envelope, 'response.error', [
      'schemaVersion',
      'type',
      'requestId',
      'errorKind',
      'error'
    ]) ??
    validateErrorPayload(envelope, 'response.error') ??
    requireNonBlankString(envelope, 'response.error', 'error.code') ??
    requireNonBlankString(envelope, 'response.error', 'error.message') ??
    rejectInternalCancellationErrorCode(envelope)
  );
}

function rejectInternalCancellationErrorCode(
  envelope: Record<string, unknown>
): string | null {
  if (
    isRecord(envelope.error) &&
    envelope.error.code === INTERNAL_CANCELLATION_RESERVED_ERROR_CODE
  ) {
    return 'invalid response.error envelope: error.code is reserved for internal cancellation';
  }
  return null;
}

function decodeServiceErrorEnvelope(
  payloadBytes: Uint8Array
): EnvelopeValidationResult<ServiceErrorEnvelopeView> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(payloadBytes));
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      ok: false,
      error: `invalid response.error fixedService frame: payload is not strict JSON: ${message}`
    };
  }
  if (!isRecord(parsed) || typeof parsed.kind !== 'string') {
    return {
      ok: false,
      error: 'invalid response.error fixedService frame: service error must be an object with kind'
    };
  }
  const validationError =
    parsed.kind === 'publicTypedError'
      ? validatePublicTypedServiceError(parsed)
      : parsed.kind === 'internalError'
        ? validateInternalServiceError(parsed)
        : parsed.kind === 'platformError'
          ? validatePlatformServiceError(parsed)
          : 'service error kind is not supported';
  if (validationError) {
    return {
      ok: false,
      error: `invalid response.error fixedService frame: ${validationError}`
    };
  }
  return {
    ok: true,
    envelope: parsed as unknown as ServiceErrorEnvelopeView
  };
}

function validatePublicTypedServiceError(envelope: Record<string, unknown>): string | null {
  return (
    rejectServiceErrorFields(envelope, [
      'kind',
      'packageId',
      'stableSchemaKey',
      'packageSchemaTypeId',
      'encodedPayload',
      'traceId',
      'errorId'
    ]) ??
    exactNonEmptyString(envelope.packageId, 'packageId') ??
    exactNonEmptyString(envelope.stableSchemaKey, 'stableSchemaKey') ??
    exactNonEmptyString(envelope.packageSchemaTypeId, 'packageSchemaTypeId') ??
    validateEncodedServiceErrorPayload(envelope.encodedPayload) ??
    validateServiceErrorCorrelation(envelope)
  );
}

function validateInternalServiceError(envelope: Record<string, unknown>): string | null {
  const topLevelError = rejectServiceErrorFields(envelope, ['kind', 'payload']);
  if (topLevelError) {
    return topLevelError;
  }
  if (!isRecord(envelope.payload)) {
    return 'payload must be an object';
  }
  return (
    rejectServiceErrorFields(envelope.payload, ['message', 'traceId', 'errorId'], 'payload') ??
    exactNonEmptyString(envelope.payload.message, 'payload.message') ??
    validateServiceErrorCorrelation(envelope.payload, 'payload.')
  );
}

function validatePlatformServiceError(envelope: Record<string, unknown>): string | null {
  return (
    rejectServiceErrorFields(envelope, [
      'kind',
      'builtinErrorIdentity',
      'encodedPayload',
      'traceId',
      'errorId'
    ]) ??
    (typeof envelope.builtinErrorIdentity === 'string' &&
    isAllowedType(envelope.builtinErrorIdentity, PLATFORM_SERVICE_ERROR_IDENTITIES)
      ? null
      : 'builtinErrorIdentity is not supported') ??
    validateEncodedServiceErrorPayload(envelope.encodedPayload) ??
    validateServiceErrorCorrelation(envelope)
  );
}

function rejectServiceErrorFields(
  value: Record<string, unknown>,
  allowedFields: readonly string[],
  prefix = ''
): string | null {
  const unknown = Object.keys(value).find((field) => !allowedFields.includes(field));
  return unknown === undefined ? null : `${prefix}${unknown} is not supported`;
}

function exactNonEmptyString(value: unknown, field: string): string | null {
  return typeof value === 'string' && value.length > 0 && value.trim() === value
    ? null
    : `${field} must be non-empty and contain no surrounding whitespace`;
}

function validateEncodedServiceErrorPayload(value: unknown): string | null {
  return Array.isArray(value) &&
    value.length > 0 &&
    value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)
    ? null
    : 'encodedPayload must be a non-empty byte array';
}

function validateServiceErrorCorrelation(
  envelope: Record<string, unknown>,
  prefix = ''
): string | null {
  return (
    exactNonEmptyString(envelope.traceId, `${prefix}traceId`) ??
    exactNonEmptyString(envelope.errorId, `${prefix}errorId`)
  );
}

function validateErrorPayload(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  const requestError = requireString(envelope, 'response.error', 'requestId');
  if (envelopeType === 'response.error' && requestError) {
    return requestError;
  }
  if (!isRecord(envelope.error)) {
    return `invalid ${envelopeType} envelope: error must be an object`;
  }
  return (
    rejectUnsupportedObjectFields(envelope.error, envelopeType, 'error', [
      'code',
      'message',
      'status',
      'details'
    ]) ??
    requireString(envelope, envelopeType, 'error.code') ??
    requireString(envelope, envelopeType, 'error.message') ??
    validateRuntimeErrorStatus(envelope.error, envelopeType)
  );
}

function validateRuntimeErrorStatus(
  error: Record<string, unknown>,
  envelopeType: string
): string | null {
  if (error.status === undefined) {
    return null;
  }
  if (!Number.isInteger(error.status) || Number(error.status) < 400 || Number(error.status) > 599) {
    return `invalid ${envelopeType} envelope: error.status must be an integer between 400 and 599`;
  }
  return null;
}

function validateRequestCancel(envelope: Record<string, unknown>): string | null {
  return (
    requireString(envelope, 'request.cancel', 'requestId') ??
    requireEnum(envelope, 'request.cancel', 'reason', cancelReasons)
  );
}

function validateConnectionSendFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'connection.send') ??
    rejectUnsupportedFrameHeaderFields(envelope, 'connection.send', [
      'schemaVersion',
      'type',
      'serviceId',
      'websocketEntryId',
      'businessIdentity',
      'connectionId',
      'payloadKind'
    ]) ??
    requireString(envelope, 'connection.send', 'serviceId') ??
    optionalString(envelope, 'connection.send', 'websocketEntryId') ??
    validateConnectionSendTarget(envelope) ??
    optionalEnum(envelope, 'connection.send', 'payloadKind', ['text', 'binary'])
  );
}

function validateConnectionRequestFrameHeader(
  envelope: Record<string, unknown>
): string | null {
  const envelopeType = 'connection.request';
  const fieldError =
    rejectHeaderPayloadFields(envelope, envelopeType) ??
    rejectUnsupportedFrameHeaderFields(envelope, envelopeType, [
      'schemaVersion',
      'type',
      'requestId',
      'serviceId',
      'websocketEntryId',
      'connectionId',
      'profile',
      'method',
      'deadline'
    ]) ??
    requireCanonicalBoundedString(envelope, envelopeType, 'requestId', 1024) ??
    requireCanonicalBoundedString(envelope, envelopeType, 'serviceId', 1024) ??
    requireStringPattern(
      envelope,
      envelopeType,
      'websocketEntryId',
      /^skiff-websocket-entry-v1:sha256:[0-9a-f]{64}$/,
      'skiff-websocket-entry-v1:sha256:<64 lowercase hex>'
    ) ??
    requireCanonicalBoundedString(envelope, envelopeType, 'connectionId', 1024) ??
    requireEnum(envelope, envelopeType, 'profile', ['jsonrpc-2.0-text']) ??
    requireCanonicalBoundedString(envelope, envelopeType, 'method', 256);
  if (fieldError !== null || envelope.deadline === undefined) {
    return fieldError;
  }
  if (!isRecord(envelope.deadline)) {
    return 'invalid connection.request envelope: deadline must be an object';
  }
  const unsupported = rejectUnsupportedObjectFields(
    envelope.deadline,
    envelopeType,
    'deadline',
    ['timeoutMs', 'expiresAt']
  );
  if (unsupported !== null) {
    return unsupported;
  }
  const timeoutMs = envelope.deadline.timeoutMs;
  const expiresAt = envelope.deadline.expiresAt;
  if (!Number.isSafeInteger(timeoutMs) || Number(timeoutMs) <= 0) {
    return 'invalid connection.request envelope: deadline.timeoutMs must be a positive safe integer';
  }
  if (
    typeof expiresAt !== 'string' ||
    !isStrictRfc3339UtcOrOffset(expiresAt)
  ) {
    return 'invalid connection.request envelope: deadline.expiresAt must be RFC3339';
  }
  return null;
}

function isStrictRfc3339UtcOrOffset(value: string): boolean {
  const match =
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:Z|([+-])(\d{2}):(\d{2}))$/.exec(
      value
    );
  if (match === null) {
    return false;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  const offsetHour = match[8] === undefined ? 0 : Number(match[8]);
  const offsetMinute = match[9] === undefined ? 0 : Number(match[9]);
  return (
    month >= 1 &&
    month <= 12 &&
    day >= 1 &&
    day <= daysInMonth(year, month) &&
    hour <= 23 &&
    minute <= 59 &&
    second <= 59 &&
    offsetHour <= 23 &&
    offsetMinute <= 59
  );
}

function daysInMonth(year: number, month: number): number {
  switch (month) {
    case 1:
    case 3:
    case 5:
    case 7:
    case 8:
    case 10:
    case 12:
      return 31;
    case 4:
    case 6:
    case 9:
    case 11:
      return 30;
    case 2:
      return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0)
        ? 29
        : 28;
    default:
      return 0;
  }
}

function validateConnectionRequestCancelFrameHeader(
  envelope: Record<string, unknown>
): string | null {
  const envelopeType = 'connection.request.cancel';
  return (
    rejectHeaderPayloadFields(envelope, envelopeType) ??
    rejectUnsupportedFrameHeaderFields(envelope, envelopeType, [
      'schemaVersion',
      'type',
      'requestId',
      'reason'
    ]) ??
    requireCanonicalBoundedString(envelope, envelopeType, 'requestId', 1024) ??
    requireEnum(envelope, envelopeType, 'reason', cancelReasons)
  );
}

function validateConnectionResponseFrameHeader(
  envelope: Record<string, unknown>
): string | null {
  const envelopeType = 'connection.response';
  const common =
    rejectHeaderPayloadFields(envelope, envelopeType) ??
    rejectUnsupportedFrameHeaderFields(envelope, envelopeType, [
      'schemaVersion',
      'type',
      'requestId',
      'outcome',
      'remote'
    ]) ??
    requireCanonicalBoundedString(envelope, envelopeType, 'requestId', 1024) ??
    requireEnum(envelope, envelopeType, 'outcome', [
      'success',
      'deadlineExceeded',
      'connectionUnavailable',
      'transportUnavailable',
      'protocolError',
      'resourceLimit',
      'remote'
    ]);
  if (common !== null) {
    return common;
  }
  if (envelope.outcome !== 'remote') {
    return envelope.remote === undefined
      ? null
      : 'invalid connection.response envelope: remote is only valid for remote outcome';
  }
  if (!isRecord(envelope.remote)) {
    return 'invalid connection.response envelope: remote outcome requires remote metadata';
  }
  const unsupported = rejectUnsupportedObjectFields(
    envelope.remote,
    envelopeType,
    'remote',
    ['code', 'message', 'dataPresent']
  );
  if (unsupported !== null) {
    return unsupported;
  }
  if (!Number.isSafeInteger(envelope.remote.code)) {
    return 'invalid connection.response envelope: remote.code must be a safe integer';
  }
  if (
    typeof envelope.remote.message !== 'string' ||
    envelope.remote.message.trim().length === 0 ||
    Buffer.byteLength(envelope.remote.message, 'utf8') > 4096
  ) {
    return 'invalid connection.response envelope: remote.message must be a bounded non-empty string';
  }
  return typeof envelope.remote.dataPresent === 'boolean'
    ? null
    : 'invalid connection.response envelope: remote.dataPresent must be a boolean';
}

function requireCanonicalBoundedString(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  maxBytes: number
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' &&
    value.length > 0 &&
    value.trim() === value &&
    Buffer.byteLength(value, 'utf8') <= maxBytes &&
    !/[\u0000-\u001f\u007f]/.test(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a bounded non-empty canonical string`;
}

function requireRuntimeAssemblyWebSocketJsonRpcCanonicalBoundedString(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  maxBytes: number
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' &&
    value.length > 0 &&
    value.trim() === value &&
    Buffer.byteLength(value, 'utf8') <= maxBytes &&
    !/\p{Cc}/u.test(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a bounded non-empty canonical string`;
}

function validateConnectionSendTarget(envelope: Record<string, unknown>): string | null {
  if (Object.prototype.hasOwnProperty.call(envelope, 'identity')) {
    return 'invalid connection.send envelope: identity is not supported; use businessIdentity';
  }
  const hasIdentity = Object.prototype.hasOwnProperty.call(envelope, 'businessIdentity');
  const hasConnectionId = Object.prototype.hasOwnProperty.call(envelope, 'connectionId');
  if (hasIdentity === hasConnectionId) {
    return 'invalid connection.send envelope: exactly one of businessIdentity or connectionId must be set';
  }
  if (hasIdentity) {
    return validateConnectionSendIdentity(envelope);
  }
  return validateConnectionSendConnectionId(envelope);
}

function validateConnectionSendIdentity(envelope: Record<string, unknown>): string | null {
  const value = envelope.businessIdentity;
  if (typeof value !== 'string' || value.trim().length === 0) {
    return 'invalid connection.send envelope: businessIdentity must be a non-empty string';
  }
  if (typeof envelope.websocketEntryId !== 'string' || envelope.websocketEntryId.trim().length === 0) {
    return 'invalid connection.send envelope: websocketEntryId must be a non-empty string for businessIdentity target';
  }
  return null;
}

function validateConnectionSendConnectionId(envelope: Record<string, unknown>): string | null {
  const value = envelope.connectionId;
  if (typeof value !== 'string' || value.trim().length === 0) {
    return 'invalid connection.send envelope: connectionId must be a non-empty string';
  }
  if (typeof envelope.websocketEntryId !== 'string' || envelope.websocketEntryId.trim().length === 0) {
    return 'invalid connection.send envelope: websocketEntryId must be a non-empty string for connectionId target';
  }
  return null;
}

function validateFrameHeaderBase(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  const schemaVersion =
    envelopeType === 'response.error'
      ? RESPONSE_ERROR_FRAME_SCHEMA_VERSION
      : RUNTIME_FRAME_SCHEMA_VERSION;
  return requireEnum(envelope, `${envelopeType} frame header`, 'schemaVersion', [
    schemaVersion
  ]);
}

function rejectHeaderPayloadFields(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  for (const field of ['args', 'payload', 'payloadBytes', 'data']) {
    if (Object.prototype.hasOwnProperty.call(envelope, field)) {
      return `invalid ${envelopeType} frame header: ${field} is not supported; use binary frame payload bytes`;
    }
  }
  return null;
}

function rejectUnsupportedFrameHeaderFields(
  envelope: Record<string, unknown>,
  envelopeType: string,
  allowedFields: readonly string[]
): string | null {
  const allowed = new Set(allowedFields);
  const unsupported = Object.keys(envelope).find((field) => !allowed.has(field));
  return unsupported === undefined
    ? null
    : `invalid ${envelopeType} frame header envelope: ${unsupported} is not supported`;
}

function rejectUnsupportedObjectFields(
  value: Record<string, unknown>,
  envelopeType: string,
  field: string,
  allowedFields: readonly string[]
): string | null {
  const allowed = new Set(allowedFields);
  const unsupported = Object.keys(value).find((key) => !allowed.has(key));
  return unsupported === undefined
    ? null
    : `invalid ${envelopeType} envelope: ${field}.${unsupported} is not supported`;
}

function validateCaller(
  envelope: Record<string, unknown>,
  envelopeType = 'request.start'
): string | null {
  if (!isRecord(envelope.caller)) {
    return `invalid ${envelopeType} envelope: caller must be an object`;
  }
  return (
    requireEnum(envelope, envelopeType, 'caller.kind', ['gateway', 'service']) ??
    requireString(envelope, envelopeType, 'caller.target')
  );
}

function validatePackageTestCaller(envelope: Record<string, unknown>): string | null {
  if (!isRecord(envelope.caller)) {
    return 'invalid package-test.start envelope: caller must be an object';
  }
  return (
    requireEnum(envelope, 'package-test.start', 'caller.kind', ['gateway']) ??
    requireString(envelope, 'package-test.start', 'caller.target')
  );
}

function validateDeadline(
  envelope: Record<string, unknown>,
  envelopeType = 'request.start'
): string | null {
  if (envelope.deadline === undefined) {
    return null;
  }
  if (!isRecord(envelope.deadline)) {
    return `invalid ${envelopeType} envelope: deadline must be an object`;
  }
  return (
    requireNumber(envelope, envelopeType, 'deadline.timeoutMs') ??
    requireString(envelope, envelopeType, 'deadline.expiresAt')
  );
}

function validateRuntimeClientSession(value: unknown): string | null {
  if (value === undefined) {
    return null;
  }
  if (!isRecord(value)) {
    return 'invalid request.start envelope: clientSession must be an object';
  }
  const supported = ['id'];
  const unsupported = Object.keys(value).find((key) => !supported.includes(key));
  if (unsupported !== undefined) {
    return `invalid request.start envelope: clientSession.${unsupported} is not supported`;
  }
  return typeof value.id === 'string'
    ? null
    : 'invalid request.start envelope: clientSession.id must be a string';
}

function validateTrace(
  envelope: Record<string, unknown>,
  envelopeType = 'request.start'
): string | null {
  if (!isRecord(envelope.trace)) {
    return `invalid ${envelopeType} envelope: trace must be an object`;
  }
  return (
    requireString(envelope, envelopeType, 'trace.traceId') ??
    requireString(envelope, envelopeType, 'trace.spanId') ??
    optionalString(envelope, envelopeType, 'trace.parentSpanId') ??
    optionalBoolean(envelope, envelopeType, 'trace.sampled')
  );
}

function requireString(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  return getPath(envelope, field, (value) => typeof value === 'string')
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a string`;
}

function requireNonBlankString(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' && value.trim().length > 0
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be non-empty`;
}

function requireObject(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return isRecord(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be an object`;
}

function optionalObject(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || isRecord(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be an object`;
}

function requirePublicationId(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' && isPublicationId(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a publication id`;
}

function forbiddenField(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  return Object.prototype.hasOwnProperty.call(envelope, field)
    ? `invalid ${envelopeType} envelope: ${field} is not supported`
    : null;
}

function optionalPublicationId(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || (typeof value === 'string' && isPublicationId(value))
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a publication id`;
}

function optionalString(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || typeof value === 'string'
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a string`;
}

function optionalStringPattern(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  pattern: RegExp,
  description: string
): string | null {
  const value = getPathValue(envelope, field);
  if (value === undefined) {
    return null;
  }
  return typeof value === 'string' && pattern.test(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be ${description}`;
}

function requireStringPattern(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  pattern: RegExp,
  description: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' && pattern.test(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be ${description}`;
}

function requirePattern(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  pattern: RegExp,
  description: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' && pattern.test(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be ${description}`;
}

function optionalBoolean(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || typeof value === 'boolean'
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a boolean`;
}

function requireBoolean(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'boolean'
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a boolean`;
}

function requireNumber(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'number' && Number.isFinite(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a number`;
}

function optionalPositiveNumber(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || (typeof value === 'number' && Number.isFinite(value) && value > 0)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a positive number`;
}

function requireInteger(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return Number.isInteger(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be an integer`;
}

function requirePositiveInteger(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return Number.isInteger(value) && Number(value) > 0
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a positive integer`;
}

function requireUnsigned16Integer(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return Number.isInteger(value) && Number(value) >= 0 && Number(value) <= 65_535
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be an unsigned 16-bit integer`;
}

function optionalUnsigned16Integer(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined ||
    (Number.isInteger(value) && Number(value) >= 0 && Number(value) <= 65_535)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be an unsigned 16-bit integer`;
}

function optionalPositiveInteger(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || (Number.isInteger(value) && Number(value) > 0)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a positive integer`;
}

function requireEnum<const TValue extends string>(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  allowedValues: readonly TValue[]
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' && isAllowedType(value, allowedValues)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be one of ${allowedValues.join(', ')}`;
}

function optionalEnum<const TValue extends string>(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  allowedValues: readonly TValue[]
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || (typeof value === 'string' && isAllowedType(value, allowedValues))
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be one of ${allowedValues.join(', ')}`;
}

function requireNonEmptyStringArray(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = envelope[field];
  return Array.isArray(value) && value.length > 0 && value.every((item) => typeof item === 'string')
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a non-empty string array`;
}

function optionalStringArray(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = envelope[field];
  return value === undefined ||
    (Array.isArray(value) && value.every((item) => typeof item === 'string'))
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a string array`;
}

function optionalStringArrayPattern(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string,
  pattern: RegExp,
  description: string
): string | null {
  const value = envelope[field];
  if (value === undefined) {
    return null;
  }
  if (!Array.isArray(value)) {
    return `invalid ${envelopeType} envelope: ${field} must be a string array`;
  }
  return value.every((item) => typeof item === 'string' && pattern.test(item))
    ? null
    : `invalid ${envelopeType} envelope: ${field} items must be ${description}`;
}

function optionalRecord(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return value === undefined || isRecord(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be an object`;
}

function validateBase64String(
  envelope: Record<string, unknown>,
  envelopeType: string,
  field: string
): string | null {
  const value = getPathValue(envelope, field);
  return typeof value === 'string' && value.length > 0 && BASE64_PATTERN.test(value)
    ? null
    : `invalid ${envelopeType} envelope: ${field} must be a non-empty base64 string`;
}

function getPath(
  envelope: Record<string, unknown>,
  field: string,
  predicate: (value: unknown) => boolean
): boolean {
  return predicate(getPathValue(envelope, field));
}

function getPathValue(envelope: Record<string, unknown>, field: string): unknown {
  let value: unknown = envelope;
  for (const part of field.split('.')) {
    if (!isRecord(value)) {
      return undefined;
    }
    value = value[part];
  }
  return value;
}

function isAllowedType<const TValue extends string>(
  value: string,
  allowedValues: readonly TValue[]
): value is TValue {
  return (allowedValues as readonly string[]).includes(value);
}
