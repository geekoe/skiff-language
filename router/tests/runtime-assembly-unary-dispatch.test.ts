import { mkdtemp, rm } from 'node:fs/promises';
import { request as httpRequest } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import WebSocket from 'ws';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { encodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  decodeBinaryFrame,
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ResponseEndFrameHeader
} from '../src/protocol/envelope.js';
import type { RuntimeAssemblyRequestStartFrameHeader } from '../src/protocol/runtimeAssemblyRequest.js';
import {
  runtimeFrameHeaderFixtures,
  validateRuntimeAssemblyRequestStartFrameHeader
} from '../src/protocol/runtimeProtocol.js';
import {
  assemblyHttpRequestHeader,
  assemblyTestHttpRequestHeader,
  AssemblyHttpGateway,
  effectiveHttpRequestTimeoutMs
} from '../src/router/assemblyHttpGateway.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import {
  FixedServiceResponseError,
  RuntimeResponseError
} from '../src/router/errors.js';
import { FilesystemRuntimeAssemblySnapshotLoader } from '../src/router/filesystemRuntimeAssemblySnapshotLoader.js';
import { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import type { RuntimeUnaryDispatchFrameHeader } from '../src/router/runtimeRegistry.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type LoadedRuntimeAssembly,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';
import { writeCurrentScopeCompilerGeneratedArtifactRoot } from './helpers/compilerArtifacts.js';

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'b'.repeat(64)}`;
const RUNTIME_ID = 'runtime-unary-a';
const HOST = 'api.localhost';
const PATH = '/v1/invoke';
const PRIVATE_SENTINELS = [
  'provider-private-secret',
  '/callee/private/source.skiff',
  'calleePrivateFunction',
  'sourceFrames',
  'stack'
] as const;
const fixtures: UnaryFixture[] = [];
let currentScopeRoot: string;
let currentScopeAssembly: LoadedRuntimeAssembly;

beforeAll(async () => {
  currentScopeRoot = await mkdtemp(
    join(tmpdir(), 'skiff-router-current-scope-unary-')
  );
  const generated =
    await writeCurrentScopeCompilerGeneratedArtifactRoot(currentScopeRoot);
  currentScopeAssembly = await new FilesystemRuntimeAssemblySnapshotLoader(
    currentScopeRoot
  ).load(generated.receipt.baseAssembly);
}, 120_000);

afterAll(async () => {
  await rm(currentScopeRoot, { recursive: true, force: true });
});

describe('RuntimeAssembly canonical HTTP unary dispatch', () => {
  afterEach(async () => {
    while (fixtures.length > 0) {
      await fixtures.pop()!.close();
    }
  });

  it('dispatches the exact S0 unary binding to an observable response', async () => {
    const exact = currentScopeAssembly.gatewayIngress.find(
      (candidate) =>
        candidate.selector.protocol === 'http' &&
        candidate.selector.path === '/current-scope/unary'
    );
    if (exact === undefined) {
      throw new Error('current-scope unary binding is missing');
    }
    const fixture = await createFixture({
      binding: exact,
      assemblyIdentity: currentScopeAssembly.assemblyIdentity,
      generation: 1
    });
    const response = sendHttp(
      fixture.httpUrl,
      Buffer.from('source-body', 'utf8'),
      '',
      {
        ...exact.selector,
        serviceId: exact.deployment.serviceId,
        contractVersion: exact.deployment.contractVersion
      }
    );
    const requestFrame = decodeBinaryFrame(
      await nextBinaryMessage(fixture.runtime)
    );
    const requestId = String(requestFrame.header.requestId);
    expect(requestFrame.header).toMatchObject({
      type: 'request.start',
      mode: 'unary',
      routing: {
        assemblyIdentity: currentScopeAssembly.assemblyIdentity,
        assemblyGeneration: 1,
        gatewayEntryIdentity:
          'skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2',
        ingress: exact.selector
      }
    });
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: true,
      httpResponse: {
        status: 201,
        headers: [{ name: 'x-source-receipt', value: 'current-scope' }]
      }
    }, Buffer.from('host-observable', 'utf8')));

    await expect(response).resolves.toEqual({
      status: 201,
      headers: expect.objectContaining({
        'x-source-receipt': 'current-scope'
      }),
      body: Buffer.from('host-observable', 'utf8')
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('selects same-Host same-path services by trusted headers and frames exact deployments', async () => {
    const selector = {
      protocol: 'http' as const,
      method: 'GET',
      path: '/v1/models'
    };
    const relay: RuntimeAssemblyIngressBinding = {
      ...BINDING,
      selector,
      deployment: {
        serviceId: 'skiff.run/codex-relay',
        contractVersion: '1.0.0',
        deploymentRevision: 'relay-revision',
        deploymentArtifactIdentity:
          `skiff-deployment-artifact-v4:sha256:${'1'.repeat(64)}`
      },
      gatewayEntryKey: 'relayModels',
      gatewayEntryIdentity:
        `skiff-gateway-entry-v2:sha256:${'2'.repeat(64)}`
    };
    const aihub: RuntimeAssemblyIngressBinding = {
      ...relay,
      deployment: {
        serviceId: 'skiff.run/aihub',
        contractVersion: '1.0.0',
        deploymentRevision: 'aihub-revision',
        deploymentArtifactIdentity:
          `skiff-deployment-artifact-v4:sha256:${'3'.repeat(64)}`
      },
      gatewayEntryKey: 'aihubModels',
      gatewayEntryIdentity:
        `skiff-gateway-entry-v2:sha256:${'4'.repeat(64)}`
    };
    const fixture = await createFixture({
      binding: relay,
      bindings: [relay, aihub]
    });

    for (const [binding, responseBody] of [
      [relay, 'relay'],
      [aihub, 'aihub']
    ] as const) {
      const response = sendHttp(fixture.httpUrl, new Uint8Array(), '', {
        ...selector,
        serviceId: binding.deployment.serviceId,
        contractVersion: binding.deployment.contractVersion
      });
      const frame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
      const validation =
        validateRuntimeAssemblyRequestStartFrameHeader(frame.header);
      expect(validation).toMatchObject({ ok: true });
      if (!validation.ok) throw new Error(validation.error);
      expect(validation.envelope.routing).toMatchObject({
        deployment: binding.deployment,
        gatewayEntryIdentity: binding.gatewayEntryIdentity,
        ingress: selector
      });
      expect(new URL(validation.envelope.httpRequest!.url).host).toBe(HOST);

      fixture.runtime.send(encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: validation.envelope.requestId,
        payloadPresent: true,
        httpResponse: { status: 200, headers: [] }
      }, Buffer.from(responseBody)));
      await expect(response).resolves.toMatchObject({
        status: 200,
        body: Buffer.from(responseBody)
      });
    }
  });

  it.each([
    {
      name: 'service',
      selector: {
        method: 'POST',
        path: PATH,
        serviceId: 'example/other'
      }
    },
    {
      name: 'version',
      selector: {
        method: 'POST',
        path: PATH,
        contractVersion: '2.0.0'
      }
    },
    {
      name: 'method',
      selector: {
        method: 'GET',
        path: PATH
      }
    },
    {
      name: 'path',
      selector: {
        method: 'POST',
        path: '/wrong'
      }
    }
  ])('fails closed before dispatch for a mismatched $name selector', async ({ selector }) => {
    const fixture = await createFixture();

    const response = await sendHttp(
      fixture.httpUrl,
      new Uint8Array(),
      '',
      selector
    );
    expect(response.status).toBe(404);
    expect(JSON.parse(response.body.toString())).toMatchObject({
      error: { code: 'AssemblyIngressNotFound' }
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('writes validator-accepted nested headers and preserves zero and opaque payloads', async () => {
    const fixture = await createFixture();

    const zeroResponse = sendHttp(fixture.httpUrl, new Uint8Array());
    const zeroFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const zeroValidation = validateRuntimeAssemblyRequestStartFrameHeader(zeroFrame.header);
    expect(zeroValidation).toMatchObject({ ok: true });
    if (!zeroValidation.ok) throw new Error(zeroValidation.error);
    expect(zeroValidation.envelope).toMatchObject({
      type: 'request.start',
      mode: 'unary',
      caller: { kind: 'gateway' },
      routing: {
        kind: 'runtimeAssembly',
        assemblyIdentity: ASSEMBLY,
        assemblyGeneration: 7,
        gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
        ingress: { protocol: 'http', method: 'POST', path: PATH }
      },
      httpRequest: {
        method: 'POST',
        path: PATH
      },
      testEffectsEnabled: false
    });
    expect(zeroValidation.envelope).not.toHaveProperty('target');
    expect(zeroValidation.envelope).not.toHaveProperty('operationAbiId');
    expect(zeroValidation.envelope).not.toHaveProperty('buildId');
    expect(zeroValidation.envelope).not.toHaveProperty('serviceProtocolIdentity');
    expect(zeroValidation.envelope).not.toHaveProperty('assemblyIdentity');
    expect(zeroValidation.envelope).not.toHaveProperty('assemblyGeneration');
    expect(zeroValidation.envelope).not.toHaveProperty('contractOperationId');
    expect(zeroValidation.envelope.caller).toEqual({ kind: 'gateway' });
    expect(zeroValidation.envelope).not.toHaveProperty('gatewayEntryIdentity');
    expect(zeroValidation.envelope).not.toHaveProperty('testEffectDoubles');
    expect(Object.keys(zeroValidation.envelope.caller)).toEqual(['kind']);
    expect(Object.keys(zeroValidation.envelope.routing).sort()).toEqual([
      'assemblyGeneration',
      'assemblyIdentity',
      'deployment',
      'gatewayEntryIdentity',
      'ingress',
      'kind'
    ]);
    expect(zeroFrame.payloadBytes).toHaveLength(0);

    const opaqueResponseBytes = new Uint8Array([0, 255, 17, 128]);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: zeroValidation.envelope.requestId,
      payloadPresent: true,
      httpResponse: { status: 200, headers: [] }
    }, opaqueResponseBytes));
    const completedZero = await zeroResponse;
    expect(completedZero.status).toBe(200);
    expect(completedZero.headers['content-type']).toBeUndefined();
    expect(completedZero.body).toEqual(Buffer.from(opaqueResponseBytes));

    const opaqueRequestBytes = new Uint8Array([123, 0, 255, 34]);
    const opaqueResponse = sendHttp(fixture.httpUrl, opaqueRequestBytes, '?mode=opaque');
    const opaqueFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const opaqueValidation = validateRuntimeAssemblyRequestStartFrameHeader(opaqueFrame.header);
    expect(opaqueValidation).toMatchObject({ ok: true });
    if (!opaqueValidation.ok) throw new Error(opaqueValidation.error);
    expect(Buffer.from(opaqueFrame.payloadBytes)).toEqual(Buffer.from(opaqueRequestBytes));
    expect(new URL(opaqueValidation.envelope.httpRequest!.url).host).toBe(HOST);
    expect(opaqueValidation.envelope.httpRequest).toMatchObject({
      method: opaqueValidation.envelope.routing.ingress.method,
      path: opaqueValidation.envelope.routing.ingress.path
    });

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: opaqueValidation.envelope.requestId,
      payloadPresent: true,
      httpResponse: {
        status: 201,
        headers: [{ name: 'x-runtime-result', value: 'opaque' }]
      }
    }, new Uint8Array([9, 8, 7])));
    const completedOpaque = await opaqueResponse;
    expect(completedOpaque.status).toBe(201);
    expect(completedOpaque.headers['x-runtime-result']).toBe('opaque');
    expect(completedOpaque.body).toEqual(Buffer.from([9, 8, 7]));
  });

  it('isolates canonical test headers and dispatch from the ordinary production seam', async () => {
    const fixture = await createFixture();
    const productionHeader = canonicalHeader(
      fixture.snapshot,
      'ordinary-production'
    );
    expect(productionHeader.testEffectsEnabled).toBe(false);

    const testHeader = assemblyTestHttpRequestHeader({
      snapshot: fixture.snapshot,
      binding: fixture.binding,
      requestId: 'isolated-test-dispatch',
      timeoutMs: 1_000,
      routing: productionHeader.routing,
      mode: productionHeader.mode,
      httpRequest: productionHeader.httpRequest
    });
    expect(testHeader).toMatchObject({
      routing: productionHeader.routing,
      mode: productionHeader.mode,
      httpRequest: productionHeader.httpRequest,
      testEffectsEnabled: true
    });

    await expect(
      fixture.dispatcher.dispatchBinary(
        {
          header: testHeader,
          payloadBytes: Buffer.from('null', 'utf8')
        },
        100
      )
    ).rejects.toThrow(
      'active RuntimeAssembly dispatch rejects test effect controls'
    );
    await expect(
      fixture.dispatcher.dispatchAssemblyTestBinary(
        {
          header: productionHeader,
          payloadBytes: Buffer.from('null', 'utf8')
        },
        100
      )
    ).rejects.toThrow(
      'test RuntimeAssembly dispatch requires test effects enabled'
    );
    const legacyTestHeader = mutate(testHeader, (header) => {
      header.contractOperationId =
        `skiff-contract-operation-v1:sha256:${'f'.repeat(64)}`;
    });
    await expect(
      fixture.dispatcher.dispatchAssemblyTestBinary(
        {
          header: legacyTestHeader,
          payloadBytes: Buffer.from('null', 'utf8')
        },
        100
      )
    ).rejects.toThrow(/contractOperationId is not supported/);

    const dispatch = fixture.dispatcher.dispatchAssemblyTestBinary(
      {
        header: testHeader,
        payloadBytes: Buffer.from('null', 'utf8')
      },
      1_000
    );
    const frame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const validation = validateRuntimeAssemblyRequestStartFrameHeader(
      frame.header
    );
    expect(validation).toMatchObject({ ok: true });
    if (!validation.ok) throw new Error(validation.error);
    expect(validation.envelope.testEffectsEnabled).toBe(true);
    expect(validation.envelope.routing).toEqual(testHeader.routing);
    expect(validation.envelope.httpRequest).toEqual(testHeader.httpRequest);
    expect(Buffer.from(frame.payloadBytes)).toEqual(
      Buffer.from('null', 'utf8')
    );

    const responseHeader: ResponseEndFrameHeader = {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: testHeader.requestId,
      payloadPresent: true,
      httpResponse: {
        status: 200,
        headers: [
          {
            name: 'content-type',
            value: 'application/json; charset=utf-8'
          }
        ]
      }
    };
    fixture.runtime.send(
      encodeRuntimeFrame(responseHeader, Buffer.from('null', 'utf8'))
    );
    const result = await dispatch;
    expect(result.header).toEqual(responseHeader);
    expect(Buffer.from(result.payloadBytes)).toEqual(
      Buffer.from('null', 'utf8')
    );
  });

  it('forwards typedJson request and response bytes without decoding or re-encoding', async () => {
    const fixture = await createFixture({ binding: TYPED_BINDING });
    const requestBytes = new Uint8Array([0, 255, 128, 17]);
    const response = sendHttp(fixture.httpUrl, requestBytes);
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    const validation = validateRuntimeAssemblyRequestStartFrameHeader(
      requestFrame.header
    );
    expect(validation).toMatchObject({ ok: true });
    if (!validation.ok) throw new Error(validation.error);
    expect(validation.envelope.routing.gatewayEntryIdentity).toBe(
      TYPED_BINDING.gatewayEntryIdentity
    );
    expect(Buffer.from(requestFrame.payloadBytes)).toEqual(
      Buffer.from(requestBytes)
    );

    const responseBytes = new Uint8Array([255, 0, 123, 128]);
    fixture.runtime.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: validation.envelope.requestId,
          payloadPresent: true,
          httpResponse: {
            status: 202,
            headers: [{ name: 'content-type', value: 'application/json' }]
          }
        },
        responseBytes
      )
    );
    await expect(response).resolves.toMatchObject({
      status: 202,
      body: Buffer.from(responseBytes)
    });
  });

  it('uses the smaller platform and deployment timeout for both deadline and cancel timer', async () => {
    const cases: Array<{
      platformCapMs: number;
      deploymentTimeoutMs?: number;
      expectedMs: number;
    }> = [
      { platformCapMs: 200, deploymentTimeoutMs: 40, expectedMs: 40 },
      { platformCapMs: 50, expectedMs: 50 },
      { platformCapMs: 30, deploymentTimeoutMs: 200, expectedMs: 30 }
    ];

    for (const [index, timeoutCase] of cases.entries()) {
      const binding: RuntimeAssemblyIngressBinding = {
        ...BINDING,
        gatewayEntryKey: `timeout-${index}`,
        ...(timeoutCase.deploymentTimeoutMs === undefined
          ? {}
          : { timeoutMs: timeoutCase.deploymentTimeoutMs })
      };
      const fixture = await createFixture({
        binding,
        requestTimeoutMs: timeoutCase.platformCapMs
      });
      const finishPending = spyOnFinishPending(fixture.dispatcher);
      const response = sendHttp(fixture.httpUrl, new Uint8Array());
      const requestFrame = decodeBinaryFrame(
        await nextBinaryMessage(fixture.runtime)
      );
      const validation = validateRuntimeAssemblyRequestStartFrameHeader(
        requestFrame.header
      );
      if (!validation.ok) throw new Error(validation.error);
      expect(validation.envelope.deadline?.timeoutMs).toBe(
        timeoutCase.expectedMs
      );
      const cancelObservation = observeRequestCancels(
        fixture.runtime,
        validation.envelope.requestId
      );
      const cancelFrame = nextBinaryMessage(fixture.runtime);
      const completed = await response;
      expect(completed.status).toBe(504);
      expect(JSON.parse(completed.body.toString())).toMatchObject({
        error: {
          code: 'TimeoutError',
          message: `Runtime did not respond within ${timeoutCase.expectedMs}ms`
        }
      });
      expect(decodeRuntimeFrame(await cancelFrame).header).toMatchObject({
        type: 'request.cancel',
        requestId: validation.envelope.requestId,
        reason: 'timeout'
      });
      fixture.runtime.send(encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: validation.envelope.requestId,
        payloadPresent: false
      }));
      await nextTurn();
      cancelObservation.stop();
      expect(cancelObservation.count()).toBe(1);
      expect(
        finishPendingCalls(finishPending, validation.envelope.requestId)
      ).toBe(1);
    }
  });

  it('rejects invalid timeout caps and overrides instead of extending them', () => {
    expect(() => effectiveHttpRequestTimeoutMs(0)).toThrow(
      /positive safe integer/
    );
    expect(() => effectiveHttpRequestTimeoutMs(-1)).toThrow(
      /positive safe integer/
    );
    expect(() => effectiveHttpRequestTimeoutMs(1.5)).toThrow(
      /positive safe integer/
    );
    expect(() =>
      effectiveHttpRequestTimeoutMs(Number.MAX_SAFE_INTEGER + 1)
    ).toThrow(/positive safe integer/);
    expect(() => effectiveHttpRequestTimeoutMs(2_147_483_648)).toThrow(
      /deadline and timer/
    );
    expect(() => effectiveHttpRequestTimeoutMs(100, 0)).toThrow(
      /positive safe integer/
    );
  });

  it('rejects oversized requests before Runtime dispatch', async () => {
    const fixture = await createFixture({ maxRequestBytes: 3 });

    const response = await sendHttp(fixture.httpUrl, new Uint8Array([1, 2, 3, 4]));
    expect(response.status).toBe(413);
    expect(JSON.parse(response.body.toString())).toMatchObject({
      error: { code: 'RequestTooLarge' }
    });
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('rejects an oversized unary Runtime response at the Router boundary', async () => {
    const fixture = await createFixture({ maxResponseBytes: 3 });
    const response = sendHttp(fixture.httpUrl, new Uint8Array());
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: String(requestFrame.header.requestId),
      payloadPresent: true,
      httpResponse: { status: 200, headers: [] }
    }, new Uint8Array([1, 2, 3, 4])));

    const completed = await response;
    expect(completed).toMatchObject({
      status: 502,
      body: expect.any(Buffer)
    });
    expect(JSON.parse(completed.body.toString())).toMatchObject({
      error: { code: 'ResponseTooLarge' }
    });
  });

  it('requires HTTP unary status and headers from Runtime', async () => {
    const fixture = await createFixture();
    const response = sendHttp(fixture.httpUrl, new Uint8Array());
    const requestFrame = decodeBinaryFrame(
      await nextBinaryMessage(fixture.runtime)
    );
    fixture.runtime.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: String(requestFrame.header.requestId),
        payloadPresent: false
      })
    );

    const completed = await response;
    expect(completed.status).toBe(502);
    expect(JSON.parse(completed.body.toString())).toMatchObject({
      error: { code: 'InvalidRuntimeResponse' }
    });
  });

  it('forwards fixed and control unaryFrame errors with exact v2 headers and bytes', async () => {
    const fixture = await createFixture();
    for (const [index, kind] of (
      ['publicTypedError', 'internalError', 'platformError'] as const
    ).entries()) {
      const requestId = `unary-frame-fixed-${kind}`;
      const dispatch = fixture.dispatcher.dispatchBinaryFrame(
        {
          header: canonicalHeader(fixture.snapshot, requestId),
          payloadBytes: new Uint8Array([index, 255 - index])
        },
        1_000
      );
      const request = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
      expect(request.header.requestId).toBe(requestId);
      const header = {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId,
        errorKind: 'fixedService'
      } as const;
      const payloadBytes = fixedServicePayload(
        kind,
        `trace-frame-${index}`,
        `error-frame-${index}`
      );
      fixture.runtime.send(encodeRuntimeFrame(header, payloadBytes));

      const result = await dispatch;
      expect(result.header).toEqual(header);
      expect(Buffer.from(result.payloadBytes)).toEqual(Buffer.from(payloadBytes));
    }

    const requestId = 'unary-frame-control';
    const dispatch = fixture.dispatcher.dispatchBinaryFrame(
      {
        header: canonicalHeader(fixture.snapshot, requestId),
        payloadBytes: new Uint8Array()
      },
      1_000
    );
    await nextBinaryMessage(fixture.runtime);
    const header = {
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId,
      errorKind: 'control',
      error: {
        code: 'InternalError',
        message: 'The service could not complete the request.',
        status: 500,
        details: { traceId: 'control-only-trace' }
      }
    } as const;
    fixture.runtime.send(encodeRuntimeFrame(header));

    const result = await dispatch;
    expect(result.header).toEqual(header);
    expect(result.payloadBytes).toHaveLength(0);
  });

  it('uses mutually exclusive fixed and control errors for ordinary pending requests', async () => {
    const fixture = await createFixture();
    const fixedHeader = canonicalHeader(fixture.snapshot, 'ordinary-fixed');
    const fixedDispatch = fixture.dispatcher
      .dispatchBinary(
        { header: fixedHeader, payloadBytes: new Uint8Array() },
        1_000
      )
      .catch((error: unknown) => error);
    await nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame(
      {
        schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: fixedHeader.requestId,
        errorKind: 'fixedService'
      },
      fixedServicePayload('internalError', 'trace-ordinary-fixed', 'error-ordinary-fixed')
    ));
    const fixedError = await fixedDispatch;
    expect(fixedError).toBeInstanceOf(FixedServiceResponseError);
    expect(fixedError).not.toBeInstanceOf(RuntimeResponseError);
    expect(fixedError).toMatchObject({
      serviceErrorKind: 'internalError',
      traceId: 'trace-ordinary-fixed',
      errorId: 'error-ordinary-fixed'
    });

    const controlHeader = canonicalHeader(fixture.snapshot, 'ordinary-control');
    const controlDispatch = fixture.dispatcher
      .dispatchBinary(
        { header: controlHeader, payloadBytes: new Uint8Array() },
        1_000
      )
      .catch((error: unknown) => error);
    await nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: controlHeader.requestId,
      errorKind: 'control',
      error: {
        code: 'InternalError',
        message: 'The service could not complete the request.',
        status: 500
      }
    }));
    const controlError = await controlDispatch;
    expect(controlError).toBeInstanceOf(RuntimeResponseError);
    expect(controlError).not.toBeInstanceOf(FixedServiceResponseError);
  });

  it('does not project the rejected legacy cancellation code to HTTP 499', () => {
    const legacyCancellation = new RuntimeResponseError({
      code: 'CancelError',
      message: 'legacy cancellation must not be an ordinary runtime error'
    });

    expect(legacyCancellation.statusCode).toBe(500);
  });

  it('fails closed on legacy fixed and control cancellation before HTTP projection', async () => {
    for (const errorKind of ['fixedService', 'control'] as const) {
      const fixture = await createFixture();
      const response = sendHttp(fixture.httpUrl, new Uint8Array());
      const request = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
      const closeFacts: Array<{ code: number; reason: string }> = [];
      fixture.runtime.once('close', (code, reason) => {
        closeFacts.push({
          code,
          reason: Buffer.from(reason).toString('utf8')
        });
      });

      if (errorKind === 'fixedService') {
        fixture.runtime.send(
          encodeRuntimeFrame(
            {
              schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
              type: 'response.error',
              requestId: String(request.header.requestId),
              errorKind
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
          )
        );
      } else {
        fixture.runtime.send(
          encodeRuntimeFrame({
            schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
            type: 'response.error',
            requestId: String(request.header.requestId),
            errorKind,
            error: {
              code: 'CancelError',
              message: 'legacy cancellation must not become an ordinary response'
            }
          })
        );
      }

      const completed = await response;
      expect(completed.status).toBe(503);
      expect(JSON.parse(completed.body.toString())).toMatchObject({
        error: {
          code: 'std.service.ProviderUnavailableError',
          message: 'Runtime disconnected before responding'
        }
      });
      await until(() => fixture.runtime.readyState === WebSocket.CLOSED);
      expect(closeFacts).toEqual([
        {
          code: 1008,
          reason:
            errorKind === 'fixedService'
              ? 'invalid response.error fixedService frame: builtinErrorIdentity is not supported'
              : 'invalid response.error envelope: error.code is reserved for internal cancellation'
        }
      ]);
      expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
        pendingUnary: 0,
        pendingStream: 0
      });
    }
  });

  it('maps every fixed kind to one redacted HTTP 5xx fact and redacts generic 5xx details', async () => {
    const fixture = await createFixture();
    for (const [index, kind] of (
      ['publicTypedError', 'internalError', 'platformError'] as const
    ).entries()) {
      const traceId = `trace-http-${index}`;
      const errorId = `error-http-${index}`;
      const response = sendHttp(fixture.httpUrl, new Uint8Array());
      const request = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
      fixture.runtime.send(encodeRuntimeFrame(
        {
          schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
          type: 'response.error',
          requestId: String(request.header.requestId),
          errorKind: 'fixedService'
        },
        fixedServicePayload(kind, traceId, errorId)
      ));

      const completed = await response;
      expect(completed.status).toBe(500);
      expect(JSON.parse(completed.body.toString())).toEqual({
        error: {
          code: 'FixedServiceError',
          message: 'Service request failed',
          details: { traceId, errorId }
        }
      });
      assertNoPrivateSentinels(completed.body.toString());
    }

    const controlResponse = sendHttp(fixture.httpUrl, new Uint8Array());
    const controlRequest = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: String(controlRequest.header.requestId),
      errorKind: 'control',
      error: {
        code: 'std.service.ProtocolError',
        message: 'runtime protocol failure',
        status: 502,
        details: {
          private: PRIVATE_SENTINELS.join('|')
        }
      }
    }));
    const completedControl = await controlResponse;
    expect(completedControl.status).toBe(502);
    expect(JSON.parse(completedControl.body.toString())).toEqual({
      error: {
        code: 'std.service.ProtocolError',
        message: 'runtime protocol failure'
      }
    });
    assertNoPrivateSentinels(completedControl.body.toString());
  });

  it('fails closed before the socket for legacy, flat, unknown, stream, adapter, and HTTP mismatches', async () => {
    const fixture = await createFixture();
    const valid = canonicalHeader(fixture.snapshot, 'valid');
    const legacy = mutate(valid, (header) => {
      header.assemblyIdentity = header.routing.assemblyIdentity;
      header.assemblyGeneration = header.routing.assemblyGeneration;
      header.contractOperationId =
        `skiff-contract-operation-v1:sha256:${'f'.repeat(64)}`;
      delete header.routing;
    });
    const invalid: RuntimeUnaryDispatchFrameHeader[] = [
      legacy,
      mutate(valid, (header) => {
        header.assemblyIdentity = ASSEMBLY;
      }),
      mutate(valid, (header) => {
        header.unknown = true;
      }),
      mutate(valid, (header) => {
        header.routing.assemblyIdentity =
          `skiff-runtime-assembly-v3:sha256:${'f'.repeat(64)}`;
      }),
      mutate(valid, (header) => {
        header.routing.assemblyGeneration += 1;
      }),
      mutate(valid, (header) => {
        header.routing.gatewayEntryIdentity =
          `skiff-gateway-entry-v2:sha256:${'f'.repeat(64)}`;
      }),
      mutate(valid, (header) => {
        header.routing.ingress.method = 'GET';
      }),
      mutate(valid, (header) => {
        header.routing.ingress.host = 'wrong.localhost';
      }),
      mutate(valid, (header) => {
        header.routing.ingress.path = '/wrong';
      }),
      mutate(valid, (header) => {
        header.mode = 'serverStream';
      }),
      mutate(valid, (header) => {
        header.httpAdapter = {
          kind: 'rawHttp',
          handler: { kind: 'serviceFunction', modulePath: 'service', symbol: 'invoke' },
          adapterArgs: []
        };
      }),
      mutate(valid, (header) => {
        delete header.httpRequest;
      }),
      mutate(valid, (header) => {
        header.gatewayEntryIdentity = `skiff-gateway-v1:sha256:${'e'.repeat(64)}`;
        header.websocketEntryId = 'entry-a';
        header.websocketAdapter = {
          kind: 'connect',
          adapterArgs: [],
          connectRequest: {
            connectionId: 'connection-a',
            url: `ws://${HOST}${PATH}`,
            query: [],
            headers: [],
            cookies: []
          }
        };
      }),
      mutate(valid, (header) => {
        header.httpRequest.method = 'GET';
      }),
      mutate(valid, (header) => {
        header.httpRequest.path = '/wrong';
      }),
      mutate(valid, (header) => {
        header.httpRequest.url = 'http://wrong.localhost/v1/invoke';
      }),
      mutate(valid, (header) => {
        header.httpRequest.url = `http://${HOST}/wrong`;
      }),
      mutate(valid, (header) => {
        header.testEffectsEnabled = true;
      }),
      mutate(valid, (header) => {
        header.testEffectDoubles = { effect: [{ response: null }] };
      })
    ];

    let ordinal = 0;
    for (const header of invalid) {
      ordinal += 1;
      header.requestId = `invalid-${ordinal}`;
      await expect(fixture.dispatcher.dispatchBinary({
        header,
        payloadBytes: new Uint8Array([ordinal])
      }, 100)).rejects.toThrow();
    }
    await nextTurn();
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });

    const timeoutHeader = canonicalHeader(fixture.snapshot, 'timeout');
    const timeoutDispatch = fixture.dispatcher.dispatchBinary({
      header: timeoutHeader,
      payloadBytes: new Uint8Array()
    }, 50);
    await nextBinaryMessage(fixture.runtime);
    const timeoutCancelPromise = nextBinaryMessage(fixture.runtime);
    await expect(timeoutDispatch).rejects.toThrow(/within 50ms/);
    const timeoutCancel = decodeRuntimeFrame(await timeoutCancelPromise);
    expect(timeoutCancel.header).toMatchObject({
      type: 'request.cancel',
      requestId: timeoutHeader.requestId
    });
    expect(timeoutCancel.payloadBytes).toHaveLength(0);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('keeps caller-abort cancel on the same request and socket with an empty payload', async () => {
    const fixture = await createFixture();
    const controller = new AbortController();
    const header = canonicalHeader(fixture.snapshot, 'caller-abort');
    const finishPending = spyOnFinishPending(fixture.dispatcher);
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000, { signal: controller.signal });
    const requestFrame = decodeBinaryFrame(await nextBinaryMessage(fixture.runtime));
    expect(requestFrame.header.requestId).toBe(header.requestId);
    const cancelObservation = observeRequestCancels(fixture.runtime, header.requestId);
    const cancelFramePromise = nextBinaryMessage(fixture.runtime);
    controller.abort();
    controller.abort();
    await expect(dispatch).rejects.toThrow(/cancelled before completion/);
    const cancelFrame = decodeRuntimeFrame(await cancelFramePromise);
    expect(cancelFrame.header).toMatchObject({
      type: 'request.cancel',
      requestId: header.requestId,
      reason: 'caller_cancel'
    });
    expect(cancelFrame.payloadBytes).toHaveLength(0);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: header.requestId,
      payloadPresent: false
    }));
    await nextTurn();
    cancelObservation.stop();
    expect(cancelObservation.count()).toBe(1);
    expect(finishPendingCalls(finishPending, header.requestId)).toBe(1);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('cancels the exact Runtime request when the HTTP client disconnects', async () => {
    const fixture = await createFixture();
    const finishPending = spyOnFinishPending(fixture.dispatcher);
    const pendingHttp = startHttp(fixture.httpUrl, new Uint8Array([1, 2, 3]));
    const responseOutcome = pendingHttp.response.catch((error: unknown) => error);
    const requestFrame = decodeBinaryFrame(
      await nextBinaryMessage(fixture.runtime)
    );
    const requestId = String(requestFrame.header.requestId);
    const cancelObservation = observeRequestCancels(fixture.runtime, requestId);
    const cancelFrame = nextBinaryMessage(fixture.runtime);
    pendingHttp.request.destroy();
    pendingHttp.request.destroy();

    expect(await responseOutcome).toBeInstanceOf(Error);
    expect(decodeRuntimeFrame(await cancelFrame).header).toMatchObject({
      type: 'request.cancel',
      requestId,
      reason: 'client_disconnect'
    });
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId,
      payloadPresent: false
    }));
    await nextTurn();
    cancelObservation.stop();
    expect(cancelObservation.count()).toBe(1);
    expect(finishPendingCalls(finishPending, requestId)).toBe(1);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('settles runtime cancel once across duplicate cancel, late responses, and disconnect', async () => {
    const fixture = await createFixture();
    const header = canonicalHeader(fixture.snapshot, 'runtime-cancel-race');
    const finishPending = spyOnFinishPending(fixture.dispatcher);
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);
    const cancelObservation = observeRequestCancels(fixture.runtime, header.requestId);

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.cancel',
      requestId: header.requestId,
      reason: 'drain'
    }));
    await expect(dispatch).rejects.toMatchObject({
      code: 'std.service.ProviderUnavailableError',
      message: 'Runtime cancelled request: drain'
    });

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.cancel',
      requestId: header.requestId,
      reason: 'drain'
    }));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: header.requestId,
      payloadPresent: false
    }));
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: header.requestId,
      errorKind: 'control',
      error: {
        code: 'LateError',
        message: 'late response must not reopen the pending request'
      }
    }));
    await nextTurn();
    fixture.runtime.close();
    await until(() => fixture.runtime.readyState === WebSocket.CLOSED);
    cancelObservation.stop();

    expect(cancelObservation.count()).toBe(0);
    expect(finishPendingCalls(finishPending, header.requestId)).toBe(1);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('keeps a live caller provider failure as ProviderUnavailable on runtime disconnect', async () => {
    const fixture = await createFixture();
    const header = canonicalHeader(fixture.snapshot, 'provider-disconnect');
    const finishPending = spyOnFinishPending(fixture.dispatcher);
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);

    fixture.runtime.close();
    await expect(dispatch).rejects.toMatchObject({
      statusCode: 503,
      code: 'std.service.ProviderUnavailableError',
      message: 'Runtime disconnected before responding'
    });
    await until(() => fixture.runtime.readyState === WebSocket.CLOSED);

    expect(finishPendingCalls(finishPending, header.requestId)).toBe(1);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('settles Router shutdown once and sends one bounded control cancellation', async () => {
    const fixture = await createFixture();
    const header = canonicalHeader(fixture.snapshot, 'router-shutdown');
    const finishPending = spyOnFinishPending(fixture.dispatcher);
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);
    const cancelObservation = observeRequestCancels(fixture.runtime, header.requestId);
    const cancelFrame = nextBinaryMessage(fixture.runtime);

    fixture.dispatcher.close();
    fixture.dispatcher.close();
    await expect(dispatch).rejects.toMatchObject({
      statusCode: 503,
      code: 'std.service.ProviderUnavailableError',
      message: 'Runtime registry is closing'
    });
    expect(decodeRuntimeFrame(await cancelFrame).header).toMatchObject({
      type: 'request.cancel',
      requestId: header.requestId,
      reason: 'router_shutdown'
    });
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: header.requestId,
      payloadPresent: false
    }));
    await nextTurn();
    cancelObservation.stop();

    expect(cancelObservation.count()).toBe(1);
    expect(finishPendingCalls(finishPending, header.requestId)).toBe(1);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });

  it('rejects response.start for unary once, cancels once, and ignores a late terminal', async () => {
    const fixture = await createFixture();
    const header = canonicalHeader(fixture.snapshot, 'unexpected-start');
    const dispatch = fixture.dispatcher.dispatchBinary({
      header,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);
    const cancelFramePromise = nextBinaryMessage(fixture.runtime);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.start',
      requestId: header.requestId,
      httpResponse: { status: 200, headers: [] }
    }));
    await expect(dispatch).rejects.toThrow(/response.start is only valid for serverStream/);
    const cancelFrame = decodeRuntimeFrame(await cancelFramePromise);
    expect(cancelFrame.header).toMatchObject({
      type: 'request.cancel',
      requestId: header.requestId
    });
    expect(cancelFrame.payloadBytes).toHaveLength(0);

    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: header.requestId,
      payloadPresent: false
    }));
    await nextTurn();
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
    expect(fixture.runtime.readyState).toBe(WebSocket.OPEN);

    const errorHeader = canonicalHeader(fixture.snapshot, 'runtime-error');
    const errorDispatch = fixture.dispatcher.dispatchBinary({
      header: errorHeader,
      payloadBytes: new Uint8Array()
    }, 1000);
    await nextBinaryMessage(fixture.runtime);
    let unexpectedOutboundFrames = 0;
    const countOutbound = () => {
      unexpectedOutboundFrames += 1;
    };
    fixture.runtime.on('message', countOutbound);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: errorHeader.requestId,
      errorKind: 'control',
      error: { code: 'Rejected', message: 'runtime rejected unary request' }
    }));
    await expect(errorDispatch).rejects.toThrow(/runtime rejected unary request/);
    fixture.runtime.send(encodeRuntimeFrame({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.end',
      requestId: errorHeader.requestId,
      payloadPresent: false
    }));
    await nextTurn();
    fixture.runtime.off('message', countOutbound);
    expect(unexpectedOutboundFrames).toBe(0);
    expect(fixture.dispatcher.pendingLifecycleCounters()).toEqual({
      pendingUnary: 0,
      pendingStream: 0
    });
  });
});

interface UnaryFixture {
  binding: RuntimeAssemblyIngressBinding;
  dispatcher: RuntimeDispatcher;
  endpoint: RuntimeEndpoint;
  gateway: AssemblyHttpGateway;
  httpUrl: string;
  runtime: WebSocket;
  snapshot: RouterActiveAssemblySnapshot;
  close(): Promise<void>;
}

const BINDING: RuntimeAssemblyIngressBinding = {
  selector: { protocol: 'http', method: 'POST', path: PATH },
  deployment: {
    serviceId: 'example/unary',
    contractVersion: '1.0.0',
    deploymentRevision: 'revision-a',
      deploymentArtifactIdentity: `skiff-deployment-artifact-v4:sha256:${'c'.repeat(64)}`
  },
  gatewayEntryKey: 'invoke',
  gatewayEntryIdentity: GATEWAY_ENTRY_IDENTITY,
  adapterKind: 'rawHttp',
  operationMode: 'unary',
};

const TYPED_BINDING: RuntimeAssemblyIngressBinding = {
  ...BINDING,
  gatewayEntryKey: 'typedInvoke',
  gatewayEntryIdentity:
    `skiff-gateway-entry-v2:sha256:${'e'.repeat(64)}`,
  adapterKind: 'typedJson'
};

async function createFixture(
  limits: {
    assemblyIdentity?: string;
    binding?: RuntimeAssemblyIngressBinding;
    bindings?: RuntimeAssemblyIngressBinding[];
    generation?: number;
    maxRequestBytes?: number;
    maxResponseBytes?: number;
    requestTimeoutMs?: number;
  } = {}
): Promise<UnaryFixture> {
  const selectedBinding = limits.binding ?? BINDING;
  const assemblyIdentity = limits.assemblyIdentity ?? ASSEMBLY;
  const generation = limits.generation ?? 7;
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    ingress: new RuntimeAssemblyIngressIndex(
      limits.bindings ?? [selectedBinding]
    )
  });
  const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
  const runtimeRegistry = new RuntimeRegistry();
  const endpoint = new RuntimeEndpoint({
    registry: runtimeRegistry,
    assemblyRegistry,
    bootstrap: {
      artifactsPath: '/tmp/skiff-test-artifacts',
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
      http: { maxResponseBytes: 67108864 }
    }
  });
  const dispatcher = new RuntimeDispatcher({ registry: assemblyRegistry, frameSender: endpoint });
  endpoint.setDispatcher(dispatcher);
  const runtimeListen = await endpoint.listen({ port: 0 });
  const gateway = new AssemblyHttpGateway({
    snapshots,
    dispatcher,
    port: 0,
    maxRequestBytes: limits.maxRequestBytes ?? 67108864,
    maxResponseBytes: limits.maxResponseBytes ?? 67108864,
    requestTimeoutMs: limits.requestTimeoutMs ?? 1000
  });
  const httpListen = await gateway.listen();
  const runtime = await openSocket(runtimeListen.url);
  runtime.send(encodeRuntimeFrame({
    ...runtimeFrameHeaderFixtures['runtime.capabilities'],
    runtimeId: RUNTIME_ID
  }));
  runtime.send(encodeAssemblyActivationFrame('runtimeToRouter', {
    type: 'register',
    environment: 'test',
    generation,
    assembly: { assemblyIdentity },
    replicaId: RUNTIME_ID
  }));
  await until(() => assemblyRegistry.healthyParticipantReplicaIds().includes(RUNTIME_ID));

  const fixture: UnaryFixture = {
    binding: selectedBinding,
    dispatcher,
    endpoint,
    gateway,
    httpUrl: httpListen.url,
    runtime,
    snapshot: snapshots.get(),
    close: async () => {
      await gateway.close();
      await endpoint.close();
    }
  };
  fixtures.push(fixture);
  return fixture;
}

function canonicalHeader(
  snapshot: RouterActiveAssemblySnapshot,
  requestId: string
): RuntimeAssemblyRequestStartFrameHeader {
  return assemblyHttpRequestHeader({
    snapshot,
    binding: BINDING,
    requestId,
    timeoutMs: 1000,
    httpRequest: requestMetadata()
  });
}

function requestMetadata() {
  return {
    method: 'POST',
    url: `http://${HOST}${PATH}`,
    path: PATH,
    query: [],
    headers: []
  };
}

function mutate(
  source: RuntimeAssemblyRequestStartFrameHeader,
  change: (header: Record<string, any>) => void
): RuntimeAssemblyRequestStartFrameHeader {
  const header = structuredClone(source) as unknown as Record<string, any>;
  change(header);
  return header as unknown as RuntimeAssemblyRequestStartFrameHeader;
}

async function sendHttp(
  baseUrl: string,
  body: Uint8Array,
  query = '',
  selector: {
    method: string | null;
    path: string;
    serviceId?: string;
    contractVersion?: string;
  } = { method: 'POST', path: PATH }
): Promise<{ status: number; headers: Record<string, string | string[] | undefined>; body: Buffer }> {
  return await startHttp(baseUrl, body, query, selector).response;
}

function startHttp(
  baseUrl: string,
  body: Uint8Array,
  query = '',
  selector: {
    method: string | null;
    path: string;
    serviceId?: string;
    contractVersion?: string;
  } = { method: 'POST', path: PATH }
): {
  request: ReturnType<typeof httpRequest>;
  response: Promise<{
    status: number;
    headers: Record<string, string | string[] | undefined>;
    body: Buffer;
  }>;
} {
  const base = new URL(baseUrl);
  let outgoing: ReturnType<typeof httpRequest>;
  const response = new Promise<{
    status: number;
    headers: Record<string, string | string[] | undefined>;
    body: Buffer;
  }>((resolve, reject) => {
    outgoing = httpRequest({
      hostname: base.hostname,
      port: base.port,
      path: `${selector.path}${query}`,
      method: selector.method ?? 'POST',
      headers: {
        host: HOST,
        'x-skiff-service':
          selector.serviceId ?? BINDING.deployment.serviceId,
        'x-skiff-version':
          selector.contractVersion ?? BINDING.deployment.contractVersion,
        'content-length': String(body.byteLength)
      }
    }, (response) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
      response.on('end', () => resolve({
        status: response.statusCode ?? 0,
        headers: response.headers,
        body: Buffer.concat(chunks)
      }));
    });
    outgoing.on('error', reject);
    outgoing.end(body);
  });
  return { request: outgoing!, response };
}

async function openSocket(url: string): Promise<WebSocket> {
  const ws = new WebSocket(url);
  await new Promise<void>((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
  return ws;
}

async function nextBinaryMessage(ws: WebSocket): Promise<Buffer> {
  return await new Promise<Buffer>((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for binary frame')), 1000);
    ws.once('message', (data, isBinary) => {
      clearTimeout(timeout);
      if (!isBinary) {
        reject(new Error('expected binary runtime frame'));
        return;
      }
      resolve(rawDataBuffer(data));
    });
  });
}

function rawDataBuffer(data: WebSocket.RawData): Buffer {
  if (Array.isArray(data)) return Buffer.concat(data);
  if (data instanceof ArrayBuffer) return Buffer.from(new Uint8Array(data));
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength);
}

function spyOnFinishPending(dispatcher: RuntimeDispatcher) {
  return vi.spyOn(
    dispatcher as unknown as {
      finishPending(requestId: string, ...args: unknown[]): void;
    },
    'finishPending'
  );
}

function finishPendingCalls(
  spy: ReturnType<typeof spyOnFinishPending>,
  requestId: string
): number {
  return spy.mock.calls.filter(([candidate]) => candidate === requestId).length;
}

function observeRequestCancels(
  ws: WebSocket,
  requestId: string
): { count(): number; stop(): void } {
  let count = 0;
  const listener = (data: WebSocket.RawData, isBinary: boolean) => {
    if (!isBinary) {
      return;
    }
    const frame = decodeRuntimeFrame(rawDataBuffer(data));
    if (
      frame.header.type === 'request.cancel' &&
      frame.header.requestId === requestId
    ) {
      count += 1;
    }
  };
  ws.on('message', listener);
  return {
    count: () => count,
    stop: () => ws.off('message', listener)
  };
}

async function until(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return;
    await nextTurn();
  }
  throw new Error('condition was not reached');
}

async function nextTurn(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve));
}

function fixedServicePayload(
  kind: 'publicTypedError' | 'internalError' | 'platformError',
  traceId: string,
  errorId: string
): Uint8Array {
  const privateBytes = Array.from(Buffer.from(PRIVATE_SENTINELS.join('|'), 'utf8'));
  const envelope =
    kind === 'publicTypedError'
      ? {
          kind,
          packageId: 'example.com/errors',
          stableSchemaKey: 'private-failure',
          packageSchemaTypeId: 'type:private-failure',
          encodedPayload: privateBytes,
          traceId,
          errorId
        }
      : kind === 'internalError'
        ? {
            kind,
            payload: {
              message: PRIVATE_SENTINELS.join('|'),
              traceId,
              errorId
            }
          }
        : {
            kind,
            builtinErrorIdentity: 'std.db.ConflictError',
            encodedPayload: privateBytes,
            traceId,
            errorId
          };
  return Buffer.from(JSON.stringify(envelope), 'utf8');
}

function assertNoPrivateSentinels(value: string): void {
  for (const sentinel of PRIVATE_SENTINELS) {
    expect(value).not.toContain(sentinel);
  }
}
