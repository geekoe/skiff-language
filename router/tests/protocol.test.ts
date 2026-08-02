import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  decodeRuntimeFrame,
  encodeBinaryFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION,
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS
} from '../src/protocol/envelope.js';
import { decodeRuntimePayload, encodeRuntimePayload } from './helpers/runtimePayloadCodec.js';
import {
  runtimeFrameHeaderFixtures,
  runtimeFrameHeaderSchemas,
  validateResponseErrorFrame,
  validateRouterToRuntimeFrameHeader,
  validateTelemetryEvent,
  validateRuntimeToRouterFrameHeader,
  type ProtocolEnvelopeObjectSchema,
  type ProtocolEnvelopeSchema,
  type ProtocolSchemaProperty,
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
  'actor.getOrCreate.request',
  'actor.getOrCreate.response',
  'actor.getOrCreate.error',
  'actor.replace.request',
  'actor.replace.response',
  'actor.replace.error',
  'actor.find.request',
  'actor.find.response',
  'actor.find.error',
  'actor.remove.request',
  'actor.remove.response',
  'actor.remove.error',
  'spawn.submit.request',
  'spawn.submit.response',
  'spawn.submit.error',
  'request.start',
  'package-test.start',
  'router.bootstrap',
  'router.control',
  'runtime.registered',
  'response.start',
  'response.end',
  'response.error',
  'response.chunk',
  'request.cancel',
  'connection.send',
  'connection.request',
  'connection.request.cancel',
  'connection.response'
] as const satisfies readonly RuntimeProtocolFrameHeaderName[];

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
  'response.start',
  'response.end',
  'response.error',
  'response.chunk',
  'request.cancel',
  'connection.send',
  'connection.request',
  'connection.request.cancel'
] as const satisfies readonly RuntimeToRouterFrameHeaderName[];

const routerToRuntimeFrameHeaderTypes = [
  'runtime.registered',
  'router.bootstrap',
  'router.control',
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
  'response.end',
  'response.error',
  'response.chunk'
] as const satisfies readonly RouterToRuntimeFrameHeaderName[];

function protocolEnvelopeSchemaBranches(
  schema: ProtocolEnvelopeSchema
): readonly ProtocolEnvelopeObjectSchema[] {
  return 'oneOf' in schema ? schema.oneOf : [schema];
}

function matchesProtocolEnvelopeSchema(
  schema: ProtocolEnvelopeSchema,
  value: unknown
): boolean {
  const matchingBranches = protocolEnvelopeSchemaBranches(schema).filter((branch) =>
    matchesProtocolObjectShape(
      branch.required,
      branch.properties,
      branch.additionalProperties,
      value
    )
  );
  return matchingBranches.length === 1;
}

function matchesProtocolSchemaProperty(
  schema: ProtocolSchemaProperty,
  value: unknown
): boolean {
  const types: readonly string[] =
    typeof schema.type === 'string' ? [schema.type] : schema.type;
  if (!types.some((type) => matchesProtocolSchemaType(type, value))) {
    return false;
  }
  if (
    schema.enum !== undefined &&
    !schema.enum.some((candidate) => Object.is(candidate, value))
  ) {
    return false;
  }
  if (typeof value === 'string') {
    if (schema.minLength !== undefined && value.length < schema.minLength) {
      return false;
    }
    if (schema.pattern !== undefined && !new RegExp(schema.pattern).test(value)) {
      return false;
    }
  }
  if (typeof value === 'number') {
    if (schema.minimum !== undefined && value < schema.minimum) {
      return false;
    }
    if (schema.maximum !== undefined && value > schema.maximum) {
      return false;
    }
  }
  if (types.includes('object') && isProtocolObject(value)) {
    return matchesProtocolObjectShape(
      schema.required ?? [],
      schema.properties ?? {},
      schema.additionalProperties ?? true,
      value
    );
  }
  const itemSchema = schema.items;
  if (types.includes('array') && itemSchema !== undefined && Array.isArray(value)) {
    return value.every((item) => matchesProtocolSchemaProperty(itemSchema, item));
  }
  return true;
}

function matchesProtocolSchemaType(type: string, value: unknown): boolean {
  switch (type) {
    case 'any':
      return true;
    case 'null':
      return value === null;
    case 'string':
      return typeof value === 'string';
    case 'number':
      return typeof value === 'number' && Number.isFinite(value);
    case 'integer':
      return typeof value === 'number' && Number.isInteger(value);
    case 'boolean':
      return typeof value === 'boolean';
    case 'array':
      return Array.isArray(value);
    case 'object':
      return isProtocolObject(value);
    default:
      return false;
  }
}

function matchesProtocolObjectShape(
  required: readonly string[],
  properties: Record<string, ProtocolSchemaProperty>,
  additionalProperties: boolean,
  value: unknown
): boolean {
  if (!isProtocolObject(value)) {
    return false;
  }
  if (
    required.some(
      (field) => !Object.prototype.hasOwnProperty.call(value, field)
    )
  ) {
    return false;
  }
  for (const [field, fieldValue] of Object.entries(value)) {
    const property = properties[field];
    if (property === undefined) {
      if (!additionalProperties) {
        return false;
      }
      continue;
    }
    if (!matchesProtocolSchemaProperty(property, fieldValue)) {
      return false;
    }
  }
  return true;
}

function isProtocolObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

const actorControlActivationIdentityCorpus = JSON.parse(
  readFileSync(
    new URL(
      '../../runtime/transport/testdata/actor-control-activation-identity.json',
      import.meta.url
    ),
    'utf8'
  )
) as {
  valid: Record<string, unknown>;
  invalid: Array<{ label: string; value: unknown }>;
};

const serviceErrorResponseV2Corpus = JSON.parse(
  readFileSync(
    new URL(
      '../../runtime/transport/testdata/service-error-response-v2.json',
      import.meta.url
    ),
    'utf8'
  )
) as {
  validCases: Array<{
    name: string;
    header: Record<string, unknown>;
    payloadUtf8: string;
    expected: Record<string, unknown>;
  }>;
  invalidCases: Array<{
    name: string;
    header: Record<string, unknown>;
    payloadUtf8: string;
  }>;
};

const runtimeAssemblyRequestCorpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as {
  requestStartHeaders: Array<Record<string, unknown>>;
  legacyRequestStartHeaders: Array<Record<string, unknown>>;
};

const runtimeWebSocketConnectWireCorpus = JSON.parse(
  readFileSync(
    new URL(
      '../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json',
      import.meta.url
    ),
    'utf8'
  )
) as {
  requestCases: Array<{
    name: string;
    header: Record<string, unknown>;
  }>;
  responseCases: Array<{
    name: string;
    header: Record<string, unknown>;
  }>;
};

const serviceErrorResponseV2HeaderInvalidCaseNames = new Set([
  'legacy-v1-generic-response-error',
  'missing-error-kind',
  'unknown-error-kind',
  'wrong-envelope-type',
  'fixed-header-extra-field',
  'fixed-missing-request-id',
  'fixed-empty-request-id',
  'fixed-carries-generic-error',
  'control-missing-error',
  'control-error-extra-field',
  'control-empty-code',
  'control-empty-message',
  'control-invalid-status-low'
]);

const serviceErrorResponseV2PayloadOnlyInvalidCaseNames = new Set([
  'fixed-empty-payload',
  'control-nonempty-payload',
  'fixed-malformed-json',
  'fixed-unknown-envelope-kind',
  'fixed-envelope-extra-field',
  'fixed-public-missing-type-id',
  'fixed-public-whitespace-package-id',
  'fixed-public-whitespace-schema-key',
  'fixed-public-whitespace-type-id',
  'fixed-public-empty-encoded-payload',
  'fixed-public-non-byte-encoded-payload',
  'fixed-public-encoded-payload-not-array',
  'fixed-public-whitespace-trace-id',
  'fixed-public-empty-error-id',
  'fixed-platform-unknown-identity',
  'fixed-internal-payload-extra-field',
  'fixed-internal-empty-message'
]);

const observabilityFixture = JSON.parse(
  readFileSync(
    new URL('../../doc/architecture/fixtures/observability-minimal.json', import.meta.url),
    'utf8'
  )
) as {
  valid: {
    batch: {
      events: Array<Record<string, unknown>>;
    };
  };
  invalidCases: Array<{
    name: string;
    payload: {
      events?: Array<Record<string, unknown>>;
    };
  }>;
};

describe('runtime protocol fixtures and schemas', () => {
  it('registers disjoint legacy, production/root/derived HTTP, websocketConnect, websocketJsonRpc, and spawn request.start branches', () => {
    const schema = runtimeFrameHeaderSchemas['request.start'];
    expect('oneOf' in schema).toBe(true);
    if (!('oneOf' in schema)) throw new Error('request.start schema must be oneOf');
    expect(schema.oneOf).toHaveLength(7);
    expect(runtimeAssemblyRequestCorpus.requestStartHeaders.length).toBeGreaterThan(0);
    expect(runtimeAssemblyRequestCorpus.legacyRequestStartHeaders.length).toBeGreaterThan(0);
    expect(runtimeWebSocketConnectWireCorpus.requestCases).toHaveLength(3);

    for (const header of runtimeAssemblyRequestCorpus.requestStartHeaders) {
      expect(matchesProtocolEnvelopeSchema(schema, header), String(header.requestId)).toBe(
        true
      );
    }
    for (const testCase of runtimeWebSocketConnectWireCorpus.requestCases) {
      const staleTestFlag = testCase.header.testEffectsEnabled === true;
      expect(
        matchesProtocolEnvelopeSchema(schema, testCase.header),
        testCase.name
      ).toBe(!staleTestFlag);
    }
    const websocketJsonRpc = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId: 'request-websocket-jsonrpc-schema',
      mode: 'unary',
      caller: { kind: 'gateway' },
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity:
          'skiff-runtime-assembly-v3:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
        assemblyGeneration: 11,
        deployment: {
          serviceId: 'example.com/chat',
          contractVersion: '1.0.0',
          deploymentRevision: 'chat-current',
          deploymentArtifactIdentity:
            'skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
        },
        gatewayEntryIdentity:
          'skiff-gateway-entry-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
        ingress: {
          protocol: 'webSocket',
          method: 'status.get',
          path: '/chat'
        }
      },
      trace: {
        traceId: 'trace-websocket-jsonrpc',
        spanId: 'span-websocket-jsonrpc'
      },
      websocketJsonRpc: {
        profile: 'jsonrpc-2.0-text',
        connectionId: 'connection-1',
        websocketEntryId:
          'skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616',
        gatewayEntryIdentity:
          'skiff-gateway-entry-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
        businessIdentity: 'tenant-1'
      },
      testEffectsEnabled: false
    };
    expect(matchesProtocolEnvelopeSchema(schema, websocketJsonRpc)).toBe(true);
    expect(
      matchesProtocolEnvelopeSchema(schema, {
        ...websocketJsonRpc,
        testEffectsEnabled: true
      })
    ).toBe(false);
    expect(
      validateRouterToRuntimeFrameHeader({
        ...websocketJsonRpc,
        testEffectsEnabled: true
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start runtimeAssembly envelope: websocketJsonRpc testEffectsEnabled must be false'
    });
    expect(
      validateRouterToRuntimeFrameHeader({
        ...websocketJsonRpc,
        testCaseCapability: 'forbidden-websocket-capability'
      })
    ).toMatchObject({
      ok: false,
      error: expect.stringContaining('testCaseCapability is not supported')
    });
    const spawn = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId: 'request-derived-spawn-schema',
      mode: 'unary',
      caller: { kind: 'service' },
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity:
          'skiff-runtime-assembly-v3:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
        assemblyGeneration: 11,
        deployment: {
          serviceId: 'example.com/chat',
          contractVersion: '1.0.0',
          deploymentRevision: 'chat-current',
          deploymentArtifactIdentity:
            'skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'
        }
      },
      invocation: {
        kind: 'spawn',
        targetKind: 'function',
        target: 'package.chat.run'
      },
      trace: {
        traceId: 'trace-derived-spawn',
        spanId: 'span-derived-spawn'
      },
      testEffectsEnabled: true,
      testCaseCapability: 'opaque-test-case-capability'
    };
    expect(matchesProtocolEnvelopeSchema(schema, spawn)).toBe(true);
    expect(validateRouterToRuntimeFrameHeader(spawn)).toMatchObject({ ok: true });
    expect(
      validateRouterToRuntimeFrameHeader({
        ...spawn,
        testEffectsEnabled: false
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start runtimeAssembly envelope: testCaseCapability requires testEffectsEnabled true'
    });
    const { testCaseCapability: _omittedCapability, ...spawnWithoutCapability } =
      spawn;
    expect(
      validateRouterToRuntimeFrameHeader(spawnWithoutCapability)
    ).toEqual({
      ok: false,
      error:
        'invalid request.start runtimeAssembly envelope: testEffectsEnabled true requires testCaseCapability'
    });

    const productionHttp =
      runtimeAssemblyRequestCorpus.requestStartHeaders[1]!;
    expect(
      validateRouterToRuntimeFrameHeader({
        ...productionHttp,
        testEffectsEnabled: true
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start runtimeAssembly envelope: testEffectsEnabled true requires testCaseCapability'
    });
    const rootTestHttp = {
      ...productionHttp,
      testEffectsEnabled: true,
      testCaseCapability: 'http-test-capability'
    };
    expect(validateRouterToRuntimeFrameHeader(rootTestHttp)).toMatchObject({
      ok: true
    });
    expect(matchesProtocolEnvelopeSchema(schema, rootTestHttp)).toBe(true);
    const derivedHttp = {
      ...productionHttp,
      testEffectsEnabled: true,
      testCaseCapability: 'http-test-capability',
      testCaseParentRequestId: 'parent-request:1'
    };
    expect(validateRouterToRuntimeFrameHeader(derivedHttp)).toEqual({
      ok: true,
      envelope: derivedHttp
    });
    expect(matchesProtocolEnvelopeSchema(schema, derivedHttp)).toBe(true);
    for (const { invalid, schemaMatches } of [
      {
        invalid: { testCaseParentRequestId: 'parent-request:1' },
        schemaMatches: false
      },
      { invalid: {
        testEffectsEnabled: true,
        testCaseCapability: 'http-test-capability',
        testCaseParentRequestId: 'contains/slash'
      }, schemaMatches: false },
      { invalid: {
        testEffectsEnabled: true,
        testCaseCapability: 'z'.repeat(257)
      }, schemaMatches: false }
    ]) {
      const invalidHttp = { ...productionHttp, ...invalid };
      expect(validateRouterToRuntimeFrameHeader(invalidHttp).ok).toBe(false);
      expect(matchesProtocolEnvelopeSchema(schema, invalidHttp)).toBe(schemaMatches);
    }

    const staleHttpRouting = structuredClone(
      runtimeAssemblyRequestCorpus.requestStartHeaders[0]!
    );
    (staleHttpRouting.routing as Record<string, unknown>).gatewayEntryIdentity =
      'skiff-gateway-entry-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    expect(matchesProtocolEnvelopeSchema(schema, staleHttpRouting)).toBe(false);

    const staleConnectRouting = structuredClone(
      runtimeWebSocketConnectWireCorpus.requestCases[1]!.header
    );
    (staleConnectRouting.routing as Record<string, unknown>).gatewayEntryIdentity =
      'skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936';
    expect(matchesProtocolEnvelopeSchema(schema, staleConnectRouting)).toBe(false);

    const staleConnectMetadata = structuredClone(
      runtimeWebSocketConnectWireCorpus.requestCases[1]!.header
    );
    (
      staleConnectMetadata.websocketConnect as Record<string, unknown>
    ).gatewayEntryIdentity =
      'skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936';
    expect(matchesProtocolEnvelopeSchema(schema, staleConnectMetadata)).toBe(false);

    const staleJsonRpcRouting = structuredClone(websocketJsonRpc);
    (
      staleJsonRpcRouting.routing as Record<string, unknown>
    ).gatewayEntryIdentity =
      'skiff-gateway-entry-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';
    expect(matchesProtocolEnvelopeSchema(schema, staleJsonRpcRouting)).toBe(false);

    const staleJsonRpcMetadata = structuredClone(websocketJsonRpc);
    (
      staleJsonRpcMetadata.websocketJsonRpc as Record<string, unknown>
    ).gatewayEntryIdentity =
      'skiff-gateway-entry-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';
    expect(matchesProtocolEnvelopeSchema(schema, staleJsonRpcMetadata)).toBe(false);

    for (const header of runtimeAssemblyRequestCorpus.legacyRequestStartHeaders) {
      expect(matchesProtocolEnvelopeSchema(schema, header), String(header.requestId)).toBe(
        true
      );
    }

    const forgedCaller = structuredClone(runtimeAssemblyRequestCorpus.requestStartHeaders[0]!);
    (forgedCaller.caller as Record<string, unknown>).target = 'forged-handler';
    expect(matchesProtocolEnvelopeSchema(schema, forgedCaller)).toBe(false);

    const websocket = structuredClone(runtimeAssemblyRequestCorpus.requestStartHeaders[0]!);
    const routing = websocket.routing as Record<string, unknown>;
    (routing.ingress as Record<string, unknown>).protocol = 'webSocket';
    expect(matchesProtocolEnvelopeSchema(schema, websocket)).toBe(false);
  });

  it('matches websocketConnect and websocketJsonRpc response branches declaratively', () => {
    const schema = runtimeFrameHeaderSchemas['response.end'];
    expect('oneOf' in schema).toBe(true);
    if (!('oneOf' in schema)) throw new Error('response.end schema must be oneOf');
    expect(schema.oneOf).toHaveLength(6);
    expect(runtimeWebSocketConnectWireCorpus.responseCases).toHaveLength(3);

    for (const testCase of runtimeWebSocketConnectWireCorpus.responseCases) {
      expect(matchesProtocolEnvelopeSchema(schema, testCase.header), testCase.name).toBe(
        true
      );
    }
    expect(
      matchesProtocolEnvelopeSchema(schema, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'request-websocket-jsonrpc-schema',
        payloadPresent: true,
        websocketJsonRpc: { outcome: 'success' }
      })
    ).toBe(true);
    expect(
      matchesProtocolEnvelopeSchema(schema, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'request-websocket-jsonrpc-schema',
        payloadPresent: false,
        websocketJsonRpc: { outcome: 'deadlineExceeded' }
      })
    ).toBe(true);
    expect(
      matchesProtocolEnvelopeSchema(schema, {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'request-websocket-jsonrpc-schema',
        payloadPresent: false,
        websocketJsonRpc: { outcome: 'cancelled' }
      })
    ).toBe(false);
  });

  it('evaluates the response.error declarative oneOf against the shared header corpus', () => {
    const schema = runtimeFrameHeaderSchemas['response.error'];
    expect(schema.oneOf).toHaveLength(2);
    expect(serviceErrorResponseV2HeaderInvalidCaseNames.size).toBe(13);
    expect(serviceErrorResponseV2PayloadOnlyInvalidCaseNames.size).toBe(17);
    expect(
      [
        ...serviceErrorResponseV2HeaderInvalidCaseNames,
        ...serviceErrorResponseV2PayloadOnlyInvalidCaseNames
      ].sort()
    ).toEqual(serviceErrorResponseV2Corpus.invalidCases.map(({ name }) => name).sort());

    for (const testCase of serviceErrorResponseV2Corpus.validCases) {
      expect(matchesProtocolEnvelopeSchema(schema, testCase.header), testCase.name).toBe(
        true
      );
    }

    for (const testCase of serviceErrorResponseV2Corpus.invalidCases) {
      if (serviceErrorResponseV2HeaderInvalidCaseNames.has(testCase.name)) {
        expect(matchesProtocolEnvelopeSchema(schema, testCase.header), testCase.name).toBe(
          false
        );
        continue;
      }
      expect(
        serviceErrorResponseV2PayloadOnlyInvalidCaseNames.has(testCase.name),
        testCase.name
      ).toBe(true);
      expect(matchesProtocolEnvelopeSchema(schema, testCase.header), testCase.name).toBe(
        true
      );
      expect(
        validateResponseErrorFrame(
          testCase.header,
          Buffer.from(testCase.payloadUtf8, 'utf8')
        ).ok,
        testCase.name
      ).toBe(false);
    }
  });

  it('validates the shared service_error_response_v2 corpus without changing payload bytes', () => {
    expect(serviceErrorResponseV2Corpus.validCases).toHaveLength(4);
    expect(serviceErrorResponseV2Corpus.invalidCases.length).toBeGreaterThanOrEqual(20);

    for (const testCase of serviceErrorResponseV2Corpus.validCases) {
      const payloadBytes = Buffer.from(testCase.payloadUtf8, 'utf8');
      const result = validateResponseErrorFrame(testCase.header, payloadBytes);
      expect(result.ok, testCase.name).toBe(true);
      if (!result.ok) {
        continue;
      }
      expect(result.envelope.payloadBytes, testCase.name).toBe(payloadBytes);
      expect(result.envelope.header.errorKind, testCase.name).toBe(
        testCase.header.errorKind
      );
      if ('serviceError' in result.envelope) {
        expect(result.envelope.serviceError.kind, testCase.name).toBe(testCase.expected.kind);
        if (result.envelope.serviceError.kind === 'internalError') {
          expect(result.envelope.serviceError.payload.traceId, testCase.name).toBe(
            testCase.expected.traceId
          );
          expect(result.envelope.serviceError.payload.errorId, testCase.name).toBe(
            testCase.expected.errorId
          );
        } else {
          expect(result.envelope.serviceError.traceId, testCase.name).toBe(
            testCase.expected.traceId
          );
          expect(result.envelope.serviceError.errorId, testCase.name).toBe(
            testCase.expected.errorId
          );
          if (result.envelope.serviceError.kind === 'publicTypedError') {
            expect(result.envelope.serviceError.packageId, testCase.name).toBe(
              testCase.expected.packageId
            );
            expect(result.envelope.serviceError.stableSchemaKey, testCase.name).toBe(
              testCase.expected.stableSchemaKey
            );
            expect(result.envelope.serviceError.packageSchemaTypeId, testCase.name).toBe(
              testCase.expected.packageSchemaTypeId
            );
          } else {
            expect(result.envelope.serviceError.builtinErrorIdentity, testCase.name).toBe(
              testCase.expected.builtinErrorIdentity
            );
          }
        }
      } else {
        expect(result.envelope.header.errorKind, testCase.name).toBe('control');
      }
    }

    for (const testCase of serviceErrorResponseV2Corpus.invalidCases) {
      expect(
        validateResponseErrorFrame(
          testCase.header,
          Buffer.from(testCase.payloadUtf8, 'utf8')
        ).ok,
        testCase.name
      ).toBe(false);
    }
  });

  it('rejects legacy cancellation from both ordinary response.error channels', () => {
    const fixedResult = validateResponseErrorFrame(
      {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: 'legacy-fixed-cancel',
        errorKind: 'fixedService'
      },
      Buffer.from(
        JSON.stringify({
          kind: 'platformError',
          builtinErrorIdentity: 'CancelError',
          encodedPayload: [1],
          traceId: 'trace-legacy-fixed-cancel',
          errorId: 'error-legacy-fixed-cancel'
        }),
        'utf8'
      )
    );
    const controlResult = validateResponseErrorFrame(
      {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: 'legacy-control-cancel',
        errorKind: 'control',
        error: {
          code: 'CancelError',
          message: 'legacy cancellation must not become an ordinary response'
        }
      },
      new Uint8Array()
    );

    expect.soft(fixedResult.ok).toBe(false);
    expect.soft(controlResult.ok).toBe(false);
    if (!fixedResult.ok) {
      expect(fixedResult.error).toContain('builtinErrorIdentity is not supported');
    }
    if (!controlResult.ok) {
      expect(controlResult.error).toContain(
        'error.code is reserved for internal cancellation'
      );
    }
  });

  it('validates telemetry visibility and correlation against the shared fixture', () => {
    for (const event of observabilityFixture.valid.batch.events) {
      expect(validateTelemetryEvent(event)).toEqual({ ok: true, envelope: event });
    }
    const invalidEvents = observabilityFixture.invalidCases
      .filter((testCase) => testCase.name.startsWith('telemetry-batch-'))
      .flatMap((testCase) => testCase.payload.events ?? []);
    expect(invalidEvents).toHaveLength(8);
    for (const event of invalidEvents) {
      expect(validateTelemetryEvent(event).ok).toBe(false);
    }
  });

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
        schemaVersion: 'skiff-runtime-frame-v3',
        type: 'response.end'
      })
    ).toEqual({
      ok: false,
      error: 'invalid response.end envelope: requestId must be a string'
    });
    expect(
      validateRuntimeToRouterFrameHeader({
        schemaVersion: 'skiff-runtime-frame-v3',
        type: 'response.error',
        requestId: 'request-1',
        errorKind: 'control',
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
        'invalid runtime frame header envelope: type must be one of runtime.register, runtime.capabilities, runtime.health, actor.getOrCreate.request, actor.replace.request, actor.find.request, actor.remove.request, spawn.submit.request, request.start, request.cancel, connection.send, connection.request, connection.request.cancel, response.start, response.chunk, response.end, response.error'
    });
  });

  it('hard-cuts the ambiguous actor.put wire operation', () => {
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['actor.getOrCreate.request'],
        type: 'actor.put.request'
      })
    ).toMatchObject({ ok: false });
  });

  it('requires a strict capability-parent token pair on actor getOrCreate only', () => {
    const base = runtimeFrameHeaderFixtures['actor.getOrCreate.request'];
    const schema = runtimeFrameHeaderSchemas['actor.getOrCreate.request'];
    const validTokens = ['a', 'case:opaque_1.parent-2', 'z'.repeat(256)];
    for (const token of validTokens) {
      const header = {
        ...base,
        testCaseCapability: token,
        testCaseParentRequestId: token
      };
      expect(validateRuntimeToRouterFrameHeader(header)).toEqual({
        ok: true,
        envelope: header
      });
      expect(matchesProtocolEnvelopeSchema(schema, header)).toBe(true);
    }

    const invalidTokens: unknown[] = [
      '',
      'z'.repeat(257),
      'contains/slash',
      'contains space',
      'contains~tilde',
      '非ascii',
      1
    ];
    for (const token of invalidTokens) {
      for (const field of [
        'testCaseCapability',
        'testCaseParentRequestId'
      ] as const) {
        const header = {
          ...base,
          testCaseCapability: 'case-capability',
          testCaseParentRequestId: 'parent-request',
          [field]: token
        };
        expect(validateRuntimeToRouterFrameHeader(header).ok).toBe(false);
        expect(matchesProtocolEnvelopeSchema(schema, header)).toBe(false);
      }
    }

    for (const halfPair of [
      { testCaseCapability: 'case-capability' },
      { testCaseParentRequestId: 'parent-request' }
    ]) {
      const header = { ...base, ...halfPair };
      expect(validateRuntimeToRouterFrameHeader(header).ok).toBe(false);
      expect(matchesProtocolEnvelopeSchema(schema, header)).toBe(false);
    }
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['actor.replace.request'],
        testCaseCapability: 'case-capability',
        testCaseParentRequestId: 'parent-request'
      }).ok
    ).toBe(false);
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

  it('accepts exact current ServiceProtocolIdentity v5 and rejects legacy v4/v3 registration', () => {
    const currentRegistration = {
      ...runtimeFrameHeaderFixtures['runtime.register'],
      serviceProtocolIdentity:
        'skiff-service-protocol-v5:sha256:2222222222222222222222222222222222222222222222222222222222222222'
    };
    expect(validateRuntimeToRouterFrameHeader(currentRegistration)).toEqual({
      ok: true,
      envelope: currentRegistration
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        serviceProtocolIdentity:
          'skiff-service-protocol-v3:sha256:2222222222222222222222222222222222222222222222222222222222222222'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: serviceProtocolIdentity must be skiff-service-protocol-v5:sha256:<64 lowercase hex>'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        serviceProtocolIdentity: 'skiff-protocol-v1:sha256:not-a-real-hash'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: serviceProtocolIdentity must be skiff-service-protocol-v5:sha256:<64 lowercase hex>'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        serviceProtocolIdentity:
          'skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid runtime.register envelope: serviceProtocolIdentity must be skiff-service-protocol-v5:sha256:<64 lowercase hex>'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['runtime.register'],
        protocolVersion: 'skiff-protocol-v1'
      })
    ).toEqual({
      ok: false,
      error: 'invalid runtime.register frame header envelope: protocolVersion is not supported'
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

  it('requires current ServiceProtocolIdentity v5 on retained legacy request.start', () => {
    expect(
      validateRouterToRuntimeFrameHeader({
        ...runtimeFrameHeaderFixtures['request.start'],
        serviceProtocolIdentity:
          'skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid request.start envelope: serviceProtocolIdentity must be skiff-service-protocol-v5:sha256:<64 lowercase hex>'
    });

    for (const serviceProtocolIdentity of [
      'skiff-service-protocol-v3:sha256:1111111111111111111111111111111111111111111111111111111111111111',
      'skiff-service-protocol-v4:sha256:1111111111111111111111111111111111111111111111111111111111111111',
      'skiff-service-protocol-v5:sha256:1111',
      'skiff-service-protocol-v5:sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
    ]) {
      expect(
        validateRouterToRuntimeFrameHeader({
          ...runtimeFrameHeaderFixtures['request.start'],
          serviceProtocolIdentity
        })
      ).toMatchObject({ ok: false });
    }

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

  it('requires current ServiceProtocolIdentity v5 on spawn submit', () => {
    const legacyV1 =
      'skiff-protocol-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111';
    const legacyV3 =
      'skiff-service-protocol-v3:sha256:1111111111111111111111111111111111111111111111111111111111111111';
    const legacyV4 =
      'skiff-service-protocol-v4:sha256:1111111111111111111111111111111111111111111111111111111111111111';
    const invalidV5 = [
      'skiff-service-protocol-v5:sha256:1111',
      'skiff-service-protocol-v5:sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
    ];

    for (const serviceProtocolIdentity of [legacyV1, legacyV3, legacyV4, ...invalidV5]) {
      expect(
        validateRuntimeToRouterFrameHeader({
          ...runtimeFrameHeaderFixtures['spawn.submit.request'],
          serviceProtocolIdentity
        })
      ).toMatchObject({ ok: false });
    }
  });

  it('accepts optional serviceId on router request.start frames', () => {
    const requestEnvelope = {
      ...runtimeFrameHeaderFixtures['request.start'],
      serviceId: 'example.com/hello'
    };

    const requestStartSchema = runtimeFrameHeaderSchemas['request.start'];
    expect('oneOf' in requestStartSchema).toBe(true);
    if (!('oneOf' in requestStartSchema)) throw new Error('request.start schema must be oneOf');
    expect(requestStartSchema.oneOf[0].properties.serviceId).toEqual({
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
      const schema = runtimeFrameHeaderSchemas[type];
      expect(schema).toBeDefined();
      expect(runtimeFrameHeaderFixtures[type]).toBeDefined();
      for (const branch of protocolEnvelopeSchemaBranches(schema)) {
        expect(branch.properties.type?.enum).toContain(type);
      }
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

  it('keeps spawn submit schema function/actorMethod-targeted', () => {
    const properties = runtimeFrameHeaderSchemas['spawn.submit.request'].properties;
    expect(properties.targetKind.enum).toEqual(['function', 'actorMethod']);
    expect(properties).not.toHaveProperty('actorRef');
    expect(properties).not.toHaveProperty('methodName');
    expect(properties).toHaveProperty('actorMethod');
    const actorMethodTarget = {
      actorRef: runtimeFrameHeaderFixtures['actor.getOrCreate.response'].actorRef,
      declarationOwner: runtimeFrameHeaderFixtures['actor.getOrCreate.request']
        .declarationOwner,
      actorAbiIdentity: 'skiff-actor-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      actorImplementationIdentity:
        'skiff-actor-implementation-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      methodIdentity:
        'skiff-actor-method-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
    };
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['spawn.submit.request'],
        targetKind: 'actorMethod',
        actorMethod: actorMethodTarget
      }).ok
    ).toBe(true);
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['spawn.submit.request'],
        targetKind: 'actorMethod',
        actorMethod: {
          ...actorMethodTarget,
          actorRef: { ...actorMethodTarget.actorRef, epoch: undefined }
        }
      })
    ).toEqual({
      ok: false,
      error:
        'invalid spawn.submit.request envelope: actorMethod.actorRef.epoch must be a positive integer'
    });
    expect(
      validateRuntimeToRouterFrameHeader({
        ...runtimeFrameHeaderFixtures['spawn.submit.request'],
        actorRef: runtimeFrameHeaderFixtures['actor.getOrCreate.response'].actorRef,
        methodName: 'receive'
      })
    ).toEqual({
      ok: false,
      error: 'invalid spawn.submit.request frame header envelope: actorRef is not supported'
    });
  });

  it('enforces the shared structured activation identity corpus on every actor/spawn request', () => {
    const requestTypes = [
      'actor.getOrCreate.request',
      'actor.replace.request',
      'actor.find.request',
      'actor.remove.request',
      'spawn.submit.request'
    ] as const;

    for (const type of requestTypes) {
      const header = runtimeFrameHeaderFixtures[type];
      expect(header.activationIdentity).toEqual(
        actorControlActivationIdentityCorpus.valid
      );
      expect(validateRuntimeToRouterFrameHeader(header)).toEqual({
        ok: true,
        envelope: header
      });
      for (const branch of protocolEnvelopeSchemaBranches(
        runtimeFrameHeaderSchemas[type]
      )) {
        expect(branch.required).toContain('activationIdentity');
        expect(branch.properties.activationIdentity).toEqual({
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
        });
      }

      const { activationIdentity: _activationIdentity, ...missingActivation } = header;
      expect(validateRuntimeToRouterFrameHeader(missingActivation).ok).toBe(false);

      expect(
        validateRuntimeToRouterFrameHeader({
          ...header,
          serviceDisplayName: 'legacy-inference'
        })
      ).toEqual({
        ok: false,
        error: `invalid ${type} frame header envelope: serviceDisplayName is not supported`
      });

      for (const invalid of actorControlActivationIdentityCorpus.invalid) {
        expect(
          validateRuntimeToRouterFrameHeader({
            ...header,
            activationIdentity: invalid.value
          }),
          `${type} accepted ${invalid.label}`
        ).toMatchObject({ ok: false });
      }
      const decoded = decodeRuntimeFrame(encodeRuntimeFrame(header));
      expect(decoded.header).toEqual(header);
    }

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
      'invalid skiff runtime frame: schemaVersion must be skiff-runtime-frame-v3'
    );
    expect(() => decodeRuntimeFrame(encodeBinaryFrame(responseEnd))).toThrow(
      'invalid skiff runtime frame: schemaVersion must be skiff-runtime-frame-v3'
    );
    expect(validateRouterToRuntimeFrameHeader(requestStart)).toEqual({
      ok: false,
      error:
        'invalid request.start frame header envelope: schemaVersion must be one of skiff-runtime-frame-v3'
    });
    expect(validateRuntimeToRouterFrameHeader(responseEnd)).toEqual({
      ok: false,
      error:
        'invalid response.end frame header envelope: schemaVersion must be one of skiff-runtime-frame-v3'
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
        'invalid response.end envelope: websocketConnect.connectionPolicy.maxConnections must be an unsigned non-zero 32-bit integer'
    });
  });

  it('accepts only positive safe websocket admission ranks', () => {
    const frame = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: 'websocket-admission-rank',
      payloadPresent: false,
      websocketConnect: {
        result: 'accept',
        businessIdentity: 'user-1',
        admissionRank: Number.MAX_SAFE_INTEGER
      }
    };

    expect(validateRuntimeToRouterFrameHeader(frame)).toMatchObject({ ok: true });
    for (const admissionRank of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      expect(
        validateRuntimeToRouterFrameHeader({
          ...frame,
          websocketConnect: { ...frame.websocketConnect, admissionRank }
        })
      ).toEqual({
        ok: false,
        error:
          'invalid response.end envelope: websocketConnect.admissionRank must be a positive safe integer'
      });
    }
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

  it('accepts strict connection request, cancel, and response frame headers', () => {
    const websocketEntryId =
      `skiff-websocket-entry-v1:sha256:${'a'.repeat(64)}`;
    const request = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.request',
      requestId: 'connection-request-1',
      serviceId: 'example.com/chat',
      websocketEntryId,
      connectionId: 'connection-1',
      profile: 'jsonrpc-2.0-text',
      method: 'chat.send',
      deadline: {
        timeoutMs: 1000,
        expiresAt: '2030-01-02T03:04:05Z'
      }
    };
    expect(validateRuntimeToRouterFrameHeader(request)).toEqual({
      ok: true,
      envelope: request
    });
    const cancel = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.request.cancel',
      requestId: request.requestId,
      reason: 'caller_cancel'
    };
    expect(validateRuntimeToRouterFrameHeader(cancel)).toEqual({
      ok: true,
      envelope: cancel
    });
    const response = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.response',
      requestId: request.requestId,
      outcome: 'remote',
      remote: {
        code: -32603,
        message: ' peer failed ',
        dataPresent: true
      }
    };
    expect(validateRouterToRuntimeFrameHeader(response)).toEqual({
      ok: true,
      envelope: response
    });
  });

  it('rejects unknown fields and invalid connection response branches', () => {
    const websocketEntryId =
      `skiff-websocket-entry-v1:sha256:${'a'.repeat(64)}`;
    const request = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.request',
      requestId: 'connection-request-1',
      serviceId: 'example.com/chat',
      websocketEntryId,
      connectionId: 'connection-1',
      profile: 'jsonrpc-2.0-text',
      method: 'chat.send',
      deadline: {
        timeoutMs: 1000,
        expiresAt: '2030-01-02T03:04:05Z'
      }
    };
    for (const expiresAt of [
      '2030-02-30T03:04:05Z',
      '2030-01-02T03:04:05suffixZ',
      '2030-01-02T24:04:05Z',
      '2030-01-02T03:04:05+24:00'
    ]) {
      expect(validateRuntimeToRouterFrameHeader({
        ...request,
        deadline: {
          ...request.deadline,
          expiresAt
        }
      }).ok).toBe(false);
    }
    expect(validateRuntimeToRouterFrameHeader({
      ...request,
      deadline: {
        ...request.deadline,
        timeoutMs: Number.MAX_SAFE_INTEGER + 1
      }
    }).ok).toBe(false);

    const base = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'connection.response',
      requestId: 'connection-request-1'
    };
    expect(validateRouterToRuntimeFrameHeader({
      ...base,
      outcome: 'protocolError',
      unexpected: true
    }).ok).toBe(false);
    expect(validateRouterToRuntimeFrameHeader({
      ...base,
      outcome: 'remote'
    }).ok).toBe(false);
    expect(validateRouterToRuntimeFrameHeader({
      ...base,
      outcome: 'success',
      remote: {
        code: 1,
        message: 'forbidden',
        dataPresent: false
      }
    }).ok).toBe(false);
  });

  it('requires service, entry, and connection identity for direct connection.send targets', () => {
    const {
      businessIdentity: _businessIdentity,
      websocketEntryId: _websocketEntryId,
      ...base
    } = runtimeFrameHeaderFixtures['connection.send'];
    expect(
      validateRuntimeToRouterFrameHeader({
        ...base,
        connectionId: 'connection-1'
      })
    ).toEqual({
      ok: false,
      error:
        'invalid connection.send envelope: websocketEntryId must be a non-empty string for connectionId target'
    });

    expect(
      validateRuntimeToRouterFrameHeader({
        ...base,
        websocketEntryId: 'entry-1',
        connectionId: 'connection-1'
      })
    ).toEqual({
      ok: true,
      envelope: {
        ...base,
        websocketEntryId: 'entry-1',
        connectionId: 'connection-1'
      }
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
