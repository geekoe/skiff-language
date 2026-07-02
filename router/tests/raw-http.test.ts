import { afterEach, describe, expect, it } from 'vitest';
import { request as createHttpRequest } from 'node:http';

import { buildActivationLookup } from '../src/artifacts/activationLookup.js';
import {
  loadManifest as loadRuntimeManifest,
  packageHttpHandlerTarget,
  loadManifestFile,
  mergeLoadedManifests
} from '../src/manifest/loadManifest.js';
import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  isRecord,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RuntimeRegisterEnvelope
} from '../src/protocol/envelope.js';
import { DEFAULT_HTTP_BODY_LIMIT_BYTES } from '../src/router/httpGateway.js';
import {
  DEFAULT_TEST_BUILD_ID,
  loadHttpRouteManifest,
  httpRequestSchema,
  httpResponseSchema,
  loadRawHttpManifest,
  loadRawHttpStreamManifest,
  withBuildId
} from './helpers/manifests.js';
import { RouterHarness } from './helpers/routerHarness.js';
import {
  closeTrackedResources,
  type RuntimeRequestFrame
} from './helpers/runtime.js';

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
    operation.operationAbiId = `operation:test:${target}`;
  });
}

describe('router raw HTTP gateway', () => {
  it('adds CORS headers to regular HTTP API responses with an Origin', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-cors',
      targets: manifest.operations.map((operation) => operation.target)
    });
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      method: request.header.httpRequest?.method,
      path: request.header.httpRequest?.path
    }));

    const origin = 'http://localhost:3000';
    const getResponse = await harness.requestHttp({
      path: '/packages/list?service=skiff.run/sample',
      method: 'GET',
      headers: {
        Origin: origin,
        'X-Skiff-User-Id': 'local-user'
      }
    });
    const postResponse = await harness.requestHttp({
      path: '/packages/publish?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Origin: origin,
        'Content-Type': 'application/json',
        Authorization: 'Bearer local-token'
      },
      body: '{"name":"demo"}'
    });

    expect(getResponse.status).toBe(200);
    expect(getResponse.headers['access-control-allow-origin']).toBe(origin);
    expect(getResponse.headers['access-control-allow-credentials']).toBe('true');
    expect(getResponse.headers.vary).toContain('Origin');
    expect(JSON.parse(getResponse.body)).toEqual({
      method: 'GET',
      path: '/packages/list'
    });
    expect(postResponse.status).toBe(200);
    expect(postResponse.headers['access-control-allow-origin']).toBe(origin);
    expect(postResponse.headers['access-control-allow-credentials']).toBe('true');
    expect(postResponse.headers.vary).toContain('Origin');
    expect(JSON.parse(postResponse.body)).toEqual({
      method: 'POST',
      path: '/packages/publish'
    });
  });

  it('answers CORS preflight requests without dispatching to a runtime', async () => {
    const harness = await RouterHarness.rawHttp();

    const response = await harness.requestHttp({
      path: '/packages/list?service=skiff.run/sample',
      method: 'OPTIONS',
      headers: {
        Origin: 'http://localhost:3000',
        'Access-Control-Request-Method': 'POST',
        'Access-Control-Request-Headers': 'content-type, authorization'
      }
    });

    expect(response.status).toBe(204);
    expect(response.body).toBe('');
    expect(response.headers['access-control-allow-origin']).toBe('http://localhost:3000');
    expect(response.headers['access-control-allow-credentials']).toBe('true');
    expect(response.headers['access-control-allow-methods']).toContain('POST');
    expect(response.headers['access-control-allow-methods']).toContain('OPTIONS');
    expect(response.headers['access-control-allow-headers']).toBe(
      'content-type, authorization'
    );
    expect(response.headers.vary).toContain('Origin');
    expect(response.headers.vary).toContain('Access-Control-Request-Method');
    expect(response.headers.vary).toContain('Access-Control-Request-Headers');
  });

  it('adds CORS headers to HTTP API error responses with an Origin', async () => {
    const harness = await RouterHarness.rawHttp();

    const response = await harness.requestHttp({
      path: '/packages/list?service=NotAService',
      method: 'GET',
      headers: {
        Origin: 'http://localhost:3000'
      }
    });

    expect(response.status).toBe(400);
    expect(response.headers['access-control-allow-origin']).toBe('http://localhost:3000');
    expect(response.headers['access-control-allow-credentials']).toBe('true');
    expect(response.headers.vary).toContain('Origin');
    expect(JSON.parse(response.body)).toEqual({
      message: 'service query must be a valid publication id',
      detail: null
    });
  });

  it('sends HTTP ingress as a binary request.start frame with metadata header and raw body payload', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-binary-ingress',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const framesPromise = runtime.collectRequestFrames(1, 'binary raw HTTP ingress');
    runtime.respondWithHttpFrame({
      status: 204
    });

    const body = Buffer.from([0, 1, 2, 123, 34, 255]);
    const response = await harness.requestHttp({
      path: '/binary/a%20b?service=skiff.run/sample&x=1&x=2&flag=',
      method: 'POST',
      headers: {
        Host: 'SAMPLE.LOCAL',
        'Content-Type': 'application/octet-stream',
        'X-Request-Id': 'client-binary-1'
      },
      body
    });
    const [frame] = await framesPromise;

    expect(response.status).toBe(204);
    expect(frame).toBeDefined();
    expect(frame!.payloadBytes).toEqual(body);
    expect(frame!.header).toMatchObject({
      schemaVersion: 'skiff-runtime-frame-v1',
      type: 'request.start',
      mode: 'unary',
      caller: {
        kind: 'gateway',
        target: 'gateway.skiff~run~~sample.http.raw'
      },
      target: 'service.skiff~run~~sample.SampleHttpApi.handle',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      httpRequest: {
        method: 'POST',
        url: 'http://sample.local/binary/a%20b?service=skiff.run/sample&x=1&x=2&flag=',
        path: '/binary/a b',
        query: [
          { name: 'service', value: 'skiff.run/sample' },
          { name: 'x', value: '1' },
          { name: 'x', value: '2' },
          { name: 'flag', value: '' }
        ]
      }
    });
    expect(frame!.header).not.toHaveProperty('args');
    expect(JSON.stringify(frame!.header)).not.toContain('__skiffBytesBase64');
    const httpRequest = frame!.header.httpRequest;
    expect(isRecord(httpRequest)).toBe(true);
    if (!isRecord(httpRequest)) {
      throw new Error('expected HTTP request metadata in request.start frame header');
    }
    expect(httpRequest).not.toHaveProperty('body');
    expect(httpRequest.headers).toEqual(
      expect.arrayContaining([
        { name: 'host', value: 'SAMPLE.LOCAL' },
        { name: 'content-type', value: 'application/octet-stream' },
        { name: 'x-request-id', value: 'client-binary-1' }
      ])
    );
  });

  it('allows requests above the old 1 MiB limit with the default HTTP body limit', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-default-body-limit',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const framesPromise = runtime.collectRequestFrames(1, 'default body limit raw HTTP ingress');
    runtime.respondWithHttpFrame({ status: 204 });

    const body = Buffer.alloc(1024 * 1024 + 1, 65);
    const response = await harness.requestHttp({
      path: '/large?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
        'Content-Type': 'application/octet-stream'
      },
      body
    });
    const [frame] = await framesPromise;

    expect(DEFAULT_HTTP_BODY_LIMIT_BYTES).toBe(64 * 1024 * 1024);
    expect(response.status).toBe(204);
    expect(frame!.payloadBytes.byteLength).toBe(body.byteLength);
    expect(frame!.payloadBytes[0]).toBe(65);
    expect(frame!.payloadBytes[frame!.payloadBytes.byteLength - 1]).toBe(65);
  });

  it('uses host and path rewrite before client selectors', async () => {
    const accountManifest = loadRawHttpManifest({ serviceId: 'skiff.run/account' });
    const registryManifest = loadRawHttpManifest({
      serviceId: 'skiff.run/registry',
      protocolIdentity:
        'skiff-protocol-v1:sha256:6666666666666666666666666666666666666666666666666666666666666666'
    });
    const manifest = mergeLoadedManifests([accountManifest, registryManifest]);
    const harness = await RouterHarness.create({ manifest });
    await harness.listenHttp({
      rewrite: [
        {
          host: 'account.localhost',
          path: '/api',
          service: 'skiff.run/account'
        },
        {
          host: 'account.localhost',
          service: 'skiff.run/registry'
        }
      ]
    });

    const accountRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-http-rewrite-account',
      serviceId: accountManifest.service.id,
      revisionId: accountManifest.service.revisionId,
      serviceProtocolIdentity: accountManifest.service.protocolIdentity,
      targets: accountManifest.operations.map((operation) => operation.target)
    });
    const registryRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-http-rewrite-registry',
      serviceId: registryManifest.service.id,
      revisionId: registryManifest.service.revisionId,
      serviceProtocolIdentity: registryManifest.service.protocolIdentity,
      targets: registryManifest.operations.map((operation) => operation.target)
    });
    accountRuntime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      path: request.header.httpRequest?.path
    }));
    registryRuntime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      path: request.header.httpRequest?.path
    }));

    const exact = await harness.requestHttp({
      path: '/api?service=skiff.run/registry',
      headers: {
        Host: 'Account.Localhost:4000',
        'X-Skiff-Service': 'skiff.run/registry'
      }
    });
    const fallback = await harness.requestHttp({
      path: '/other?service=skiff.run/account',
      headers: {
        Host: 'account.localhost'
      }
    });

    expect(JSON.parse(exact.body)).toEqual({
      serviceId: 'skiff.run/account',
      path: '/api'
    });
    expect(JSON.parse(fallback.body)).toEqual({
      serviceId: 'skiff.run/registry',
      path: '/other'
    });
  });

  it('uses exact rewrite path matching before host fallback', async () => {
    const accountManifest = loadRawHttpManifest({ serviceId: 'skiff.run/account' });
    const fallbackManifest = loadRawHttpManifest({
      serviceId: 'skiff.run/fallback',
      protocolIdentity:
        'skiff-protocol-v1:sha256:7777777777777777777777777777777777777777777777777777777777777777'
    });
    const manifest = mergeLoadedManifests([accountManifest, fallbackManifest]);
    const harness = await RouterHarness.create({ manifest });
    await harness.listenHttp({
      rewrite: [
        {
          host: 'account.localhost',
          path: '/api',
          service: 'skiff.run/account'
        },
        {
          host: 'account.localhost',
          service: 'skiff.run/fallback'
        }
      ]
    });

    const accountRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-http-rewrite-exact-account',
      serviceId: accountManifest.service.id,
      revisionId: accountManifest.service.revisionId,
      serviceProtocolIdentity: accountManifest.service.protocolIdentity,
      targets: accountManifest.operations.map((operation) => operation.target)
    });
    const fallbackRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-http-rewrite-exact-fallback',
      serviceId: fallbackManifest.service.id,
      revisionId: fallbackManifest.service.revisionId,
      serviceProtocolIdentity: fallbackManifest.service.protocolIdentity,
      targets: fallbackManifest.operations.map((operation) => operation.target)
    });
    accountRuntime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      path: request.header.httpRequest?.path
    }));
    fallbackRuntime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      path: request.header.httpRequest?.path
    }));

    const exact = await harness.requestHttp({
      path: '/api',
      headers: { Host: 'account.localhost' }
    });
    const trailingSlash = await harness.requestHttp({
      path: '/api/',
      headers: { Host: 'account.localhost' }
    });
    const prefix = await harness.requestHttp({
      path: '/api/users',
      headers: { Host: 'account.localhost' }
    });

    expect(JSON.parse(exact.body)).toEqual({
      serviceId: 'skiff.run/account',
      path: '/api'
    });
    expect(JSON.parse(trailingSlash.body)).toEqual({
      serviceId: 'skiff.run/fallback',
      path: '/api/'
    });
    expect(JSON.parse(prefix.body)).toEqual({
      serviceId: 'skiff.run/fallback',
      path: '/api/users'
    });
  });

  it('writes HTTP egress from a binary response.end frame with raw body payload', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-binary-egress',
      targets: manifest.operations.map((operation) => operation.target)
    });
    runtime.respondWithHttpFrame({
      status: 202,
      headers: [
        { name: 'content-type', value: 'application/octet-stream' },
        { name: 'set-cookie', value: 'raw_session=abc; Path=/; HttpOnly' },
        { name: 'set-cookie', value: 'raw_theme=dark; Path=/' }
      ],
      body: Buffer.from([255, 0, 1, 2, 123, 34])
    });

    const response = await harness.requestHttp({
      path: '/binary-response?service=skiff.run/sample',
      headers: {
        Host: 'sample.local',
      }
    });

    expect(response.status).toBe(202);
    expect(response.headers['content-type']).toBe('application/octet-stream');
    expect(response.headers['set-cookie']).toEqual([
      'raw_session=abc; Path=/; HttpOnly',
      'raw_theme=dark; Path=/'
    ]);
    expect(response.rawBody).toEqual(Buffer.from([255, 0, 1, 2, 123, 34]));
  });

  it('writes raw HTTP serverStream start chunks and end in frame order', async () => {
    const manifest = loadRawHttpStreamManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-stream',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const framesPromise = runtime.collectRequestFrames(1, 'streaming raw HTTP ingress');
    runtime.onRequestFrame((frame) => {
      runtime.ws.send(
        encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.start',
          requestId: frame.header.requestId,
          httpResponse: {
            status: 202,
            headers: [
              { name: 'content-type', value: 'text/plain' },
              { name: 'x-stream', value: 'yes' }
            ]
          }
        })
      );
      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.chunk',
            requestId: frame.header.requestId,
            seq: 0
          },
          Buffer.from('hello ')
        )
      );
      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.chunk',
            requestId: frame.header.requestId,
            seq: 1
          },
          Buffer.from('stream')
        )
      );
      runtime.ws.send(
        encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: frame.header.requestId,
          payloadPresent: false
        })
      );
    });

    const response = await harness.requestHttp({
      path: '/stream?service=skiff.run/sample',
      method: 'POST',
      body: 'ignored'
    });
    const [frame] = await framesPromise;

    expect(frame?.header.mode).toBe('serverStream');
    expect(response.status).toBe(202);
    expect(response.headers['content-type']).toBe('text/plain');
    expect(response.headers['x-stream']).toBe('yes');
    expect(response.body).toBe('hello stream');
  });

  it('cancels raw HTTP serverStream dispatch when the client disconnects', async () => {
    const manifest = loadRawHttpStreamManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-stream-cancel',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const cancelPromise = new Promise<{ requestId: string; reason: string }>((resolve, reject) => {
      let cleanup = () => {};
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error('timed out waiting for request.cancel'));
      }, 1000);
      const onMessage = (data: unknown) => {
        try {
          const frame = decodeRuntimeFrame(data as Parameters<typeof decodeRuntimeFrame>[0]);
          if (frame.header.type === 'request.cancel') {
            cleanup();
            resolve({
              requestId: frame.header.requestId,
              reason: frame.header.reason
            });
          }
        } catch {
          // Ignore non-frame messages while the runtime is registering.
        }
      };
      cleanup = () => {
        clearTimeout(timeout);
        runtime.ws.off('message', onMessage);
      };
      runtime.ws.on('message', onMessage);
    });
    runtime.onRequestFrame((frame) => {
      runtime.ws.send(
        encodeRuntimeFrame({
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.start',
          requestId: frame.header.requestId,
          httpResponse: {
            status: 200,
            headers: [{ name: 'content-type', value: 'text/plain' }]
          }
        })
      );
      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'response.chunk',
            requestId: frame.header.requestId,
            seq: 0
          },
          Buffer.from('partial')
        )
      );
    });

    await new Promise<void>((resolve, reject) => {
      const request = createHttpRequest(
        harness.httpUrl('/stream-cancel?service=skiff.run/sample'),
        {
          method: 'POST'
        },
        (response) => {
          response.once('data', () => {
            request.destroy();
            resolve();
          });
        }
      );
      request.on('error', (error: NodeJS.ErrnoException) => {
        if (error.code !== 'ECONNRESET') {
          reject(error);
        }
      });
      request.end('ignored');
    });
    const cancel = await cancelPromise;

    expect(cancel.reason).toBe('client_disconnect');
  });

  it('maps serverStream runtime errors before response.start to platform errors', async () => {
    const manifest = loadRawHttpStreamManifest();
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-stream-error',
      targets: manifest.operations.map((operation) => operation.target)
    });
    runtime.onRequestFrame((frame) => {
      runtime.sendError(frame.header.requestId, {
        code: 'StreamBoom',
        message: 'stream failed before start'
      });
    });

    const response = await harness.requestHttp({
      path: '/stream?service=skiff.run/sample',
      method: 'POST'
    });

    expect(response.status).toBe(500);
    expect(JSON.parse(response.body)).toEqual({
      message: 'stream failed before start',
      detail: null
    });
  });


  it('dispatches manifest HTTP routes by method and path before raw fallback', async () => {
    const manifest = loadHttpRouteManifest();
    const harness = await RouterHarness.http({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-routes',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const requestsPromise = runtime.collectRequestFrames(2, 'manifest HTTP route dispatch');
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      caller: request.header.caller.target,
      target: request.header.target,
      path: request.header.httpRequest?.path
    }));

    const sessionResponse = await harness.requestHttp({
      path: '/session?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
      },
      body: '{"session":true}'
    });
    const trackResponse = await harness.requestHttp({
      path: '/track?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
      }
    });
    const unknownResponse = await harness.requestHttp({
      path: '/legacy?service=skiff.run/sample',
      method: 'GET',
      headers: {
        Host: 'sample.local',
      }
    });
    const [sessionRequest, trackRequest] = await requestsPromise;

    expect(sessionResponse.status).toBe(200);
    expect(JSON.parse(sessionResponse.body)).toEqual({
      caller: 'gateway.skiff~run~~sample.http.post.session',
      target: 'service.skiff~run~~sample.SessionApi.handle',
      path: '/session'
    });
    expect(trackResponse.status).toBe(200);
    expect(JSON.parse(trackResponse.body)).toEqual({
      caller: 'gateway.skiff~run~~sample.http.post.track',
      target: 'service.skiff~run~~sample.TrackApi.handle',
      path: '/track'
    });
    expect(unknownResponse.status).toBe(404);
    expect(JSON.parse(unknownResponse.body)).toEqual({
      message: 'No HTTP route is loaded for GET /legacy',
      detail: null
    });

    expect(sessionRequest?.header).toMatchObject({
      caller: {
        kind: 'gateway',
        target: 'gateway.skiff~run~~sample.http.post.session'
      },
      target: 'service.skiff~run~~sample.SessionApi.handle',
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
    expect(trackRequest?.header).toMatchObject({
      caller: {
        kind: 'gateway',
        target: 'gateway.skiff~run~~sample.http.post.track'
      },
      target: 'service.skiff~run~~sample.TrackApi.handle'
    });
  });

  it('forwards typed HTTP adapter metadata on route dispatch frames', async () => {
    const typedTarget = 'service.skiff~run~~sample.internal.todos.create';
    const manifest = withBuildId(loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/sample',
        revisionId: '6666666666666666666666666666666666666666666666666666666666666666',
        protocolIdentity: 'skiff-protocol-v1:sha256:7777777777777777777777777777777777777777777777777777777777777777'
      },
      operations: [
        {
          operation: 'internal.todos.create',
          target: typedTarget,
          mode: 'unary',
          parameters: [
            {
              name: 'input',
              schema: { type: 'object' }
            }
          ],
          response: { type: 'object' }
        }
      ],
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/todos',
              operation: 'internal.todos.create',
              operationAbiId: `operation:test:${typedTarget}`,
              target: typedTarget,
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
                  'skiff-http-ingress-v1:sha256:7777777777777777777777777777777777777777777777777777777777777777',
                adapter: {
                  kind: 'typedJson',
                  handler: {
                    kind: 'serviceFunction',
                    modulePath: 'internal.todos',
                    symbol: 'create'
                  },
                  adapterArgs: [
                    { param: 'input', source: { kind: 'http.body' } }
                  ]
                }
              }
            }
          ]
        }
      }
    }));
    const harness = await RouterHarness.http({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-typed-http-adapter',
      targets: [typedTarget]
    });
    const requestsPromise = runtime.collectRequestFrames(1, 'typed HTTP adapter dispatch');
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      target: request.header.target,
      adapter: request.header.httpAdapter
    }));

    const body = Buffer.from('{"title":"Ship adapter"}');
    const response = await harness.requestHttp({
      path: '/todos?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
        'Content-Type': 'application/json'
      },
      body
    });
    const [request] = await requestsPromise;

    expect(response.status).toBe(200);
    expect(request?.payloadBytes).toEqual(body);
    expect(request?.header).toMatchObject({
      caller: {
        kind: 'gateway',
        target: 'gateway.skiff~run~~sample.http.post.todos'
      },
      target: typedTarget,
      httpAdapter: {
        kind: 'typedJson',
        handler: {
          kind: 'serviceFunction',
          modulePath: 'internal.todos',
          symbol: 'create'
        },
        adapterArgs: [
          { param: 'input', source: { kind: 'http.body' } }
        ]
      }
    });
    expect(JSON.parse(response.body)).toEqual({
      target: typedTarget,
      adapter: {
        kind: 'typedJson',
        handler: {
          kind: 'serviceFunction',
          modulePath: 'internal.todos',
          symbol: 'create'
        },
        adapterArgs: [
          { param: 'input', source: { kind: 'http.body' } }
        ]
      }
    });
  });


  it('returns 404 for an unknown path when a selected service declares HTTP routes without raw fallback', async () => {
    const manifest = loadHttpRouteManifest();
    const loaded = loadManifest({
      ...manifest,
      gateway: {
        http: {
          routes: manifest.gateway?.http?.routes
        }
      }
    });
    const harness = await RouterHarness.http({ manifest: withBuildId(loaded) });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-http-routes-no-raw',
      targets: loaded.operations.map((operation) => operation.target)
    });
    runtime.respondHttpEmpty();

    const response = await harness.requestHttp({
      path: '/not-mapped?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
      }
    });

    expect(response.status).toBe(404);
    expect(JSON.parse(response.body)).toEqual({
      message: 'No HTTP route is loaded for POST /not-mapped',
      detail: null
    });
  });

  it('dispatches package handler HTTP routes without requiring an app operation', async () => {
    const packageTarget = packageHttpHandlerTarget('skiff.run/http-session', 'issue');
    const manifest = withBuildId(loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/sample',
        revisionId: '4444444444444444444444444444444444444444444444444444444444444444',
        protocolIdentity: 'skiff-protocol-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555'
      },
      operations: [
        {
          operation: 'SampleHttpApi.handle',
          target: 'service.skiff~run~~sample.SampleHttpApi.handle',
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
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/session',
              handler: {
                kind: 'packageFunction',
                source: 'httpSession.issue',
                packageId: 'skiff.run/http-session',
                alias: 'httpSession',
                symbolPath: 'issue'
              }
            }
          ],
          raw: {
            operation: 'SampleHttpApi.handle',
            target: 'gateway.skiff~run~~sample.http.raw'
          }
        }
      }
    }));
    const route = manifest.httpRouteEntries[0];
    expect(route).toBeDefined();
    const harness = await RouterHarness.http({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-package-http-route',
      targets: [packageTarget, ...manifest.operations.map((operation) => operation.target)]
    });
    const requestsPromise = runtime.collectRequestFrames(1, 'package HTTP route dispatch');
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      target: request.header.target,
      path: request.header.httpRequest?.path
    }));

    const response = await harness.requestHttp({
      path: '/session?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
      }
    });
    const [request] = await requestsPromise;

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toEqual({
      target: packageTarget,
      path: '/session'
    });
    expect(request?.header).toMatchObject({
      target: packageTarget,
      operationAbiId: route!.operationAbiId,
      selector: route!.selector,
      httpRequest: {
        path: '/session'
      }
    });
    expect(request?.header.operationAbiId).toMatch(/^operation:http-route:[0-9a-f]{64}$/);
    expect(request?.header.operationAbiId).not.toBe(packageTarget);
    expect(request?.header.selector).toBe('POST /session');
    expect(request?.header.selector).not.toBe(packageTarget);
  });

  it('loads package-only HTTP routes without service operations', () => {
    const packageTarget = packageHttpHandlerTarget('skiff.run/http-session', 'issue');
    const manifest = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/sample',
        revisionId: '5555555555555555555555555555555555555555555555555555555555555555',
        protocolIdentity: 'skiff-protocol-v1:sha256:6666666666666666666666666666666666666666666666666666666666666666'
      },
      operations: [],
      gateway: {
        http: {
          routes: [
            {
              method: 'POST',
              path: '/session',
              handler: {
                kind: 'packageFunction',
                packageId: 'skiff.run/http-session',
                symbolPath: 'issue'
              }
            }
          ]
        }
      }
    });

    expect(manifest.operations).toEqual([]);
    expect(manifest.httpRouteEntries[0]).toMatchObject({
      path: '/session',
      method: 'POST',
      dispatchTarget: packageTarget,
      operationAbiId: expect.stringMatching(/^operation:http-route:[0-9a-f]{64}$/),
      selector: 'POST /session',
      requestParameterName: 'request'
    });
  });


  it('dispatches service-selected raw HTTP as a standard HttpRequest envelope', async () => {
    const manifest = withBuildId(await loadManifestFile('fixtures/hello/manifest.json'));
    const harness = await RouterHarness.http({ manifest });

    const register: RuntimeRegisterEnvelope = {
      type: 'runtime.register',
      runtimeId: 'runtime-test-1',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    };
    const runtime = await harness.registerRuntime(register);

    const seenRequests: RuntimeRequestFrame[] = [];
    runtime.onRequestFrame((request) => {
      seenRequests.push(request);
      runtime.sendHttpFrameResponse({
        requestId: request.header.requestId,
        status: 202,
        headers: [
          { name: 'content-type', value: 'application/json; charset=utf-8' },
          { name: 'set-cookie', value: 'raw_session=abc; Path=/; HttpOnly' },
          { name: 'set-cookie', value: 'raw_theme=dark; Path=/' }
        ],
        body: JSON.stringify({
          ok: true,
          target: request.header.target
        })
      });
    });

    const response = await harness.requestHttp({
      path: '/raw/a%20b?service=skiff.run/hello&x=1&x=2&flag=',
      method: 'POST',
      headers: {
        Host: 'HELLO.LOCAL',
        'Content-Type': 'application/json',
        'X-Request-Id': 'client-request-1'
      },
      body: JSON.stringify({ message: 'hello runtime', extra: true })
    });

    expect(response.status).toBe(202);
    expect(response.headers['set-cookie']).toEqual([
      'raw_session=abc; Path=/; HttpOnly',
      'raw_theme=dark; Path=/'
    ]);
    expect(JSON.parse(response.body)).toEqual({
      ok: true,
      target: 'service.skiff~run~~hello.HelloHttpApi.handle'
    });

    expect(seenRequests).toHaveLength(1);
    const firstRequest = seenRequests[0];
    expect(firstRequest).toBeDefined();
    expect(firstRequest?.header).toMatchObject({
      type: 'request.start',
      mode: 'unary',
      caller: {
        kind: 'gateway',
        target: 'gateway.skiff~run~~hello.http.raw'
      },
      target: 'service.skiff~run~~hello.HelloHttpApi.handle',
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      httpRequest: {
        method: 'POST',
        url: 'http://hello.local/raw/a%20b?service=skiff.run/hello&x=1&x=2&flag=',
        path: '/raw/a b',
        query: [
          { name: 'service', value: 'skiff.run/hello' },
          { name: 'x', value: '1' },
          { name: 'x', value: '2' },
          { name: 'flag', value: '' }
        ]
      }
    });
    expect(firstRequest?.header.gatewayEntryIdentity).toBeUndefined();
    expect(firstRequest?.payloadBytes).toEqual(
      Buffer.from(JSON.stringify({ message: 'hello runtime', extra: true }))
    );
    expect(firstRequest?.header.httpRequest).not.toHaveProperty('body');
    const headers = firstRequest!.header.httpRequest!.headers;
    expect(headers).toEqual(
      expect.arrayContaining([
        { name: 'host', value: 'HELLO.LOCAL' },
        { name: 'content-type', value: 'application/json' },
        { name: 'x-request-id', value: 'client-request-1' }
      ])
    );
  });


  it('adds activationIdentity from the loaded activation lookup to raw HTTP dispatch', async () => {
    const manifest = loadRawHttpManifest();
    const activationIdentity = 'skiff-runtime-activation-v1:opaque:http-loaded-activation';
    const harness = await RouterHarness.rawHttp({
      manifest,
      activationByServiceOperation: buildActivationLookup([
        {
          buildId: manifest.rawHttpEntries[0]!.buildId!,
          manifestValue: manifest,
          serviceVersion: '0.1.0',
          sourcePath: 'test',
          activation: {
            operationTargets: ['service.skiff~run~~sample.SampleHttpApi.handle'],
            serviceId: manifest.service.id,
            payload: {
              serviceId: manifest.service.id,
              buildId: manifest.rawHttpEntries[0]!.buildId!,
              activationIdentity,
              resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:test',
              resolvedConfig: {},
              redactedResolvedConfig: {},
              redactionProjectionIdentity:
                'skiff-config-redaction-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000',
              configShape: { schemaVersion: 'skiff-config-shape-v1', entries: [] }
            }
          }
        }
      ])
    });

    const defaultRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-http-default-activation',
      revisionId: 'revision-shared',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const activatedRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-http-loaded-activation',
      revisionId: 'revision-shared',
      activationIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });

    const defaultRequests: RuntimeRequestFrame[] = [];
    defaultRuntime.onRequestFrame((request) => {
      defaultRequests.push(request);
    });
    const activatedRequestsPromise = activatedRuntime.collectRequestFrames(
      1,
      'activated raw HTTP dispatch'
    );
    activatedRuntime.respondHttpJson((request: RuntimeRequestFrame) => ({
      activationIdentity: request.header.activationIdentity
    }));

    const response = await harness.requestHttp({
      path: '/activation?service=skiff.run/sample',
      headers: {
        Host: 'sample.local',
      }
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toEqual({ activationIdentity });
    const [request] = await activatedRequestsPromise;
    expect(request).toBeDefined();
    expect(request!.header.activationIdentity).toBe(activationIdentity);
    expect(defaultRequests).toHaveLength(0);
  });


  it('resolves raw HTTP activation by service and operation when services share a protocol', async () => {
    const sharedProtocolIdentity =
      'skiff-protocol-v1:sha256:6666666666666666666666666666666666666666666666666666666666666666';
    const serviceA = loadRawHttpManifest({
      serviceId: 'skiff.run/sample-a',
      protocolIdentity: sharedProtocolIdentity
    });
    const serviceB = loadRawHttpManifest({
      serviceId: 'skiff.run/sample-b',
      protocolIdentity: sharedProtocolIdentity
    });
    const manifest = mergeLoadedManifests([serviceA, serviceB]);
    const activationA = 'skiff-runtime-activation-v1:opaque:http-shared-activation-a';
    const harness = await RouterHarness.rawHttp({
      manifest,
      activationByServiceOperation: buildActivationLookup([
        {
          buildId: serviceA.rawHttpEntries[0]!.buildId!,
          manifestValue: serviceA,
          serviceVersion: '0.1.0',
          sourcePath: 'test-a',
          activation: {
            operationTargets: [serviceA.operations[0]!.target],
            serviceId: serviceA.service.id,
            payload: {
              serviceId: serviceA.service.id,
              buildId: serviceA.rawHttpEntries[0]!.buildId!,
              activationIdentity: activationA,
              resolvedConfigIdentity: 'skiff-config-resolved-v1:opaque:test-a',
              resolvedConfig: {},
              redactedResolvedConfig: {},
              redactionProjectionIdentity:
                'skiff-config-redaction-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000',
              configShape: { schemaVersion: 'skiff-config-shape-v1', entries: [] }
            }
          }
        }
      ])
    });

    const serviceADefault = await harness.registerRuntime({
      runtimeId: 'runtime-http-shared-a-default',
      serviceId: serviceA.service.id,
      revisionId: serviceA.service.revisionId,
      serviceProtocolIdentity: sharedProtocolIdentity,
      targets: serviceA.operations.map((operation) => operation.target)
    });
    const serviceAActivated = await harness.registerRuntime({
      runtimeId: 'runtime-http-shared-a-activated',
      serviceId: serviceA.service.id,
      revisionId: serviceA.service.revisionId,
      activationIdentity: activationA,
      serviceProtocolIdentity: sharedProtocolIdentity,
      targets: serviceA.operations.map((operation) => operation.target)
    });
    const serviceBDefault = await harness.registerRuntime({
      runtimeId: 'runtime-http-shared-b-default',
      serviceId: serviceB.service.id,
      revisionId: serviceB.service.revisionId,
      serviceProtocolIdentity: sharedProtocolIdentity,
      targets: serviceB.operations.map((operation) => operation.target)
    });

    const serviceADefaultRequests: RuntimeRequestFrame[] = [];
    serviceADefault.onRequestFrame((request) => {
      serviceADefaultRequests.push(request);
    });
    serviceAActivated.respondWithActivationIdentity();
    serviceBDefault.respondWithActivationIdentity();
    const serviceARequests = serviceAActivated.collectRequestFrames(
      1,
      'activated shared service A raw HTTP dispatch'
    );
    const serviceBRequests = serviceBDefault.collectRequestFrames(
      1,
      'default shared service B raw HTTP dispatch'
    );

    const responseA = await harness.requestHttp({
      path: `/activation-a?service=${serviceA.service.id}`,
      headers: {
        Host: 'sample-a.local',
      }
    });
    const responseB = await harness.requestHttp({
      path: `/activation-b?service=${serviceB.service.id}`,
      headers: {
        Host: 'sample-b.local',
      }
    });

    expect(responseA.status).toBe(200);
    expect(JSON.parse(responseA.body)).toEqual({ activationIdentity: activationA });
    expect(responseB.status).toBe(200);
    expect(JSON.parse(responseB.body)).toEqual({});
    const [requestA] = await serviceARequests;
    const [requestB] = await serviceBRequests;
    expect(requestA?.header.activationIdentity).toBe(activationA);
    expect(requestB?.header.activationIdentity).toBeUndefined();
    expect(serviceADefaultRequests).toHaveLength(0);
  });


  it('uses X-Skiff-Service as a raw HTTP dispatch selector', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.http({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-hello-service-header'
    });
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      path: request.header.httpRequest?.path,
      query: request.header.httpRequest?.query
    }));

    const missingResponse = await harness.requestHttp({
      path: '/hello/Ada',
      headers: {
        Host: 'hello.local'
      }
    });
    expect(missingResponse.status).toBe(404);
    expect(JSON.parse(missingResponse.body)).toEqual({
      message: 'No service selector is available for raw dispatch',
      detail: null
    });

    const headerResponse = await harness.requestHttp({
      path: '/hello/Ada',
      headers: {
        Host: 'hello.local',
        'X-Skiff-Service': 'skiff.run/hello'
      }
    });
    expect(headerResponse.status).toBe(200);
    expect(JSON.parse(headerResponse.body)).toEqual({
      serviceId: 'skiff.run/hello',
      path: '/hello/Ada',
      query: []
    });

    const invalidResponse = await harness.requestHttp({
      path: '/hello/Ada',
      headers: {
        Host: 'hello.local',
        'X-Skiff-Service': 'Hello'
      }
    });
    expect(invalidResponse.status).toBe(400);
    expect(JSON.parse(invalidResponse.body)).toEqual({
      message: 'X-Skiff-Service must be a valid publication id',
      detail: null
    });
  });

  it('rejects duplicate service query selectors only when X-Skiff-Service is absent', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.http({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-hello-service-header-query'
    });
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      query: request.header.httpRequest?.query
    }));

    const duplicateResponse = await harness.requestHttp({
      path: '/hello/Ada?service=skiff.run/hello&service=skiff.run/hello',
      headers: {
        Host: 'hello.local'
      }
    });
    expect(duplicateResponse.status).toBe(400);
    expect(JSON.parse(duplicateResponse.body)).toEqual({
      message: 'service query parameter must be singular',
      detail: null
    });

    const headerResponse = await harness.requestHttp({
      path: '/hello/Ada?service=business-value&service=another-business-value',
      headers: {
        Host: 'hello.local',
        'X-Skiff-Service': 'skiff.run/hello'
      }
    });
    expect(headerResponse.status).toBe(200);
    expect(JSON.parse(headerResponse.body)).toEqual({
      serviceId: 'skiff.run/hello',
      query: [
        { name: 'service', value: 'business-value' },
        { name: 'service', value: 'another-business-value' }
      ]
    });
  });

  it('does not treat version query as a selector outside release routing', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.http({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-hello-version-query'
    });
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      serviceId: request.header.serviceId,
      query: request.header.httpRequest?.query
    }));

    const response = await harness.requestHttp({
      path: '/hello/Ada?version=business-a&version=business-b',
      headers: {
        Host: 'hello.local',
        'X-Skiff-Service': 'skiff.run/hello'
      }
    });
    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toEqual({
      serviceId: 'skiff.run/hello',
      query: [
        { name: 'version', value: 'business-a' },
        { name: 'version', value: 'business-b' }
      ]
    });
  });

  it('dispatches raw HTTP with typed service route target and selected buildId', async () => {
    const typedTarget = 'runtime.sample.SampleHttpApi.handle';
    const buildId =
      'skiff-service-build-v1:sha256:abababababababababababababababababababababababababababababababab';
    const manifest = withBuildId(
      loadManifest({
        schemaVersion: 'skiff-runtime-manifest-v1',
        service: {
          id: 'skiff.run/sample',
          revisionId: '6666666666666666666666666666666666666666666666666666666666666666',
          protocolIdentity:
            'skiff-protocol-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555'
        },
        operations: [
          {
            operation: 'SampleHttpApi.handle',
            target: typedTarget,
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
      }),
      buildId
    );
    const harness = await RouterHarness.rawHttp({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-typed-raw-http',
      buildId,
      targets: [typedTarget]
    });
    const requestsPromise = runtime.collectRequestFrames(1, 'typed raw HTTP route');
    runtime.respondHttpJson((request: RuntimeRequestFrame) => ({
      buildId: request.header.buildId,
      target: request.header.target
    }));

    const response = await harness.requestHttp({
      path: '/typed-route?service=skiff.run/sample',
      headers: {
        Host: 'sample.local',
      }
    });
    const [request] = await requestsPromise;

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toEqual({ buildId, target: typedTarget });
    expect(request?.header).toMatchObject({
      buildId,
      target: typedTarget
    });
  });


  it('passes form, multipart, and text request bodies as raw bytes', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-body-variants',
      targets: manifest.operations.map((operation) => operation.target)
    });

    const requestsPromise = runtime.collectRequestFrames(3, 'raw HTTP body variants');
    runtime.respondHttpEmpty();

    const formBody = 'a=1&a=2&empty=';
    const textBody = 'raw bytes';
    const multipartBody = [
      '--test-boundary',
      'content-disposition: form-data; name="title"',
      '',
      'hello',
      '--test-boundary',
      'content-disposition: form-data; name="file"; filename="a.txt"',
      'content-type: text/plain',
      '',
      'file body',
      '--test-boundary--',
      ''
    ].join('\r\n');

    await harness.requestHttp({
      path: '/form?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
        'Content-Type': 'application/x-www-form-urlencoded'
      },
      body: formBody
    });
    await harness.requestHttp({
      path: '/bytes?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
      },
      body: textBody
    });
    await harness.requestHttp({
      path: '/multipart?service=skiff.run/sample',
      method: 'POST',
      headers: {
        Host: 'sample.local',
        'Content-Type': 'multipart/form-data; boundary=test-boundary'
      },
      body: multipartBody
    });

    const requests = await requestsPromise;
    expect(requests.map((request) => Buffer.from(request.payloadBytes))).toEqual([
      Buffer.from(formBody),
      Buffer.from(textBody),
      Buffer.from(multipartBody)
    ]);
    for (const request of requests) {
      expect(request.header.httpRequest).not.toHaveProperty('body');
    }
  });


  it('dispatches every selected service request to the raw HTTP operation', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-raw',
      targets: manifest.operations.map((operation) => operation.target)
    });

    const seenRequests: RuntimeRequestFrame[] = [];
    runtime.onRequestFrame((request) => {
      seenRequests.push(request);
      runtime.sendHttpFrameResponse({
        requestId: request.header.requestId,
        status: 202,
        headers: [
          { name: 'content-type', value: 'text/plain; charset=utf-8' },
          { name: 'set-cookie', value: 'raw_session=abc; Path=/; HttpOnly' },
          { name: 'set-cookie', value: 'raw_theme=dark; Path=/' }
        ],
        body: 'raw-ok'
      });
    });

    const rawResponse = await harness.requestHttp({
      path: '/typed/route-id?service=skiff.run/sample',
      headers: {
        Host: 'SAMPLE.LOCAL',
      }
    });

    expect(rawResponse.status).toBe(202);
    expect(rawResponse.body).toBe('raw-ok');
    expect(rawResponse.headers['set-cookie']).toEqual([
      'raw_session=abc; Path=/; HttpOnly',
      'raw_theme=dark; Path=/'
    ]);
    expect(seenRequests.map((request) => request.header.target)).toEqual([
      'service.skiff~run~~sample.SampleHttpApi.handle'
    ]);
    expect(seenRequests[0]?.header).toMatchObject({
      caller: {
        target: 'gateway.skiff~run~~sample.http.raw'
      },
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      httpRequest: {
        method: 'GET',
        path: '/typed/route-id',
        query: [{ name: 'service', value: 'skiff.run/sample' }]
      }
    });
    expect(seenRequests[0]?.payloadBytes.byteLength).toBe(0);
    expect(seenRequests[0]?.header.httpRequest).not.toHaveProperty('body');
  });


  it('uses service query for dispatch and X-Forwarded-Host for request URL', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-forwarded-host',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const requestsPromise = runtime.collectRequestFrames(1, 'forwarded host raw request');
    runtime.respondHttpJson({ ok: true });

    const response = await harness.requestHttp({
      path: '/from-forwarded-host?service=skiff.run/sample',
      headers: {
        Host: 'HOST.EXAMPLE',
        'X-Forwarded-Host': 'FORWARDED.EXAMPLE:3011'
      }
    });

    expect(response.status).toBe(200);
    await expect(Promise.resolve(JSON.parse(response.body))).resolves.toEqual({ ok: true });
    const [request] = await requestsPromise;
    expect(request?.header).toMatchObject({
      caller: {
        target: 'gateway.skiff~run~~sample.http.raw'
      },
      target: 'service.skiff~run~~sample.SampleHttpApi.handle',
      httpRequest: {
        url: 'http://forwarded.example:3011/from-forwarded-host?service=skiff.run/sample',
        path: '/from-forwarded-host'
      }
    });
  });


  it('keeps Host only as request URL data and not as dispatch routing', async () => {
    const manifest = loadRawHttpManifest();
    const harness = await RouterHarness.rawHttp({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-sample-local-port',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const requestsPromise = runtime.collectRequestFrames(2, 'host URL data requests');
    runtime.respondHttpEmpty();

    const portHostResponse = await harness.requestHttp({
      path: '/raw-local-port?service=skiff.run/sample',
      headers: {
        Host: 'LOCALHOST:3011',
      }
    });
    expect(portHostResponse.status).toBe(204);

    const noPortResponse = await harness.requestHttp({
      path: '/raw-local-port?service=skiff.run/sample',
      headers: {
        Host: 'localhost',
      }
    });
    expect(noPortResponse.status).toBe(204);

    const requests = await requestsPromise;
    expect(requests.map((request) => request.header.httpRequest?.url)).toEqual([
      'http://localhost:3011/raw-local-port?service=skiff.run/sample',
      'http://localhost/raw-local-port?service=skiff.run/sample'
    ]);
  });


  it('does not dispatch raw-shaped HTTP operations without explicit raw metadata', async () => {
    const manifest = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/sample',
        revisionId: '7777777777777777777777777777777777777777777777777777777777777777',
        protocolIdentity: 'skiff-protocol-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333'
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
        http: {}
      }
    });
    const harness = await RouterHarness.http({ manifest });

    const response = await harness.requestHttp({
      path: '/raw-without-metadata?service=skiff.run/sample',
      headers: {
        Host: 'sample.local',
      }
    });

    expect(response.status).toBe(404);
    expect(JSON.parse(response.body)).toEqual({
      message: 'No raw HTTP service is loaded for skiff.run/sample',
      detail: null
    });
  });


  it('uses explicit raw HTTP metadata instead of scanning multiple raw-shaped operations', async () => {
    const manifest = loadManifest({
      schemaVersion: 'skiff-runtime-manifest-v1',
      service: {
        id: 'skiff.run/sample',
        revisionId: '8888888888888888888888888888888888888888888888888888888888888888',
        protocolIdentity: 'skiff-protocol-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444'
      },
      operations: [
        {
          operation: 'SampleHttpApi.handle',
          target: 'service.skiff~run~~sample.SampleHttpApi.handle',
          mode: 'unary',
          parameters: [{ name: 'request', schema: httpRequestSchema() }],
          response: httpResponseSchema()
        },
        {
          operation: 'AdminHttpApi.handle',
          target: 'service.skiff~run~~sample.AdminHttpApi.handle',
          mode: 'unary',
          parameters: [{ name: 'request', schema: httpRequestSchema() }],
          response: httpResponseSchema()
        }
      ],
      gateway: {
        http: {
          raw: {
            operation: 'AdminHttpApi.handle',
            target: 'gateway.skiff~run~~sample.http.raw'
          }
        }
      }
    });
    const harness = await RouterHarness.http({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-explicit-raw',
      targets: manifest.operations.map((operation) => operation.target)
    });
    const requestsPromise = runtime.collectRequestFrames(1, 'explicit raw metadata request');
    runtime.respondHttpEmpty();

    const response = await harness.requestHttp({
      path: '/explicit-raw?service=skiff.run/sample',
      headers: {
        Host: 'sample.local',
      }
    });
    expect(response.status).toBe(204);
    const [request] = await requestsPromise;
    expect(request).toBeDefined();
    if (!request) {
      throw new Error('expected explicit raw metadata request');
    }
    expect(request.header.target).toBe('service.skiff~run~~sample.AdminHttpApi.handle');
    expect(request.header.caller).toEqual({
      kind: 'gateway',
      target: 'gateway.skiff~run~~sample.http.raw'
    });
  });
});
