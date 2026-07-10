import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS,
  isRecord,
  type RequestCancelReason,
  type RouterToRuntimeFrameHeader,
  type RuntimeFrameHeader,
  type RuntimeFrameHeaderName,
  type RuntimeToRouterFrameHeader,
  type TelemetryTopic
} from './envelope.js';
import {
  REQUEST_CANCEL_REASONS,
  REQUEST_CANCEL_REASON_BY_SITUATION
} from './cancelReason.js';
import { CONFIG_SHAPE_VALUE_TYPES, isConfigShapeValueType } from '../config/index.js';
import { isPublicationId, publicationStorageSegment } from '../publicationId.js';

export type RuntimeProtocolFrameHeaderName = RuntimeFrameHeaderName;
export type RuntimeToRouterFrameHeaderName = RuntimeToRouterFrameHeader['type'];
export type RouterToRuntimeFrameHeaderName = RouterToRuntimeFrameHeader['type'];

export interface ProtocolSchemaProperty {
  type: string | readonly string[];
  enum?: readonly string[];
  required?: readonly string[];
  properties?: Record<string, ProtocolSchemaProperty>;
  items?: ProtocolSchemaProperty;
  additionalProperties?: boolean;
}

export interface ProtocolEnvelopeSchema {
  type: 'object';
  required: readonly string[];
  properties: Record<string, ProtocolSchemaProperty>;
  additionalProperties: boolean;
}

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

const runtimeToRouterFrameHeaderTypes = [
  'runtime.register',
  'runtime.capabilities',
  'actor.put.request',
  'actor.find.request',
  'actor.remove.request',
  'spawn.submit.request',
  'spawn.claim.request',
  'spawn.renew.request',
  'spawn.complete.request',
  'spawn.fail.request',
  'request.start',
  'request.cancel',
  'connection.send',
  'response.start',
  'response.chunk',
  'response.end',
  'response.error'
] as const satisfies readonly RuntimeToRouterFrameHeaderName[];

const routerToRuntimeFrameHeaderTypes = [
  'router.control',
  'runtime.registered',
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
  'response.chunk',
  'response.end',
  'response.error'
] as const satisfies readonly RouterToRuntimeFrameHeaderName[];

const PROTOCOL_IDENTITY_PATTERN = /^skiff-protocol-v1:sha256:[0-9a-f]{64}$/;
const GATEWAY_IDENTITY_PATTERN = /^skiff-gateway-v1:sha256:[0-9a-f]{64}$/;
const BUILD_ID_PATTERN = /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/;
const PACKAGE_TEST_BUILD_ID_PATTERN = /^skiff-package-test-build-v1:sha256:[0-9a-f]{64}$/;
const SERVICE_OR_PACKAGE_TEST_BUILD_ID_PATTERN =
  /^skiff-(?:service|package-test)-build-v1:sha256:[0-9a-f]{64}$/;
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
const spawnFailReasons = ['failed', 'cancelled', 'timed_out'] as const;
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
  protocolVersion: { type: 'string' },
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

const runtimeRegisteredProperties = {
  type: { type: 'string', enum: ['runtime.registered'] },
  runtimeId: { type: 'string' }
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

const runtimeRpcRequestBaseProperties = {
  rpcId: { type: 'string' },
  runtimeId: { type: 'string' }
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

const spawnClaimDescriptorProperties = {
  itemId: { type: 'string' },
  leaseId: { type: 'string' },
  spawnExecutionId: { type: 'string' },
  runtimeRequestId: { type: 'string' },
  spawnId: { type: 'string' },
  targetKind: { type: 'string', enum: spawnTargetKinds },
  target: { type: 'string' },
  serviceId: { type: 'string' },
  serviceVersion: { type: 'string' },
  serviceProtocolIdentity: { type: 'string' },
  buildId: { type: 'string' },
  payloadSchemaIdentity: { type: 'string' },
  leaseExpiresAt: { type: 'string' }
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

const responseErrorProperties = {
  type: { type: 'string', enum: ['response.error'] },
  requestId: { type: 'string' },
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

const requestCancelProperties = {
  type: { type: 'string', enum: ['request.cancel'] },
  requestId: { type: 'string' },
  reason: { type: 'string', enum: cancelReasons }
} as const satisfies Record<string, ProtocolSchemaProperty>;

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
  'runtime.registered': {
    type: 'object',
    required: ['schemaVersion', 'type', 'runtimeId'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...runtimeRegisteredProperties
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
  'actor.put.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'rpcId',
      'runtimeId',
      'actorKey',
      'objectSchemaIdentity',
      'objectEncodingVersion'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.put.request'] },
      ...runtimeRpcRequestBaseProperties,
      actorKey: actorKeySchema,
      objectSchemaIdentity: { type: 'string' },
      objectEncodingVersion: { type: 'string' }
    },
    additionalProperties: false
  },
  'actor.put.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'actorRef'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.put.response'] },
      ...runtimeRpcResponseBaseProperties,
      actorRef: actorRefSchema
    },
    additionalProperties: false
  },
  'actor.put.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['actor.put.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'actor.find.request': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'runtimeId', 'actorKey'],
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
    required: ['schemaVersion', 'type', 'rpcId', 'runtimeId', 'actorKey'],
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
      'targetKind',
      'serviceId',
      'serviceVersion',
      'serviceProtocolIdentity',
      'target'
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
      activationIdentity: { type: 'string' },
      callerRequestId: { type: 'string' },
      traceId: { type: 'string' },
      callerTarget: { type: 'string' },
      maxQueueWaitMs: { type: 'number' }
    },
    additionalProperties: false
  },
  'spawn.submit.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'spawnId', 'itemId', 'status'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.submit.response'] },
      ...runtimeRpcResponseBaseProperties,
      spawnId: { type: 'string' },
      itemId: { type: 'string' },
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
  'spawn.claim.request': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'rpcId',
      'runtimeId',
      'workerId',
      'serviceId',
      'serviceVersion',
      'serviceProtocolIdentity',
      'supportedTargets',
      'supportedSpawnCompatibilityKeys'
    ],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.claim.request'] },
      ...runtimeRpcRequestBaseProperties,
      workerId: { type: 'string' },
      serviceId: { type: 'string' },
      serviceVersion: { type: 'string' },
      serviceProtocolIdentity: { type: 'string' },
      supportedTargets: { type: 'array', items: { type: 'string' } },
      supportedSpawnCompatibilityKeys: { type: 'array', items: { type: 'string' } },
      buildId: { type: 'string' },
      maxExecutionMs: { type: 'number' },
      maxConcurrency: { type: 'number' }
    },
    additionalProperties: false
  },
  'spawn.claim.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'claimed'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.claim.response'] },
      ...runtimeRpcResponseBaseProperties,
      claimed: { type: 'boolean' },
      item: {
        type: 'object',
        required: [
          'itemId',
          'leaseId',
          'spawnExecutionId',
          'runtimeRequestId',
          'spawnId',
          'targetKind',
          'target',
          'serviceId',
          'serviceVersion',
          'serviceProtocolIdentity',
          'buildId'
        ],
        properties: spawnClaimDescriptorProperties,
        additionalProperties: false
      }
    },
    additionalProperties: false
  },
  'spawn.claim.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.claim.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'spawn.renew.request': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'runtimeId', 'itemId', 'leaseId', 'workerId'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.renew.request'] },
      ...runtimeRpcRequestBaseProperties,
      itemId: { type: 'string' },
      leaseId: { type: 'string' },
      workerId: { type: 'string' }
    },
    additionalProperties: false
  },
  'spawn.renew.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'itemId', 'renewed'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.renew.response'] },
      ...runtimeRpcResponseBaseProperties,
      itemId: { type: 'string' },
      renewed: { type: 'boolean' },
      leaseExpiresAt: { type: 'string' }
    },
    additionalProperties: false
  },
  'spawn.renew.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.renew.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'spawn.complete.request': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'runtimeId', 'itemId', 'leaseId'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.complete.request'] },
      ...runtimeRpcRequestBaseProperties,
      itemId: { type: 'string' },
      leaseId: { type: 'string' },
      diagnostics: { type: 'object', additionalProperties: true }
    },
    additionalProperties: false
  },
  'spawn.complete.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'itemId', 'status'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.complete.response'] },
      ...runtimeRpcResponseBaseProperties,
      itemId: { type: 'string' },
      status: { type: 'string', enum: ['completed'] }
    },
    additionalProperties: false
  },
  'spawn.complete.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.complete.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'spawn.fail.request': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'runtimeId', 'itemId', 'leaseId', 'reason'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.fail.request'] },
      ...runtimeRpcRequestBaseProperties,
      itemId: { type: 'string' },
      leaseId: { type: 'string' },
      reason: { type: 'string', enum: spawnFailReasons },
      diagnostics: { type: 'object', additionalProperties: true }
    },
    additionalProperties: false
  },
  'spawn.fail.response': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'itemId', 'status'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.fail.response'] },
      ...runtimeRpcResponseBaseProperties,
      itemId: { type: 'string' },
      status: { type: 'string', enum: spawnFailReasons }
    },
    additionalProperties: false
  },
  'spawn.fail.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'rpcId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['spawn.fail.error'] },
      ...runtimeControlErrorProperties
    },
    additionalProperties: false
  },
  'request.start': {
    type: 'object',
    required: [
      'schemaVersion',
      'type',
      'requestId',
      'mode',
      'caller',
      'target',
      'operationAbiId',
      'buildId',
      'serviceProtocolIdentity',
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
    type: 'object',
    required: ['schemaVersion', 'type', 'requestId', 'payloadPresent'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      type: { type: 'string', enum: ['response.end'] },
      requestId: { type: 'string' },
      payloadPresent: { type: 'boolean' },
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
      },
      websocketConnect: {
        type: 'object',
        required: ['result', 'contextPayloadPresent'],
        properties: {
          result: { type: 'string', enum: ['accept', 'reject'] },
          businessIdentity: { type: 'string' },
          connectionPolicy: {
            type: 'object',
            required: ['maxConnections', 'overflow'],
            properties: {
              maxConnections: { type: 'integer' },
              overflow: { type: 'string', enum: ['close-oldest', 'reject-new'] },
              closeCode: { type: 'integer' },
              closeReason: { type: 'string' }
            },
            additionalProperties: false
          },
          contextCodec: {
            type: 'object',
            required: ['operationAbiId', 'contextTypeIdentity'],
            properties: {
              operationAbiId: { type: 'string' },
              contextTypeIdentity: { type: 'string' }
            },
            additionalProperties: false
          },
          contextPayloadPresent: { type: 'boolean' },
          code: { type: 'integer' },
          reason: { type: 'string' }
        },
        additionalProperties: false
      }
    },
    additionalProperties: false
  },
  'response.error': {
    type: 'object',
    required: ['schemaVersion', 'type', 'requestId', 'error'],
    properties: {
      schemaVersion: { type: 'string', enum: [RUNTIME_FRAME_SCHEMA_VERSION] },
      ...responseErrorProperties
    },
    additionalProperties: false
  },
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
  }
} as const satisfies Record<RuntimeProtocolFrameHeaderName, ProtocolEnvelopeSchema>;

const runtimeRegisterTargetFixture = 'service.example~com~~hello.HelloApi.hello' as const;
const spawnTargetFixture = `function:${runtimeRegisterTargetFixture}` as const;

const runtimeRegisterFixture = {
  type: 'runtime.register',
  runtimeId: 'runtime-fixture-1',
  serviceId: 'example.com/hello',
  revisionId: '1111111111111111111111111111111111111111111111111111111111111111',
  buildId:
    'skiff-service-build-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333',
  serviceProtocolIdentity:
    'skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111',
  targets: [runtimeRegisterTargetFixture] as string[],
  protocolVersion: 'skiff-protocol-v1',
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
  serviceProtocolIdentity:
    'skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111',
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

const spawnFixture = {
  runtimeId: runtimeRegisterFixture.runtimeId,
  workerId: 'spawn-worker-fixture-1',
  serviceId: runtimeRegisterFixture.serviceId,
  serviceVersion: '0.1.0',
  serviceProtocolIdentity: runtimeRegisterFixture.serviceProtocolIdentity,
  buildId: runtimeRegisterFixture.buildId,
  target: spawnTargetFixture,
  spawnCompatibilityKey: `${'0.1.0'}:${runtimeRegisterFixture.serviceProtocolIdentity}:${spawnTargetFixture}`,
  spawnId: 'spawn-fixture-1',
  itemId: 'spawn-item-fixture-1',
  leaseId: 'spawn-lease-fixture-1',
  spawnExecutionId: 'spawn-exec-fixture-1',
  runtimeRequestId: 'spawn-request-fixture-1'
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
  'runtime.registered': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...runtimeRegisteredFixture
  },
  'router.control': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    ...routerControlFixture
  },
  'actor.put.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.put.request',
    rpcId: 'actor-put-rpc-fixture-1',
    runtimeId: runtimeRegisterFixture.runtimeId,
    actorKey: actorKeyFixture,
    objectSchemaIdentity: 'schema.example.ThreadActorState',
    objectEncodingVersion: 'json-v1'
  },
  'actor.put.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.put.response',
    rpcId: 'actor-put-rpc-fixture-1',
    actorRef: actorRefFixture
  },
  'actor.put.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.put.error',
    rpcId: 'actor-put-rpc-fixture-1',
    error: {
      code: 'ActorPutFixtureError',
      message: 'fixture actor put failed'
    }
  },
  'actor.find.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'actor.find.request',
    rpcId: 'actor-find-rpc-fixture-1',
    runtimeId: runtimeRegisterFixture.runtimeId,
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
    targetKind: 'function',
    serviceId: spawnFixture.serviceId,
    serviceVersion: spawnFixture.serviceVersion,
    serviceProtocolIdentity: spawnFixture.serviceProtocolIdentity,
    target: spawnFixture.target,
    spawnId: spawnFixture.spawnId,
    buildId: runtimeRegisterFixture.buildId,
    callerRequestId: 'caller-request-fixture-1',
    traceId: 'trace-fixture-1',
    callerTarget: runtimeRegisterTargetFixture,
    maxQueueWaitMs: 30000
  },
  'spawn.submit.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.submit.response',
    rpcId: 'spawn-submit-rpc-fixture-1',
    spawnId: spawnFixture.spawnId,
    itemId: spawnFixture.itemId,
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
  'spawn.claim.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.claim.request',
    rpcId: 'spawn-claim-rpc-fixture-1',
    runtimeId: spawnFixture.runtimeId,
    workerId: spawnFixture.workerId,
    serviceId: spawnFixture.serviceId,
    serviceVersion: spawnFixture.serviceVersion,
    serviceProtocolIdentity: spawnFixture.serviceProtocolIdentity,
    supportedTargets: [spawnFixture.target],
    supportedSpawnCompatibilityKeys: [spawnFixture.spawnCompatibilityKey],
    maxExecutionMs: 30000,
    maxConcurrency: 4
  },
  'spawn.claim.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.claim.response',
    rpcId: 'spawn-claim-rpc-fixture-1',
    claimed: true,
    item: {
      itemId: spawnFixture.itemId,
      leaseId: spawnFixture.leaseId,
      spawnExecutionId: spawnFixture.spawnExecutionId,
      runtimeRequestId: spawnFixture.runtimeRequestId,
      spawnId: spawnFixture.spawnId,
      targetKind: 'function',
      target: spawnFixture.target,
      serviceId: spawnFixture.serviceId,
      serviceVersion: spawnFixture.serviceVersion,
      serviceProtocolIdentity: spawnFixture.serviceProtocolIdentity,
      buildId: spawnFixture.buildId,
      payloadSchemaIdentity: `skiff-spawn-payload-v1:${spawnFixture.serviceProtocolIdentity}:${spawnFixture.target}`,
      leaseExpiresAt: '2026-06-06T10:00:30.000Z'
    }
  },
  'spawn.claim.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.claim.error',
    rpcId: 'spawn-claim-rpc-fixture-1',
    error: {
      code: 'SpawnClaimFixtureError',
      message: 'fixture spawn claim failed'
    }
  },
  'spawn.renew.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.renew.request',
    rpcId: 'spawn-renew-rpc-fixture-1',
    runtimeId: spawnFixture.runtimeId,
    itemId: spawnFixture.itemId,
    leaseId: spawnFixture.leaseId,
    workerId: spawnFixture.workerId
  },
  'spawn.renew.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.renew.response',
    rpcId: 'spawn-renew-rpc-fixture-1',
    itemId: spawnFixture.itemId,
    renewed: true,
    leaseExpiresAt: '2026-06-06T10:00:30.000Z'
  },
  'spawn.renew.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.renew.error',
    rpcId: 'spawn-renew-rpc-fixture-1',
    error: {
      code: 'SpawnRenewFixtureError',
      message: 'fixture spawn renew failed'
    }
  },
  'spawn.complete.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.complete.request',
    rpcId: 'spawn-complete-rpc-fixture-1',
    runtimeId: spawnFixture.runtimeId,
    itemId: spawnFixture.itemId,
    leaseId: spawnFixture.leaseId,
    diagnostics: {
      ok: true
    }
  },
  'spawn.complete.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.complete.response',
    rpcId: 'spawn-complete-rpc-fixture-1',
    itemId: spawnFixture.itemId,
    status: 'completed'
  },
  'spawn.complete.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.complete.error',
    rpcId: 'spawn-complete-rpc-fixture-1',
    error: {
      code: 'SpawnCompleteFixtureError',
      message: 'fixture spawn complete failed'
    }
  },
  'spawn.fail.request': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.fail.request',
    rpcId: 'spawn-fail-rpc-fixture-1',
    runtimeId: spawnFixture.runtimeId,
    itemId: spawnFixture.itemId,
    leaseId: spawnFixture.leaseId,
    reason: 'failed',
    diagnostics: {
      reason: 'fixture'
    }
  },
  'spawn.fail.response': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.fail.response',
    rpcId: 'spawn-fail-rpc-fixture-1',
    itemId: spawnFixture.itemId,
    status: 'failed'
  },
  'spawn.fail.error': {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'spawn.fail.error',
    rpcId: 'spawn-fail-rpc-fixture-1',
    error: {
      code: 'SpawnFailFixtureError',
      message: 'fixture spawn fail failed'
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
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
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
      : type === 'actor.put.request'
        ? validateActorPutRequest(envelope)
      : type === 'actor.find.request'
        ? validateActorFindRequest(envelope)
      : type === 'actor.remove.request'
        ? validateActorRemoveRequest(envelope)
      : type === 'spawn.submit.request'
        ? validateSpawnSubmitRequest(envelope)
      : type === 'spawn.claim.request'
        ? validateSpawnClaimRequest(envelope)
      : type === 'spawn.renew.request'
        ? validateSpawnRenewRequest(envelope)
      : type === 'spawn.complete.request'
        ? validateSpawnCompleteRequest(envelope)
      : type === 'spawn.fail.request'
        ? validateSpawnFailRequest(envelope)
      : type === 'request.start'
        ? validateRequestStartFrameHeader(envelope)
      : type === 'request.cancel'
        ? validateRequestCancel(envelope)
        : type === 'connection.send'
          ? validateConnectionSendFrameHeader(envelope)
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
    (type === 'router.control'
      ? validateRouterControl(envelope)
      : type === 'runtime.registered'
        ? validateRuntimeRegistered(envelope)
      : type === 'actor.put.response'
        ? validateActorPutResponse(envelope)
      : type === 'actor.put.error'
        ? validateRuntimeControlError(envelope, 'actor.put.error')
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
      : type === 'spawn.claim.response'
        ? validateSpawnClaimResponse(envelope)
      : type === 'spawn.claim.error'
        ? validateRuntimeControlError(envelope, 'spawn.claim.error')
      : type === 'spawn.renew.response'
        ? validateSpawnRenewResponse(envelope)
      : type === 'spawn.renew.error'
        ? validateRuntimeControlError(envelope, 'spawn.renew.error')
      : type === 'spawn.complete.response'
        ? validateSpawnCompleteResponse(envelope)
      : type === 'spawn.complete.error'
        ? validateRuntimeControlError(envelope, 'spawn.complete.error')
      : type === 'spawn.fail.response'
        ? validateSpawnFailResponse(envelope)
      : type === 'spawn.fail.error'
        ? validateRuntimeControlError(envelope, 'spawn.fail.error')
      : type === 'request.start'
        ? validateRequestStartFrameHeader(envelope)
      : type === 'package-test.start'
        ? validatePackageTestStartFrameHeader(envelope)
      : type === 'request.cancel'
        ? validateRequestCancel(envelope)
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
      PROTOCOL_IDENTITY_PATTERN,
      'skiff-protocol-v1:sha256:<64 lowercase hex>'
    ) ??
    requireNonEmptyStringArray(envelope, 'runtime.register', 'targets') ??
    optionalString(envelope, 'runtime.register', 'protocolVersion') ??
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

function validateRuntimeRpcRequestBase(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  return (
    validateRuntimeRpcBase(envelope, envelopeType) ??
    requireString(envelope, envelopeType, 'runtimeId')
  );
}

function validateActorPutRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'actor.put.request') ??
    validateActorKey(envelope, 'actor.put.request', 'actorKey', false) ??
    requireString(envelope, 'actor.put.request', 'objectSchemaIdentity') ??
    requireString(envelope, 'actor.put.request', 'objectEncodingVersion')
  );
}

function validateActorPutResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'actor.put.response') ??
    validateActorKey(envelope, 'actor.put.response', 'actorRef', true) ??
    optionalPositiveInteger(envelope, 'actor.put.response', 'actorRef.epoch')
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
      PROTOCOL_IDENTITY_PATTERN,
      'skiff-protocol-v1:sha256:<64 lowercase hex>'
    ) ??
    requireString(envelope, 'spawn.submit.request', 'target') ??
    forbiddenField(envelope, 'spawn.submit.request', 'actorRef') ??
    forbiddenField(envelope, 'spawn.submit.request', 'methodName') ??
    optionalString(envelope, 'spawn.submit.request', 'spawnId') ??
    optionalStringPattern(
      envelope,
      'spawn.submit.request',
      'buildId',
      SERVICE_OR_PACKAGE_TEST_BUILD_ID_PATTERN,
      'skiff-service-build-v1:sha256:<64 lowercase hex> or skiff-package-test-build-v1:sha256:<64 lowercase hex>'
    ) ??
    optionalStringPattern(
      envelope,
      'spawn.submit.request',
      'activationIdentity',
      ACTIVATION_IDENTITY_PATTERN,
      'skiff-runtime-activation-v1:opaque:<opaque id>'
    ) ??
    optionalString(envelope, 'spawn.submit.request', 'callerRequestId') ??
    optionalString(envelope, 'spawn.submit.request', 'traceId') ??
    optionalString(envelope, 'spawn.submit.request', 'callerTarget') ??
    optionalPositiveNumber(envelope, 'spawn.submit.request', 'maxQueueWaitMs')
  );
}

function validateSpawnSubmitResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'spawn.submit.response') ??
    requireString(envelope, 'spawn.submit.response', 'spawnId') ??
    requireString(envelope, 'spawn.submit.response', 'itemId') ??
    requireEnum(envelope, 'spawn.submit.response', 'status', ['submitted'])
  );
}

function validateSpawnClaimRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'spawn.claim.request') ??
    requireString(envelope, 'spawn.claim.request', 'workerId') ??
    requirePublicationId(envelope, 'spawn.claim.request', 'serviceId') ??
    requireString(envelope, 'spawn.claim.request', 'serviceVersion') ??
    requireStringPattern(
      envelope,
      'spawn.claim.request',
      'serviceProtocolIdentity',
      PROTOCOL_IDENTITY_PATTERN,
      'skiff-protocol-v1:sha256:<64 lowercase hex>'
    ) ??
    requireNonEmptyStringArray(envelope, 'spawn.claim.request', 'supportedTargets') ??
    requireNonEmptyStringArray(
      envelope,
      'spawn.claim.request',
      'supportedSpawnCompatibilityKeys'
    ) ??
    optionalStringPattern(
      envelope,
      'spawn.claim.request',
      'buildId',
      SERVICE_OR_PACKAGE_TEST_BUILD_ID_PATTERN,
      'skiff-service-build-v1:sha256:<64 lowercase hex> or skiff-package-test-build-v1:sha256:<64 lowercase hex>'
    ) ??
    optionalPositiveNumber(envelope, 'spawn.claim.request', 'maxExecutionMs') ??
    optionalPositiveNumber(envelope, 'spawn.claim.request', 'maxConcurrency')
  );
}

function validateSpawnClaimResponse(envelope: Record<string, unknown>): string | null {
  const baseError =
    validateRuntimeRpcBase(envelope, 'spawn.claim.response') ??
    requireBoolean(envelope, 'spawn.claim.response', 'claimed');
  if (baseError) {
    return baseError;
  }
  if (envelope.item === undefined) {
    return envelope.claimed === true
      ? 'invalid spawn.claim.response envelope: item must be an object when claimed is true'
      : null;
  }
  if (!isRecord(envelope.item)) {
    return 'invalid spawn.claim.response envelope: item must be an object';
  }
  return (
    requireString(envelope, 'spawn.claim.response', 'item.itemId') ??
    requireString(envelope, 'spawn.claim.response', 'item.leaseId') ??
    requireString(envelope, 'spawn.claim.response', 'item.spawnExecutionId') ??
    requireString(envelope, 'spawn.claim.response', 'item.runtimeRequestId') ??
    requireString(envelope, 'spawn.claim.response', 'item.spawnId') ??
    requireEnum(envelope, 'spawn.claim.response', 'item.targetKind', spawnTargetKinds) ??
    requireString(envelope, 'spawn.claim.response', 'item.target') ??
    requirePublicationId(envelope, 'spawn.claim.response', 'item.serviceId') ??
    requireString(envelope, 'spawn.claim.response', 'item.serviceVersion') ??
    requireStringPattern(
      envelope,
      'spawn.claim.response',
      'item.serviceProtocolIdentity',
      PROTOCOL_IDENTITY_PATTERN,
      'skiff-protocol-v1:sha256:<64 lowercase hex>'
    ) ??
    requireStringPattern(
      envelope,
      'spawn.claim.response',
      'item.buildId',
      SERVICE_OR_PACKAGE_TEST_BUILD_ID_PATTERN,
      'skiff-service-build-v1:sha256:<64 lowercase hex> or skiff-package-test-build-v1:sha256:<64 lowercase hex>'
    ) ??
    optionalString(envelope, 'spawn.claim.response', 'item.payloadSchemaIdentity') ??
    optionalString(envelope, 'spawn.claim.response', 'item.leaseExpiresAt')
  );
}

function validateSpawnRenewRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'spawn.renew.request') ??
    requireString(envelope, 'spawn.renew.request', 'itemId') ??
    requireString(envelope, 'spawn.renew.request', 'leaseId') ??
    requireString(envelope, 'spawn.renew.request', 'workerId')
  );
}

function validateSpawnRenewResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'spawn.renew.response') ??
    requireString(envelope, 'spawn.renew.response', 'itemId') ??
    requireBoolean(envelope, 'spawn.renew.response', 'renewed') ??
    optionalString(envelope, 'spawn.renew.response', 'leaseExpiresAt')
  );
}

function validateSpawnCompleteRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'spawn.complete.request') ??
    requireString(envelope, 'spawn.complete.request', 'itemId') ??
    requireString(envelope, 'spawn.complete.request', 'leaseId') ??
    optionalRecord(envelope, 'spawn.complete.request', 'diagnostics')
  );
}

function validateSpawnCompleteResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'spawn.complete.response') ??
    requireString(envelope, 'spawn.complete.response', 'itemId') ??
    requireEnum(envelope, 'spawn.complete.response', 'status', ['completed'])
  );
}

function validateSpawnFailRequest(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcRequestBase(envelope, 'spawn.fail.request') ??
    requireString(envelope, 'spawn.fail.request', 'itemId') ??
    requireString(envelope, 'spawn.fail.request', 'leaseId') ??
    requireEnum(envelope, 'spawn.fail.request', 'reason', spawnFailReasons) ??
    optionalRecord(envelope, 'spawn.fail.request', 'diagnostics')
  );
}

function validateSpawnFailResponse(envelope: Record<string, unknown>): string | null {
  return (
    validateRuntimeRpcBase(envelope, 'spawn.fail.response') ??
    requireString(envelope, 'spawn.fail.response', 'itemId') ??
    requireEnum(envelope, 'spawn.fail.response', 'status', spawnFailReasons)
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

function validateRequestStartFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'request.start') ??
    requireString(envelope, 'request.start', 'requestId') ??
    requireEnum(envelope, 'request.start', 'mode', ['unary', 'serverStream']) ??
    validateCaller(envelope) ??
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
    requirePattern(
      envelope,
      'request.start',
      'serviceProtocolIdentity',
      PROTOCOL_IDENTITY_PATTERN,
      'skiff-protocol-v1:sha256:<64 lowercase hex>'
    ) ??
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
    validateTestEffectDoubles(envelope, 'request.start')
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
    requireString(envelope, 'response.chunk', 'requestId') ??
    requireInteger(envelope, 'response.chunk', 'seq')
  );
}

function validateResponseStartFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'response.start') ??
    requireString(envelope, 'response.start', 'requestId') ??
    validateHttpResponseFrameMetadata(envelope, 'response.start')
  );
}

function validateResponseEndFrameHeader(envelope: Record<string, unknown>): string | null {
  return (
    rejectHeaderPayloadFields(envelope, 'response.end') ??
    requireString(envelope, 'response.end', 'requestId') ??
    requireBoolean(envelope, 'response.end', 'payloadPresent') ??
    validateHttpResponseFrameMetadata(envelope, 'response.end') ??
    validateWebSocketConnectResponseFrameMetadata(envelope)
  );
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
  const websocketContextError =
    requireString(envelope, 'request.start', 'websocketEntryId') ??
    requireString(envelope, 'request.start', 'gatewayEntryIdentity');
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
  const baseError =
    requireEnum(envelope, 'response.end', 'websocketConnect.result', ['accept', 'reject']) ??
    requireBoolean(envelope, 'response.end', 'websocketConnect.contextPayloadPresent') ??
    optionalString(envelope, 'response.end', 'websocketConnect.businessIdentity') ??
    optionalPositiveInteger(envelope, 'response.end', 'websocketConnect.code') ??
    optionalString(envelope, 'response.end', 'websocketConnect.reason') ??
    validateOptionalContextCodec(metadata.contextCodec, 'websocketConnect.contextCodec') ??
    validateWebSocketConnectionPolicy(metadata.connectionPolicy);
  if (baseError) {
    return baseError;
  }
  if (metadata.contextPayloadPresent === true && metadata.contextCodec === undefined) {
    return 'invalid response.end envelope: websocketConnect.contextCodec is required when contextPayloadPresent is true';
  }
  if (metadata.contextPayloadPresent === false && metadata.contextCodec !== undefined) {
    return 'invalid response.end envelope: websocketConnect.contextCodec is not supported when contextPayloadPresent is false';
  }
  return null;
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
  const policy = value as Record<string, unknown>;
  if (!Number.isInteger(policy.maxConnections) || Number(policy.maxConnections) <= 0) {
    return 'invalid response.end envelope: websocketConnect.connectionPolicy.maxConnections must be a positive integer';
  }
  if (policy.overflow !== 'close-oldest' && policy.overflow !== 'reject-new') {
    return 'invalid response.end envelope: websocketConnect.connectionPolicy.overflow must be one of close-oldest, reject-new';
  }
  return (
    optionalPositiveInteger(value, 'response.end', 'closeCode') ??
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
  }
  return null;
}

function validateResponseError(envelope: Record<string, unknown>): string | null {
  return validateErrorPayload(envelope, 'response.error');
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
    requireString(envelope, 'connection.send', 'serviceId') ??
    optionalString(envelope, 'connection.send', 'websocketEntryId') ??
    validateConnectionSendTarget(envelope) ??
    optionalEnum(envelope, 'connection.send', 'payloadKind', ['text', 'binary'])
  );
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
  return null;
}

function validateFrameHeaderBase(
  envelope: Record<string, unknown>,
  envelopeType: string
): string | null {
  return requireEnum(envelope, `${envelopeType} frame header`, 'schemaVersion', [
    RUNTIME_FRAME_SCHEMA_VERSION
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
