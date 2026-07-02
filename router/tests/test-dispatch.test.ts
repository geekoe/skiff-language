import { afterEach, describe, expect, it } from 'vitest';

import { ActivationLookup } from '../src/artifacts/activationLookup.js';
import {
  decodeRuntimeFrame,
  type PackageTestStartFrameHeader,
  type RuntimeBinaryFrame,
  type RuntimeFrameHeader
} from '../src/protocol/envelope.js';
import {
  RouterActiveSnapshotStore,
  type RouterActiveSnapshot
} from '../src/router/activeSnapshot.js';
import { RouterControlPlane } from '../src/router/controlPlane.js';
import {
  DEFAULT_TEST_BUILD_ID,
  loadRawHttpManifest
} from './helpers/manifests.js';
import { requestHttp } from './helpers/request.js';
import {
  MockRuntime,
  closeTrackedResources,
  createRuntimeRouter,
  trackResource
} from './helpers/runtime.js';

afterEach(closeTrackedResources);

const PACKAGE_TEST_BODY = {
  kind: 'packageTest',
  packageId: 'skiff.run/agent',
  packageVersion: '0.1.0',
  testBuildIdentity:
    'skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  entrypointId:
    'skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
  activationId: 'skiff-package-test-run-v1:skiff.run~agent:aaaaaaaa:run-1:1'
} as const;

function collectPackageTestFrame(
  runtime: MockRuntime,
  label: string
): Promise<RuntimeBinaryFrame<PackageTestStartFrameHeader>> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: Parameters<typeof decodeRuntimeFrame>[0]) => {
      let frame: RuntimeBinaryFrame<RuntimeFrameHeader>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch (error) {
        cleanup();
        reject(error);
        return;
      }
      if (frame.header.type !== 'package-test.start') {
        return;
      }
      cleanup();
      resolve(frame as RuntimeBinaryFrame<PackageTestStartFrameHeader>);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      runtime.ws.off('message', onMessage);
    };
    runtime.ws.on('message', onMessage);
  });
}

describe('router test dispatch control endpoint', () => {
  it('rejects non-POST methods', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const snapshotStore = new RouterActiveSnapshotStore({
      activationByServiceOperation: new ActivationLookup(),
      manifest
    });
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'GET'
    });

    expect(response.status).toBe(405);
    expect(response.headers.allow).toBe('POST');
    expect(JSON.parse(response.body)).toMatchObject({
      error: {
        code: 'MethodNotAllowed',
        message: 'test dispatch requires POST'
      }
    });
  });

  it('dispatches a binary request through a registered runtime and returns the binary response', async () => {
    const manifest = loadRawHttpManifest();
    const operation = manifest.operations[0]!;
    const activationByServiceOperation = new ActivationLookup();
    activationByServiceOperation.set({
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      target: operation.target,
      activationIdentity: 'skiff-runtime-activation-v1:opaque:test-dispatch'
    });
    const snapshot: RouterActiveSnapshot = {
      activationByServiceOperation,
      manifest
    };
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore(snapshot)
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const runtime = await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-test-dispatch',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((entry) => entry.target),
      activationIdentity: 'skiff-runtime-activation-v1:opaque:test-dispatch'
    });
    runtime.onRequestFrame((request) => {
      runtime.sendBinaryResponse(request.header.requestId, request.payloadBytes);
    });
    const requestFrames = runtime.collectRequestFrames(1, 'test dispatch request');
    const payloadBase64 = Buffer.from('test dispatch payload').toString('base64');
    const websocketEntryId = 'websocket-entry-test-dispatch';
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        target: operation.target,
        payloadBase64,
        websocketEntryId,
        testEffectsEnabled: true,
        testEffectDoubles: {
          'service.skiff~run~~dependency.Dependency.call': [
            {
              expectRequest: { id: 'req-1' },
              response: { ok: true }
            }
          ]
        }
      })
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toMatchObject({
      ok: true,
      header: {
        type: 'response.end',
        payloadPresent: true
      },
      payloadBase64
    });
    const [requestFrame] = await requestFrames;
    expect(Buffer.from(requestFrame!.payloadBytes).toString('utf8')).toBe('test dispatch payload');
    expect(requestFrame!.header).toMatchObject({
      type: 'request.start',
      mode: operation.mode,
      caller: {
        kind: 'gateway',
        target: '__skiff.test-dispatch'
      },
      target: operation.target,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      activationIdentity: 'skiff-runtime-activation-v1:opaque:test-dispatch',
      websocketEntryId,
      testEffectsEnabled: true,
      testEffectDoubles: {
        'service.skiff~run~~dependency.Dependency.call': [
          {
            expectRequest: { id: 'req-1' },
            response: { ok: true }
          }
        ]
      }
    });
    expect(requestFrame!.header.deadline?.timeoutMs).toBe(operation.timeoutMs);
  });

  it('returns runtime response.error frames without converting them to HTTP errors', async () => {
    const manifest = loadRawHttpManifest();
    const operation = manifest.operations[0]!;
    const activationByServiceOperation = new ActivationLookup();
    activationByServiceOperation.set({
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      target: operation.target,
      activationIdentity: 'skiff-runtime-activation-v1:opaque:test-dispatch-error'
    });
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation,
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const runtime = await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-test-dispatch-error',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((entry) => entry.target),
      activationIdentity: 'skiff-runtime-activation-v1:opaque:test-dispatch-error'
    });
    runtime.onRequestFrame((request) => {
      runtime.sendError(request.header.requestId, {
        code: 'UnhandledServiceError',
        message: 'unhandled user exception std.json.DecodeError',
        details: {
          actualPayloadType: 'std.json.DecodeError'
        }
      });
    });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        target: operation.target
      })
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toMatchObject({
      ok: true,
      header: {
        type: 'response.error',
        error: {
          code: 'UnhandledServiceError',
          message: 'unhandled user exception std.json.DecodeError'
        }
      },
      payloadBase64: ''
    });
  });

  it('allows explicit mode and timeout when the active manifest cannot resolve the target', async () => {
    const manifest = loadRawHttpManifest();
    const target = 'custom.test.target';
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const runtime = await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-test-dispatch-explicit',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    runtime.respondWithBinaryJsonPayload({ ok: true });
    const requestFrames = runtime.collectRequestFrames(1, 'explicit test dispatch request');
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        target,
        operationAbiId: `operation:test:${target}`,
        mode: 'unary',
        timeoutMs: 1234
      })
    });

    expect(response.status).toBe(200);
    const [requestFrame] = await requestFrames;
    expect(requestFrame!.header).toMatchObject({
      target,
      mode: 'unary'
    });
    expect(requestFrame!.header.deadline?.timeoutMs).toBe(1234);
  });

  it('uses the explicit runtime address instead of a colliding active manifest target', async () => {
    const manifest = loadRawHttpManifest();
    const operation = manifest.operations[0]!;
    const explicitProtocolIdentity =
      'skiff-protocol-v1:sha256:6666666666666666666666666666666666666666666666666666666666666666';
    const activationByServiceOperation = new ActivationLookup();
    activationByServiceOperation.set({
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      target: operation.target,
      activationIdentity: 'skiff-runtime-activation-v1:opaque:colliding-active-manifest'
    });
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation,
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const runtime = await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-test-dispatch-explicit-collision',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: explicitProtocolIdentity,
      targets: [operation.target]
    });
    runtime.respondWithBinaryJsonPayload({ ok: true });
    const requestFrames = runtime.collectRequestFrames(
      1,
      'explicit colliding test dispatch request'
    );
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceId: manifest.service.id,
        serviceProtocolIdentity: explicitProtocolIdentity,
        target: operation.target,
        operationAbiId: operation.operationAbiId,
        mode: 'unary',
        timeoutMs: 1234
      })
    });

    expect(response.status).toBe(200);
    const [requestFrame] = await requestFrames;
    expect(requestFrame!.header).toMatchObject({
      target: operation.target,
      serviceId: manifest.service.id,
      serviceProtocolIdentity: explicitProtocolIdentity,
      mode: 'unary'
    });
    expect(requestFrame!.header).not.toHaveProperty('activationIdentity');
    expect(requestFrame!.header.deadline?.timeoutMs).toBe(1234);
  });

  it('rejects unknown test dispatch kinds before service resolution', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        kind: 'notARealKind',
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        target: manifest.operations[0]!.target
      })
    });

    expect(response.status).toBe(400);
    expect(JSON.parse(response.body)).toMatchObject({
      error: {
        code: 'InvalidTestDispatchKind'
      }
    });
  });

  it('rejects package-only fields on explicit service test dispatch', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        kind: 'service',
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        target: manifest.operations[0]!.target,
        packageId: PACKAGE_TEST_BODY.packageId
      })
    });

    expect(response.status).toBe(400);
    expect(JSON.parse(response.body)).toMatchObject({
      error: {
        code: 'InvalidTestDispatchRequest',
        message: 'packageId is not supported for service test dispatch'
      }
    });
  });

  it('rejects service-only fields on package test dispatch', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        ...PACKAGE_TEST_BODY,
        target: manifest.operations[0]!.target
      })
    });

    expect(response.status).toBe(400);
    expect(JSON.parse(response.body)).toMatchObject({
      error: {
        code: 'InvalidTestDispatchRequest',
        message: 'target is not supported for packageTest test dispatch'
      }
    });
  });

  it('rejects unknown fields on package test dispatch', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        ...PACKAGE_TEST_BODY,
        operationName: 'root.internal.helper'
      })
    });

    expect(response.status).toBe(400);
    expect(JSON.parse(response.body)).toMatchObject({
      error: {
        code: 'InvalidTestDispatchRequest',
        message: 'operationName is not supported for packageTest test dispatch'
      }
    });
  });

  it('fails closed when no runtime advertises packageTestDispatch', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-without-package-test-capability',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((entry) => entry.target)
    });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify(PACKAGE_TEST_BODY)
    });

    expect(response.status).toBe(503);
    expect(JSON.parse(response.body)).toMatchObject({
      error: {
        code: 'std.service.ProviderUnavailableError',
        message:
          'No runtime with packageTestDispatch capability is registered for package test dispatch'
      }
    });
  });

  it('dispatches package tests only to runtimes with packageTestDispatch capability', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-service-only-for-package-test',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((entry) => entry.target)
    });
    const runtime = await MockRuntime.register(listen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-package-test-capable',
      serviceId: manifest.service.id,
      revisionId: `${manifest.service.revisionId}-package-test`,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((entry) => entry.target),
      capabilities: {
        packageTestDispatch: true
      }
    });
    const packageFrame = collectPackageTestFrame(runtime, 'package test dispatch frame');
    runtime.ws.on('message', (data) => {
      const frame = decodeRuntimeFrame(data);
      if (frame.header.type !== 'package-test.start') {
        return;
      }
      runtime.sendBinaryResponse(frame.header.requestId, 'package-test-ok');
    });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        ...PACKAGE_TEST_BODY,
        payloadBase64: Buffer.from('package test payload').toString('base64'),
        timeoutMs: 1234,
        testEffectsEnabled: true,
        testEffectDoubles: {
          'package.skiff~run~~agent.test.entrypoint': [
            {
              response: { ok: true }
            }
          ]
        }
      })
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toMatchObject({
      ok: true,
      header: {
        type: 'response.end',
        payloadPresent: true
      },
      payloadBase64: Buffer.from('package-test-ok').toString('base64')
    });

    const frame = await packageFrame;
    expect(Buffer.from(frame.payloadBytes).toString('utf8')).toBe('package test payload');
    expect(frame.header).toMatchObject({
      type: 'package-test.start',
      caller: {
        kind: 'gateway',
        target: '__skiff.test-dispatch'
      },
      packageId: PACKAGE_TEST_BODY.packageId,
      packageVersion: PACKAGE_TEST_BODY.packageVersion,
      testBuildIdentity: PACKAGE_TEST_BODY.testBuildIdentity,
      entrypointId: PACKAGE_TEST_BODY.entrypointId,
      activationId: PACKAGE_TEST_BODY.activationId,
      testEffectsEnabled: true,
      testEffectDoubles: {
        'package.skiff~run~~agent.test.entrypoint': [
          {
            response: { ok: true }
          }
        ]
      }
    });
    expect(frame.header.deadline?.timeoutMs).toBe(1234);
    expect(frame.header).not.toHaveProperty('serviceId');
    expect(frame.header).not.toHaveProperty('operation');
    expect(frame.header).not.toHaveProperty('operationAbiId');
    expect(frame.header).not.toHaveProperty('target');
  });

  it('dispatches package tests to runtime-level capability registrations without service routes', async () => {
    const manifest = loadRawHttpManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore: new RouterActiveSnapshotStore({
        activationByServiceOperation: new ActivationLookup(),
        manifest
      })
    });
    const listen = await endpoint.listen({ port: 0, controlPlane });
    const runtime = await MockRuntime.capabilities(listen.url, {
      type: 'runtime.capabilities',
      runtimeId: 'runtime-package-test-capability-only',
      capabilities: {
        packageTestDispatch: true
      }
    });
    const packageFrame = collectPackageTestFrame(
      runtime,
      'runtime-level package test dispatch frame'
    );
    runtime.ws.on('message', (data) => {
      const frame = decodeRuntimeFrame(data);
      if (frame.header.type !== 'package-test.start') {
        return;
      }
      runtime.sendBinaryResponse(frame.header.requestId, 'runtime-level-package-test-ok');
    });
    const controlUrl = listen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/test-dispatch`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify(PACKAGE_TEST_BODY)
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toMatchObject({
      ok: true,
      header: {
        type: 'response.end',
        payloadPresent: true
      },
      payloadBase64: Buffer.from('runtime-level-package-test-ok').toString('base64')
    });

    const frame = await packageFrame;
    expect(frame.header).toMatchObject({
      type: 'package-test.start',
      packageId: PACKAGE_TEST_BODY.packageId,
      testBuildIdentity: PACKAGE_TEST_BODY.testBuildIdentity,
      entrypointId: PACKAGE_TEST_BODY.entrypointId
    });
  });
});
