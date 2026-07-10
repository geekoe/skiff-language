import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { ActivationLookup } from '../src/artifacts/activationLookup.js';
import { loadManifestFile } from '../src/manifest/loadManifest.js';
import {
  decodeRuntimeFrame,
  encodeRuntimeFrame,
  type DispatchMode,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RequestCancelFrameHeader,
  type RequestStartFrameHeader,
  type ResponseChunkFrameHeader,
  type ResponseEndFrameHeader,
  type ResponseErrorFrameHeader,
  type ResponseStartFrameHeader
} from '../src/protocol/envelope.js';
import {
  RouterActiveSnapshotStore,
  type RouterActiveSnapshot
} from '../src/router/activeSnapshot.js';
import { RouterControlPlane } from '../src/router/controlPlane.js';
import type { RuntimeDispatcher } from '../src/router/runtimeDispatcher.js';
import { closeSocket, delay, onceWithTimeout } from './helpers/events.js';
import { findRuntime, hasRuntime, readHealth, waitForRuntimeAbsent } from './helpers/health.js';
import { DEFAULT_TEST_BUILD_ID, loadWebSocketManifest } from './helpers/manifests.js';
import { requestHttp } from './helpers/request.js';
import { RouterHarness } from './helpers/routerHarness.js';
import {
  closeTrackedResources,
  createRequestStart,
  createRuntimeRouter,
  openBinaryRegisteredRuntime,
  openRegisteredRuntime,
  sendRuntimeBinaryResponse,
  trackResource,
  type RequestStartEnvelope,
  type RuntimeRequestFrame,
  waitForRuntimeRequestFrame
} from './helpers/runtime.js';

afterEach(closeTrackedResources);

describe('router runtime registry dispatch', () => {

  it('returns 503 when no runtime supports the exact protocol identity', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.http({ manifest });

    const response = await harness.requestHttp({
      path: '/hello/Ada?service=skiff.run/hello',
      headers: {
        Host: 'hello.local',
      }
    });
    expect(response.status).toBe(503);
    expect(JSON.parse(response.body)).toEqual({
      message: 'No runtime is registered for the requested service operation',
      detail: null
    });
  });

  it('dispatches build misses to an open runtime connection for lazy loading', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const runtime = new WebSocket(harness.registryListen!.url);
    trackResource({ close: () => runtime.close() });
    await onceWithTimeout(runtime, 'open', 'lazy runtime socket open');

    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const lazyBuildId =
      'skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd';
    const requestFrame = waitForRuntimeRequestFrame(runtime, 'request-lazy-load');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-lazy-load',
        target,
        serviceId: manifest.service.id,
        buildId: lazyBuildId,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );

    const frame = await requestFrame;
    expect(frame.header.serviceId).toBe(manifest.service.id);
    expect(frame.header.buildId).toBe(lazyBuildId);
    sendRuntimeBinaryResponse(
      runtime,
      frame.header.requestId,
      Buffer.from(JSON.stringify({ runtimeId: 'lazy-runtime' }))
    );

    await expect(dispatch).resolves.toEqual({ runtimeId: 'lazy-runtime' });
  });

  it('dispatches version-addressed lazy loads with the resolved current build id', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const runtime = new WebSocket(harness.registryListen!.url);
    trackResource({ close: () => runtime.close() });
    await onceWithTimeout(runtime, 'open', 'lazy runtime socket open');

    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const serviceId = manifest.service.id;
    const currentBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000010cc';
    const staleBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000010dd';
    registry.setServiceVersionIndex(
      new Map([[serviceId, new Map([['0.1.0', { buildId: currentBuild }]])]])
    );

    const requestFrame = waitForRuntimeRequestFrame(runtime, 'request-lazy-version-load');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-lazy-version-load',
        target,
        serviceId,
        version: '0.1.0',
        buildId: staleBuild,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );

    const frame = await requestFrame;
    expect(frame.header).toMatchObject({
      requestId: 'request-lazy-version-load',
      serviceId,
      version: '0.1.0',
      buildId: currentBuild
    });
    sendRuntimeBinaryResponse(
      runtime,
      frame.header.requestId,
      Buffer.from(JSON.stringify({ runtimeId: 'lazy-current-runtime' }))
    );

    await expect(dispatch).resolves.toEqual({ runtimeId: 'lazy-current-runtime' });
  });

  it('lazy-loads a different activation on a connection with the same target build registered', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const activationA = 'skiff-runtime-activation-v1:opaque:lazy-activation-a';
    const activationB = 'skiff-runtime-activation-v1:opaque:lazy-activation-b';

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-lazy-activation-a',
      revisionId: 'revision-lazy-activation',
      activationIdentity: activationA,
      targets: [target]
    });

    const requestFrame = runtime.waitForRequestFrame('request-lazy-activation-b');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-lazy-activation-b',
        target,
        serviceId: manifest.service.id,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        activationIdentity: activationB
      }),
      2000
    );

    const frame = await requestFrame;
    expect(frame.header).toMatchObject({
      requestId: 'request-lazy-activation-b',
      target,
      serviceId: manifest.service.id,
      activationIdentity: activationB
    });
    runtime.sendBinaryJsonResponse(frame.header.requestId, {
      runtimeId: 'runtime-lazy-activation-b',
      activationIdentity: frame.header.activationIdentity
    });

    await expect(dispatch).resolves.toEqual({
      runtimeId: 'runtime-lazy-activation-b',
      activationIdentity: activationB
    });
  });

  it('prefers an exact registered runtime over lazy-load fallback connections', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const lazyRuntime = new WebSocket(harness.registryListen!.url);
    trackResource({ close: () => lazyRuntime.close() });
    await onceWithTimeout(lazyRuntime, 'open', 'lazy fallback socket open');

    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const exactRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-exact-lazy-priority',
      targets: [target]
    });
    exactRuntime.respondWithBinaryJsonPayload({ runtimeId: 'runtime-exact-lazy-priority' });

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-exact-lazy-priority',
          target,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-exact-lazy-priority' });
  });

  it('prunes runtime registrations outside the keep set', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const staleBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000030aa';
    const currentBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000030bb';

    const staleRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-prune-stale',
      revisionId: 'revision-prune-stale',
      buildId: staleBuild,
      targets: [target]
    });
    const currentRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-prune-current',
      revisionId: 'revision-prune-current',
      buildId: currentBuild,
      targets: [target]
    });

    const result = registry.pruneRuntimes({
      keep: [{ serviceId: manifest.service.id, buildId: currentBuild }]
    });

    expect(result.deleted.map((runtime) => runtime.runtimeId)).toEqual([
      'runtime-prune-stale'
    ]);
    expect(result.kept.map((runtime) => runtime.runtimeId)).toEqual([
      'runtime-prune-current'
    ]);
    expect(hasRuntime(registry.snapshot(), 'runtime-prune-stale')).toBe(false);
    expect(findRuntime(registry.snapshot(), 'runtime-prune-current')).toMatchObject({
      active: true,
      buildId: currentBuild
    });
    expect(() =>
      registry.validateRuntimeRequestStartSource(
        staleRuntime.ws,
        serviceRequestStart({
          requestId: 'runtime-prune-stale-source',
          callerTarget: target,
          target,
          serviceId: manifest.service.id,
          buildId: staleBuild,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        })
      )
    ).toThrow('runtime-originated request.start requires a registered runtime for the caller target');

    currentRuntime.respondWithBinaryJsonPayload({
      runtimeId: 'runtime-prune-current'
    });
    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-prune-current',
          target,
          serviceId: manifest.service.id,
          buildId: currentBuild,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-prune-current' });
  });


  it('rejects legacy text request.start dispatch without sending a runtime message', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-text-dispatch-rejected',
      targets: ['service.skiff~run~~hello.HelloApi.hello']
    });
    let runtimeMessageSent = false;
    runtime.ws.on('message', () => {
      runtimeMessageSent = true;
    });

    await expect(
      dispatcher.dispatch(
        createRequestStart({
          requestId: 'request-text-dispatch-rejected',
          target: 'service.skiff~run~~hello.HelloApi.hello',
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).rejects.toMatchObject({
      statusCode: 502,
      code: 'UnsupportedRuntimeTransport',
      message: 'text JSON request.start is not supported; use typed binary runtime frames'
    });

    await delay(10);
    expect(runtimeMessageSent).toBe(false);
  });


  it('falls back to identity-less route keys for gateway entry dispatch to legacy runtimes', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;

    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-no-gateway-entry',
      targets: [target]
    });

    runtime.respondWithBinaryJsonPayload((request: RuntimeRequestFrame) => ({
      runtimeId: 'runtime-no-gateway-entry',
      gatewayEntryIdentity: request.header.gatewayEntryIdentity
    }));

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-gateway-entry',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          gatewayEntryIdentity:
            'skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111'
        }),
        2000
      )
    ).resolves.toEqual({
      runtimeId: 'runtime-no-gateway-entry',
      gatewayEntryIdentity:
        'skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111'
    });
  });


  it('sends new requests to a new same-protocol revision while old pending work completes', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.http({ manifest });
    const { dispatcher, registry } = harness;

    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const runtimeV1 = await harness.registerRuntime({
      runtimeId: 'runtime-revision-1',
      revisionId: 'revision-1',
      targets: [target],
      protocolVersion: 'skiff-protocol-v1',
      runtimeVersion: '1.0.0',
      codeRevisionId: 'code-1',
      artifactIdentity: 'artifact-1',
      capabilities: {
        dispatchModes: ['unary']
      }
    });

    const firstRequestPromise = runtimeV1.waitForRequestFrame('request-old');
    const firstDispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-old',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );
    const firstRequest = await firstRequestPromise;
    expect(firstRequest.header.requestId).toBe('request-old');

    const runtimeV2 = await harness.registerRuntime({
      runtimeId: 'runtime-revision-2',
      revisionId: 'revision-2',
      targets: [target],
      protocolVersion: 'skiff-protocol-v1',
      runtimeVersion: '2.0.0',
      codeRevisionId: 'code-2',
      artifactIdentity: 'artifact-2'
    });

    const healthWhileDraining = await readHealth(harness.registryListen!.url.replace('ws://', 'http://').replace('/runtime', ''));
    const v1WhileDraining = findRuntime(healthWhileDraining, 'runtime-revision-1');
    const v2Active = findRuntime(healthWhileDraining, 'runtime-revision-2');
    expect(v1WhileDraining).toMatchObject({
      revisionState: 'draining',
      active: false,
      draining: true,
      inFlightCount: 1,
      registeredAt: expect.any(String),
      protocolVersion: 'skiff-protocol-v1',
      runtimeVersion: '1.0.0',
      codeRevisionId: 'code-1',
      artifactIdentity: 'artifact-1',
      capabilities: {
        dispatchModes: ['unary']
      }
    });
    expect(v2Active).toMatchObject({
      revisionState: 'active',
      active: true,
      draining: false,
      inFlightCount: 0,
      runtimeVersion: '2.0.0'
    });

    const secondRequestPromise = runtimeV2.waitForRequestFrame('request-new');
    const secondDispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-new',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );
    const secondRequest = await secondRequestPromise;
    runtimeV2.sendBinaryJsonResponse(secondRequest.header.requestId, { revision: 'revision-2' });
    await expect(secondDispatch).resolves.toEqual({ revision: 'revision-2' });

    runtimeV1.sendBinaryJsonResponse(firstRequest.header.requestId, { revision: 'revision-1' });
    await expect(firstDispatch).resolves.toEqual({ revision: 'revision-1' });

    const healthAfterDrain = await readHealth(harness.registryListen!.url.replace('ws://', 'http://').replace('/runtime', ''));
    expect(findRuntime(healthAfterDrain, 'runtime-revision-1')).toMatchObject({
      revisionState: 'retained',
      active: false,
      draining: false,
      inFlightCount: 0
    });
  });


  it('dispatches by activationIdentity when multiple same-revision activations are registered', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const activationA = 'skiff-runtime-activation-v1:opaque:activation-a';
    const activationB = 'skiff-runtime-activation-v1:opaque:activation-b';
    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-activation-a',
      revisionId: 'revision-shared',
      activationIdentity: activationA,
      targets: [target]
    });
    const runtimeB = await harness.registerRuntime({
      runtimeId: 'runtime-activation-b',
      revisionId: 'revision-shared',
      activationIdentity: activationB,
      targets: [target]
    });
    runtimeA.respondWithBinaryRuntimeId('runtime-activation-a');
    runtimeB.respondWithBinaryRuntimeId('runtime-activation-b');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-activation-b',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          activationIdentity: activationB
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-activation-b' });

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-activation-a',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          activationIdentity: activationA
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-activation-a' });
  });


  it('dispatches release requests by buildId while keeping legacy dev registrations separate', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const buildA =
      'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const buildB =
      'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-build-a',
      revisionId: 'revision-build-a',
      buildId: buildA,
      targets: [target]
    });
    const runtimeB = await harness.registerRuntime({
      runtimeId: 'runtime-build-b',
      revisionId: 'revision-build-b',
      buildId: buildB,
      targets: [target]
    });
    const legacyRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-no-build',
      revisionId: 'revision-no-build',
      targets: [target]
    });
    runtimeA.respondWithBinaryRuntimeId('runtime-build-a');
    runtimeB.respondWithBinaryRuntimeId('runtime-build-b');
    legacyRuntime.respondWithBinaryRuntimeId('runtime-no-build');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-build-b',
          target,
          buildId: buildB,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-build-b' });

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-legacy-dev',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-no-build' });
  });


  it('keeps single activation fallback but rejects ambiguous multi-activation dispatch', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-single-activation',
      revisionId: 'revision-shared',
      activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-a',
      targets: [target]
    });
    runtimeA.respondWithBinaryRuntimeId('runtime-single-activation');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-single-fallback',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-single-activation' });

    await harness.registerRuntime({
      runtimeId: 'runtime-second-activation',
      revisionId: 'revision-shared',
      activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-b',
      targets: [target]
    });

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-ambiguous-activation',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).rejects.toMatchObject({
      code: 'std.service.ProviderUnavailableError',
      message:
        'Multiple runtime activations match request; activationIdentity is required'
    });
  });


  it('rejects missing activationIdentity when default and activation-specific runtimes match', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const defaultRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-default-activation',
      revisionId: 'revision-shared',
      targets: [target]
    });

    const explicitRuntime = await harness.registerRuntime({
      runtimeId: 'runtime-explicit-activation',
      revisionId: 'revision-shared',
      activationIdentity: 'skiff-runtime-activation-v1:opaque:activation-a',
      targets: [target]
    });
    defaultRuntime.respondWithBinaryRuntimeId('runtime-default-activation');
    explicitRuntime.respondWithBinaryRuntimeId('runtime-explicit-activation');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-default-plus-activation',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).rejects.toMatchObject({
      code: 'std.service.ProviderUnavailableError',
      message:
        'Multiple runtime activations match request; activationIdentity is required'
    });
  });


  it('keeps gateway entry dispatch strict for runtimes with registered gateway identities', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = manifest.websocketEntry!.connect!.operationManifest.target;

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-wrong-gateway-identity',
      targets: [target],
      gatewayEntryIdentities: [manifest.websocketEntry!.receive.gatewayEntryIdentity]
    });
    runtime.respondWithBinaryRuntimeId('runtime-ws-wrong-gateway-identity');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-ws-wrong-gateway-identity',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          gatewayEntryIdentity: manifest.websocketEntry!.connect!.gatewayEntryIdentity
        }),
        2000
      )
    ).rejects.toMatchObject({
      statusCode: 503,
      code: 'std.service.ProviderUnavailableError'
    });
  });

  it('lazy-loads a different gateway entry on a connection with the same target build registered', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = manifest.websocketEntry!.connect!.operationManifest.target;
    const connectIdentity = manifest.websocketEntry!.connect!.gatewayEntryIdentity;

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-lazy-gateway-entry',
      targets: [target],
      gatewayEntryIdentities: [manifest.websocketEntry!.receive.gatewayEntryIdentity]
    });

    const requestFrame = runtime.waitForRequestFrame('request-lazy-gateway-entry');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-lazy-gateway-entry',
        target,
        serviceId: manifest.service.id,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        gatewayEntryIdentity: connectIdentity
      }),
      2000
    );

    const frame = await requestFrame;
    expect(frame.header).toMatchObject({
      requestId: 'request-lazy-gateway-entry',
      target,
      serviceId: manifest.service.id,
      gatewayEntryIdentity: connectIdentity
    });
    runtime.sendBinaryJsonResponse(frame.header.requestId, {
      runtimeId: 'runtime-lazy-gateway-entry',
      gatewayEntryIdentity: frame.header.gatewayEntryIdentity
    });

    await expect(dispatch).resolves.toEqual({
      runtimeId: 'runtime-lazy-gateway-entry',
      gatewayEntryIdentity: connectIdentity
    });
  });


  it('accepts binary runtime.register, returns binary runtime.registered, and dispatches binary requests', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtime = await openBinaryRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-binary-register',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });

    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-binary-register');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-binary-register',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );
    const request = await requestPromise;
    expect(request.header.target).toBe(target);

    sendRuntimeBinaryResponse(
      runtime,
      request.header.requestId,
      JSON.stringify({ runtimeId: 'runtime-binary-register' })
    );
    await expect(dispatch).resolves.toEqual({ runtimeId: 'runtime-binary-register' });
  });

  it('runs a binary-only runtime session through control, dispatch, timeout cancel, and connection.send', async () => {
    const manifest = loadWebSocketManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const target = manifest.websocketEntry!.receive.operationManifest.target;

    const runtime = await openBinaryRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-binary-session-flow',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target],
      gatewayEntryIdentities: [manifest.websocketEntry!.receive.gatewayEntryIdentity]
    });

    const controlMessage = onceWithTimeout(
      runtime,
      'message',
      'binary-only session router.control'
    );
    endpoint.broadcastControl({
      artifactRoots: ['/tmp/skiff-binary-session-flow'],
      generation: 'generation-binary-session-flow',
      fingerprint: 'sha256:binary-session-flow'
    });
    const [controlData, controlIsBinary] = await controlMessage;
    expect(controlIsBinary).toBe(true);
    const controlFrame = decodeRuntimeFrame(controlData as WebSocket.RawData);
    expect(controlFrame.header).toMatchObject({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'router.control',
      fingerprint: 'sha256:binary-session-flow'
    });
    expect(controlFrame.payloadBytes.byteLength).toBe(0);

    const firstRequestPromise = waitForRuntimeRequestFrame(
      runtime,
      'request-binary-session-ok'
    );
    const firstDispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-binary-session-ok',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        gatewayEntryIdentity: manifest.websocketEntry!.receive.gatewayEntryIdentity
      }),
      2000
    );
    const firstRequest = await firstRequestPromise;
    expect(firstRequest.header).toMatchObject({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.start',
      requestId: 'request-binary-session-ok',
      target,
      gatewayEntryIdentity: manifest.websocketEntry!.receive.gatewayEntryIdentity
    });
    sendRuntimeBinaryResponse(
      runtime,
      firstRequest.header.requestId,
      JSON.stringify({ ok: true })
    );
    await expect(firstDispatch).resolves.toEqual({ ok: true });

    const cancelPromise = waitForRuntimeCancel(
      runtime,
      'request-binary-session-timeout',
      'binary-only session timeout cancel'
    );
    const timeoutRequestPromise = waitForRuntimeRequestFrame(
      runtime,
      'request-binary-session-timeout'
    );
    const timeoutDispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-binary-session-timeout',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        gatewayEntryIdentity: manifest.websocketEntry!.receive.gatewayEntryIdentity
      }),
      10
    );
    await timeoutRequestPromise;
    await expect(timeoutDispatch).rejects.toMatchObject({
      code: 'TimeoutError'
    });
    await expect(cancelPromise).resolves.toEqual({
      isBinary: true,
      message: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'request.cancel',
        requestId: 'request-binary-session-timeout',
        reason: 'timeout'
      },
      payloadByteLength: 0
    });

    let unsubscribe: (() => void) | undefined;
    const forwardedPromise = new Promise<unknown>((resolve) => {
      unsubscribe = endpoint.onConnectionSend((message) => {
        unsubscribe?.();
        resolve(message);
      });
    });
    trackResource({ close: () => unsubscribe?.() });

    runtime.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'connection.send',
          serviceId: manifest.service.id,
          websocketEntryId: 'client',
          businessIdentity: 'binary-session-user'
        },
        new Uint8Array([7, 8, 9])
      )
    );

    await expect(forwardedPromise).resolves.toEqual({
      type: 'connection.send',
      serviceId: manifest.service.id,
      websocketEntryId: 'client',
      businessIdentity: 'binary-session-user',
      payloadKind: 'binary',
      payloadBytes: Buffer.from([7, 8, 9])
    });
  });

  it('forwards runtime-originated unary request.start frames and routes responses to the caller request id', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.handle';
    const calleeTarget = 'service.skiff~run~~hello.Callee.handle';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-service-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-service-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-service-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-service-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded service call request'
    );
    const callerResponsePromise = waitForRuntimeResponseEnd(
      caller,
      'caller-service-request-1',
      'forwarded service call response'
    );
    const requestHeader = serviceRequestStart({
      requestId: 'caller-service-request-1',
      callerTarget,
      target: calleeTarget,
      serviceId: manifest.service.id,
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });

    caller.send(encodeRuntimeFrame(requestHeader, Buffer.from('service payload')));

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header.requestId).toMatch(/^router-forward:/);
    expect(calleeRequest.header.requestId).not.toBe('caller-service-request-1');
    expect(calleeRequest.header).toMatchObject({
      caller: {
        kind: 'service',
        target: callerTarget
      },
      target: calleeTarget,
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
    expect(Buffer.from(calleeRequest.payloadBytes).toString('utf8')).toBe('service payload');

    sendRuntimeBinaryResponse(
      callee,
      calleeRequest.header.requestId,
      Buffer.from('service response')
    );

    await expect(callerResponsePromise).resolves.toEqual({
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'caller-service-request-1',
        payloadPresent: true
      },
      payload: 'service response'
    });
  });

  it('uses activation lookup to route runtime-originated calls without explicit activationIdentity', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerServiceId = 'example.com/chat';
    const calleeServiceId = 'example.com/remotellm';
    const callerTarget = 'service.example~com~~chat.ChatApi.send';
    const calleeTarget = 'service.example~com~~remotellm.RemoteLlmApi.chat';
    const activationA = 'skiff-runtime-activation-v1:opaque:remoteLlm-stale-config';
    const activationB = 'skiff-runtime-activation-v1:opaque:remoteLlm-current-config';
    const activationLookup = new ActivationLookup();
    activationLookup.set({
      serviceId: calleeServiceId,
      target: calleeTarget,
      buildId: DEFAULT_TEST_BUILD_ID,
      activationIdentity: activationB
    });
    registry.setActivationLookup(activationLookup);

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-activation-lookup-caller',
      serviceId: callerServiceId,
      revisionId: 'revision-activation-lookup-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-remoteLlm-stale-config',
      serviceId: calleeServiceId,
      revisionId: 'revision-remoteLlm-shared',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      activationIdentity: activationA,
      targets: [calleeTarget]
    });
    const selectedCallee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-remoteLlm-current-config',
      serviceId: calleeServiceId,
      revisionId: 'revision-remoteLlm-shared',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      activationIdentity: activationB,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      selectedCallee,
      'activation lookup selected callee request'
    );
    const callerResponsePromise = waitForRuntimeResponseEnd(
      caller,
      'caller-service-request-activation-lookup',
      'activation lookup selected callee response'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-service-request-activation-lookup',
          callerTarget,
          target: calleeTarget,
          serviceId: calleeServiceId,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header).toMatchObject({
      caller: {
        kind: 'service',
        target: callerTarget
      },
      target: calleeTarget,
      serviceId: calleeServiceId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
    sendRuntimeBinaryResponse(
      selectedCallee,
      calleeRequest.header.requestId,
      Buffer.from('selected activation response')
    );

    await expect(callerResponsePromise).resolves.toEqual({
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'caller-service-request-activation-lookup',
        payloadPresent: true
      },
      payload: 'selected activation response'
    });
  });

  it('refreshes runtime activation lookup after artifact reload', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const callerServiceId = 'example.com/chat';
    const calleeServiceId = 'example.com/remotellm';
    const callerTarget = 'service.example~com~~chat.ChatApi.send';
    const calleeTarget = 'service.example~com~~remotellm.RemoteLlmApi.chat';
    const activationA = 'skiff-runtime-activation-v1:opaque:reload-stale-config';
    const activationB = 'skiff-runtime-activation-v1:opaque:reload-current-config';
    const activationLookup = new ActivationLookup();
    activationLookup.set({
      serviceId: calleeServiceId,
      target: calleeTarget,
      buildId: DEFAULT_TEST_BUILD_ID,
      activationIdentity: activationB
    });
    const snapshotV1: RouterActiveSnapshot = {
      activationByServiceOperation: new ActivationLookup(),
      manifest
    };
    const snapshotV2: RouterActiveSnapshot = {
      activationByServiceOperation: activationLookup,
      manifest
    };
    const snapshotStore = new RouterActiveSnapshotStore(snapshotV1);
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore,
      reloadArtifacts: async () => snapshotV2
    });
    const registryListen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = registryListen.url
      .replace('ws://', 'http://')
      .replace('/runtime', '');

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-reload-activation-caller',
      serviceId: callerServiceId,
      revisionId: 'revision-reload-activation-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-reload-stale-config',
      serviceId: calleeServiceId,
      revisionId: 'revision-reload-remoteLlm-shared',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      activationIdentity: activationA,
      targets: [calleeTarget]
    });
    const selectedCallee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-reload-current-config',
      serviceId: calleeServiceId,
      revisionId: 'revision-reload-remoteLlm-shared',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      activationIdentity: activationB,
      targets: [calleeTarget]
    });

    const reloadResponse = await requestHttp({
      url: `${controlUrl}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    expect(reloadResponse.status).toBe(200);

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      selectedCallee,
      'reloaded activation lookup selected callee request'
    );
    const callerResponsePromise = waitForRuntimeResponseEnd(
      caller,
      'caller-service-request-reloaded-activation-lookup',
      'reloaded activation lookup selected callee response'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-service-request-reloaded-activation-lookup',
          callerTarget,
          target: calleeTarget,
          serviceId: calleeServiceId,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header).toMatchObject({
      target: calleeTarget,
      serviceId: calleeServiceId,
      buildId: DEFAULT_TEST_BUILD_ID
    });
    sendRuntimeBinaryResponse(
      selectedCallee,
      calleeRequest.header.requestId,
      Buffer.from('reloaded selected activation response')
    );

    await expect(callerResponsePromise).resolves.toEqual({
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'caller-service-request-reloaded-activation-lookup',
        payloadPresent: true
      },
      payload: 'reloaded selected activation response'
    });
  });

  it('rewrites stale version-addressed runtime-originated lazy loads to the current build id', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerServiceId = 'skiff.run/api';
    const calleeServiceId = 'skiff.run/account';
    const callerTarget = 'service.skiff~run~~api.ChatApi.send';
    const calleeTarget = 'service.skiff~run~~account.AccountApi.lookup';
    const currentBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000020cc';
    const staleBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000020dd';
    registry.setServiceVersionIndex(
      new Map([[calleeServiceId, new Map([['0.1.0', { buildId: currentBuild }]])]])
    );

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-cross-service-caller',
      serviceId: callerServiceId,
      revisionId: 'revision-cross-service-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      caller,
      'version-addressed lazy callee request'
    );
    const callerResponsePromise = waitForRuntimeResponseEnd(
      caller,
      'caller-service-request-current-build',
      'version-addressed lazy callee response'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-service-request-current-build',
          callerTarget,
          target: calleeTarget,
          serviceId: calleeServiceId,
          version: '0.1.0',
          buildId: staleBuild,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header.requestId).toMatch(/^router-forward:/);
    expect(calleeRequest.header).toMatchObject({
      caller: {
        kind: 'service',
        target: callerTarget
      },
      target: calleeTarget,
      serviceId: calleeServiceId,
      version: '0.1.0',
      buildId: currentBuild,
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
    expect(Buffer.from(calleeRequest.payloadBytes).toString('utf8')).toBe('service payload');

    sendRuntimeBinaryResponse(
      caller,
      calleeRequest.header.requestId,
      Buffer.from('service response')
    );

    await expect(callerResponsePromise).resolves.toEqual({
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'caller-service-request-current-build',
        payloadPresent: true
      },
      payload: 'service response'
    });
  });

  it('forwards runtime-originated serverStream response frames to the caller request id', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.stream';
    const calleeTarget = 'service.skiff~run~~hello.Callee.stream';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-stream-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-stream-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-stream-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-stream-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded stream service call request'
    );
    const callerFramesPromise = collectRuntimeResponseFrames(
      caller,
      'caller-stream-request-1',
      3,
      'forwarded stream service call response frames'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-stream-request-1',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          mode: 'serverStream'
        }),
        Buffer.from('stream payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header.requestId).toMatch(/^router-forward:/);
    expect(calleeRequest.header).toMatchObject({
      mode: 'serverStream',
      caller: {
        kind: 'service',
        target: callerTarget
      },
      target: calleeTarget
    });

    callee.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: calleeRequest.header.requestId,
        httpResponse: {
          status: 200,
          headers: []
        }
      })
    );
    callee.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId: calleeRequest.header.requestId,
          seq: 0
        },
        Buffer.from('chunk-1')
      )
    );
    callee.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: calleeRequest.header.requestId,
        payloadPresent: false
      })
    );

    await expect(callerFramesPromise).resolves.toEqual([
      {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.start',
          requestId: 'caller-stream-request-1',
          httpResponse: {
            status: 200,
            headers: []
          }
        },
        payload: ''
      },
      {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId: 'caller-stream-request-1',
          seq: 0
        },
        payload: 'chunk-1'
      },
      {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: 'caller-stream-request-1',
          payloadPresent: false
        },
        payload: ''
      }
    ]);
  });

  it('does not timeout a runtime-originated serverStream after response.start', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.streamSlow';
    const calleeTarget = 'service.skiff~run~~hello.Callee.streamSlow';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-slow-stream-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-slow-stream-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-slow-stream-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-slow-stream-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded slow stream service call request'
    );
    const callerFramesPromise = collectRuntimeResponseFrames(
      caller,
      'caller-slow-stream-request-1',
      3,
      'forwarded slow stream service call response frames'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-slow-stream-request-1',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          mode: 'serverStream',
          timeoutMs: 25
        }),
        Buffer.from('stream payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    callee.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: calleeRequest.header.requestId,
        httpResponse: {
          status: 200,
          headers: []
        }
      })
    );
    await delay(50);
    callee.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId: calleeRequest.header.requestId,
          seq: 0
        },
        Buffer.from('chunk-after-timeout-window')
      )
    );
    callee.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: calleeRequest.header.requestId,
        payloadPresent: false
      })
    );

    await expect(callerFramesPromise).resolves.toEqual([
      {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.start',
          requestId: 'caller-slow-stream-request-1',
          httpResponse: {
            status: 200,
            headers: []
          }
        },
        payload: ''
      },
      {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId: 'caller-slow-stream-request-1',
          seq: 0
        },
        payload: 'chunk-after-timeout-window'
      },
      {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.end',
          requestId: 'caller-slow-stream-request-1',
          payloadPresent: false
        },
        payload: ''
      }
    ]);
  });

  it('forwards runtime-originated response.error frames to the caller request id', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.error';
    const calleeTarget = 'service.skiff~run~~hello.Callee.error';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-error-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-error-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-error-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-error-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded error service call request'
    );
    const callerErrorPromise = waitForRuntimeResponseError(
      caller,
      'caller-error-1',
      'forwarded service call error'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-error-1',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header.requestId).toMatch(/^router-forward:/);
    callee.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: calleeRequest.header.requestId,
        error: {
          code: 'CalleeFailed',
          message: 'callee failed'
        }
      })
    );

    await expect(callerErrorPromise).resolves.toMatchObject({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: 'caller-error-1',
      error: {
        code: 'CalleeFailed',
        message: 'callee failed'
      }
    });
    expect(findRuntime(registry.snapshot(), 'runtime-error-callee')).toMatchObject({
      revisionState: 'active',
      inFlightCount: 0
    });
  });

  it('fails closed when a runtime-originated unary forward receives response.start', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.unaryStart';
    const calleeTarget = 'service.skiff~run~~hello.Callee.unaryStart';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-unary-start-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-unary-start-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-unary-start-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-unary-start-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded unary start request'
    );
    const callerErrorPromise = waitForRuntimeResponseError(
      caller,
      'caller-unary-start-1',
      'unary start protocol error'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-unary-start-1',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    callee.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: calleeRequest.header.requestId,
        httpResponse: {
          status: 200,
          headers: []
        }
      })
    );

    await expect(callerErrorPromise).resolves.toMatchObject({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: 'caller-unary-start-1',
      error: {
        code: 'UnexpectedStart',
        message: 'response.start is only valid for serverStream dispatch'
      }
    });
    sendRuntimeBinaryResponse(
      callee,
      calleeRequest.header.requestId,
      Buffer.from('late response')
    );
    await delay(10);
    expect(findRuntime(registry.snapshot(), 'runtime-unary-start-callee')).toMatchObject({
      revisionState: 'active',
      inFlightCount: 0
    });
  });

  it('fails closed when a runtime-originated unary forward receives response.chunk', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.unaryChunk';
    const calleeTarget = 'service.skiff~run~~hello.Callee.unaryChunk';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-unary-chunk-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-unary-chunk-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-unary-chunk-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-unary-chunk-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded unary chunk request'
    );
    const callerErrorPromise = waitForRuntimeResponseError(
      caller,
      'caller-unary-chunk-1',
      'unary chunk protocol error'
    );

    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-unary-chunk-1',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    callee.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId: calleeRequest.header.requestId,
          seq: 0
        },
        Buffer.from('chunk')
      )
    );

    await expect(callerErrorPromise).resolves.toMatchObject({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'response.error',
      requestId: 'caller-unary-chunk-1',
      error: {
        code: 'UnexpectedChunk',
        message: 'response.chunk is only valid for serverStream dispatch'
      }
    });
    sendRuntimeBinaryResponse(
      callee,
      calleeRequest.header.requestId,
      Buffer.from('late response')
    );
    await delay(10);
    expect(findRuntime(registry.snapshot(), 'runtime-unary-chunk-callee')).toMatchObject({
      revisionState: 'active',
      inFlightCount: 0
    });
  });

  it('isolates runtime-originated request ids from router-owned pending ids on the same socket', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.handle';
    const calleeTarget = 'service.skiff~run~~hello.Callee.handle';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-collision-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-collision-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-collision-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-collision-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const inboundRequestPromise = waitForRuntimeRequestFrame(caller, 'shared-id');
    const inboundDispatch = dispatcher.dispatchBinary(
      {
        header: {
          ...serviceRequestStart({
            requestId: 'shared-id',
            callerTarget: 'gateway.skiff~run~~hello.http.test',
            callerKind: 'gateway',
            target: callerTarget,
            serviceId: manifest.service.id,
            serviceProtocolIdentity: manifest.service.protocolIdentity
          })
        },
        payloadBytes: Buffer.from('gateway payload')
      },
      2000
    );
    await inboundRequestPromise;

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded same-id service request'
    );
    const callerResponsePromise = waitForRuntimeResponseEnd(
      caller,
      'shared-id',
      'forwarded same-id service response'
    );
    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'shared-id',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    expect(calleeRequest.header.requestId).toMatch(/^router-forward:/);
    expect(calleeRequest.header.requestId).not.toBe('shared-id');
    sendRuntimeBinaryResponse(
      callee,
      calleeRequest.header.requestId,
      Buffer.from('service response')
    );
    await expect(callerResponsePromise).resolves.toEqual({
      header: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: 'shared-id',
        payloadPresent: true
      },
      payload: 'service response'
    });

    sendRuntimeBinaryResponse(caller, 'shared-id', Buffer.from('gateway response'));
    await expect(inboundDispatch).resolves.toMatchObject({
      header: {
        requestId: 'shared-id',
        type: 'response.end'
      },
      payloadBytes: Buffer.from('gateway response')
    });
  });

  it('forwards runtime-originated cancels to the selected runtime with the router request id', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const callerTarget = 'service.skiff~run~~hello.Caller.handle';
    const calleeTarget = 'service.skiff~run~~hello.Callee.handle';

    const caller = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-cancel-caller',
      serviceId: manifest.service.id,
      revisionId: 'revision-cancel-caller',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [callerTarget]
    });
    const callee = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-cancel-callee',
      serviceId: manifest.service.id,
      revisionId: 'revision-cancel-callee',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [calleeTarget]
    });

    const calleeRequestPromise = waitForAnyRuntimeRequestFrame(
      callee,
      'forwarded cancel request'
    );
    caller.send(
      encodeRuntimeFrame(
        serviceRequestStart({
          requestId: 'caller-cancel-1',
          callerTarget,
          target: calleeTarget,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        Buffer.from('service payload')
      )
    );

    const calleeRequest = await calleeRequestPromise;
    const cancelPromise = waitForRuntimeCancel(
      callee,
      calleeRequest.header.requestId,
      'forwarded service cancel'
    );
    caller.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'request.cancel',
        requestId: 'caller-cancel-1',
        reason: 'caller_cancel'
      })
    );

    await expect(cancelPromise).resolves.toMatchObject({
      isBinary: true,
      message: {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'request.cancel',
        requestId: calleeRequest.header.requestId,
        reason: 'caller_cancel'
      }
    });
  });


  it('broadcasts router.control as binary frames', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtimeA = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-control-a',
      serviceId: manifest.service.id,
      revisionId: 'revision-control-a',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    const runtimeB = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-control-b',
      serviceId: manifest.service.id,
      revisionId: 'revision-control-b',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });

    const controlA = onceWithTimeout(
      runtimeA,
      'message',
      'runtime A router control'
    );
    const controlB = onceWithTimeout(
      runtimeB,
      'message',
      'runtime B router control'
    );
    const control = {
      artifactRoots: ['/tmp/skiff-router-control', '/tmp/skiff-router-control-overlay'],
      generation: 'generation-control-broadcast',
      fingerprint: 'sha256:control-broadcast'
    };
    endpoint.broadcastControl(control);

    for (const [data, isBinary] of [await controlA, await controlB]) {
      expect(isBinary).toBe(true);
      const frame = decodeRuntimeFrame(data as WebSocket.RawData);
      expect(frame.header).toEqual({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'router.control',
        ...control
      });
      expect(frame.payloadBytes.byteLength).toBe(0);
    }
  });


  it('closes binary runtime.register frames with non-empty payloads', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const ws = new WebSocket(registryListen.url);
    trackResource({ close: () => ws.close() });
    await onceWithTimeout(ws, 'open', 'binary register payload socket open');

    ws.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'runtime.register',
          runtimeId: 'runtime-binary-register-payload',
          serviceId: manifest.service.id,
          revisionId: manifest.service.revisionId,
          buildId: DEFAULT_TEST_BUILD_ID,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          targets: ['service.skiff~run~~hello.HelloApi.hello']
        },
        Buffer.from('unexpected payload')
      )
    );

    const [code, reason] = await onceWithTimeout(
      ws,
      'close',
      'binary register payload close'
    );
    expect(code).toBe(1011);
    expect(Buffer.from(reason as Buffer).toString('utf8')).toBe(
      'runtime.register binary frame payload must be empty'
    );
  });

  it('closes runtime.register frames with raw service targets', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const ws = new WebSocket(registryListen.url);
    trackResource({ close: () => ws.close() });
    await onceWithTimeout(ws, 'open', 'raw register target socket open');

    ws.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'runtime.register',
        runtimeId: 'runtime-raw-register-target',
        serviceId: manifest.service.id,
        revisionId: manifest.service.revisionId,
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        targets: ['service.skiff.run/hello.HelloApi.hello']
      })
    );

    const [code, reason] = await onceWithTimeout(
      ws,
      'close',
      'raw register target close'
    );
    expect(code).toBe(1011);
    expect(Buffer.from(reason as Buffer).toString('utf8')).toBe(
      'invalid runtime.register envelope: targets items must use service.skiff~run~~hello.<target suffix>'
    );
  });


  it('ignores response and cancel envelopes from a runtime that does not own the pending request', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtimeA = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-owner-a',
      serviceId: manifest.service.id,
      revisionId: 'revision-owner-a',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    const ownerRequestPromise = waitForRuntimeRequestFrame(runtimeA, 'request-owned-by-a');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-owned-by-a',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );
    const ownerRequest = await ownerRequestPromise;

    const runtimeB = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-owner-b',
      serviceId: manifest.service.id,
      revisionId: 'revision-owner-b',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    sendRuntimeBinaryResponse(
      runtimeB,
      ownerRequest.header.requestId,
      JSON.stringify({ runtimeId: 'runtime-owner-b' })
    );
    runtimeB.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.error',
        requestId: ownerRequest.header.requestId,
        error: {
          code: 'SpoofedError',
          message: 'runtime B tried to reject runtime A request'
        }
      })
    );
    runtimeB.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'request.cancel',
        requestId: ownerRequest.header.requestId,
        reason: 'drain'
      })
    );

    sendRuntimeBinaryResponse(
      runtimeA,
      ownerRequest.header.requestId,
      JSON.stringify({ runtimeId: 'runtime-owner-a' })
    );
    await expect(dispatch).resolves.toEqual({ runtimeId: 'runtime-owner-a' });
  });


  it('sends timeout request.cancel as a binary frame', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-cancel-timeout',
      serviceId: manifest.service.id,
      revisionId: 'revision-cancel-timeout',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-cancel-timeout');
    const cancelPromise = waitForRuntimeCancel(
      runtime,
      'request-cancel-timeout',
      'runtime timeout cancel'
    );
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-cancel-timeout',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      10
    );
    await requestPromise;
    await expect(dispatch).rejects.toMatchObject({
      code: 'TimeoutError'
    });
    const cancel = await cancelPromise;
    expect(cancel.isBinary).toBe(true);
    expect(cancel.message).toEqual({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'request.cancel',
      requestId: 'request-cancel-timeout',
      reason: 'timeout'
    });
    expect(cancel.payloadByteLength).toBe(0);
  });

  it('keeps binary serverStream pending after response.start until response.end terminal', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const target = 'service.skiff~run~~hello.HelloApi.stream';

    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-stream-pending-start',
      serviceId: manifest.service.id,
      revisionId: 'revision-stream-pending-start',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });

    let closeCount = 0;
    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-stream-pending-start');
    const dispatch = dispatcher.dispatchBinaryStream(
      {
        header: serviceRequestStart({
          requestId: 'request-stream-pending-start',
          callerTarget: 'gateway.test.stream',
          callerKind: 'gateway',
          target,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          mode: 'serverStream'
        }),
        payloadBytes: Buffer.from('stream payload')
      },
      25,
      {
        onStart: () => {},
        onChunk: () => {},
        onEnd: (_response, requestTerminal) => {
          requestTerminal({ source: 'runtime_response_end', kind: 'completed' });
        },
        closeFromPendingTerminal: () => {
          closeCount += 1;
        }
      }
    );
    const request = await requestPromise;
    runtime.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: request.header.requestId,
        httpResponse: {
          status: 200,
          headers: []
        }
      })
    );

    await delay(50);
    expect(dispatcher.pendingLifecycleCounters()).toMatchObject({
      pendingStream: 1
    });

    runtime.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.end',
        requestId: request.header.requestId,
        payloadPresent: false
      })
    );
    await expect(dispatch).resolves.toMatchObject({
      header: {
        type: 'response.end',
        requestId: 'request-stream-pending-start'
      }
    });
    expect(dispatcher.pendingLifecycleCounters()).toMatchObject({
      pendingStream: 0
    });
    expect(closeCount).toBe(1);
  });

  it('cancels runtime work and closes stream writer on stream callback errors', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const target = 'service.skiff~run~~hello.HelloApi.streamCallbackError';

    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-stream-callback-error',
      serviceId: manifest.service.id,
      revisionId: 'revision-stream-callback-error',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });

    let closedTerminalSource: string | undefined;
    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-stream-callback-error');
    const dispatch = dispatcher.dispatchBinaryStream(
      {
        header: serviceRequestStart({
          requestId: 'request-stream-callback-error',
          callerTarget: 'gateway.test.stream',
          callerKind: 'gateway',
          target,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          mode: 'serverStream'
        }),
        payloadBytes: Buffer.from('stream payload')
      },
      2000,
      {
        onStart: () => {
          throw new Error('stream writer exploded');
        },
        onChunk: () => {},
        onEnd: () => {},
        closeFromPendingTerminal: (terminal) => {
          closedTerminalSource = terminal.source;
        }
      }
    );
    const request = await requestPromise;
    const cancelPromise = waitForRuntimeCancel(
      runtime,
      request.header.requestId,
      'stream callback error cancel'
    );
    runtime.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.start',
        requestId: request.header.requestId,
        httpResponse: {
          status: 200,
          headers: []
        }
      })
    );

    await expect(dispatch).rejects.toThrow('stream writer exploded');
    await expect(cancelPromise).resolves.toMatchObject({
      message: {
        type: 'request.cancel',
        requestId: request.header.requestId,
        reason: 'protocol_error'
      }
    });
    expect(closedTerminalSource).toBe('callback_error');
    expect(dispatcher.pendingLifecycleCounters()).toMatchObject({
      pendingStream: 0
    });
  });

  it('classifies runtime stream protocol violations as protocol_error terminal', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const target = 'service.skiff~run~~hello.HelloApi.streamProtocolError';

    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-stream-protocol-error',
      serviceId: manifest.service.id,
      revisionId: 'revision-stream-protocol-error',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });

    let closedTerminalSource: string | undefined;
    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-stream-protocol-error');
    const dispatch = dispatcher.dispatchBinaryStream(
      {
        header: serviceRequestStart({
          requestId: 'request-stream-protocol-error',
          callerTarget: 'gateway.test.stream',
          callerKind: 'gateway',
          target,
          serviceId: manifest.service.id,
          serviceProtocolIdentity: manifest.service.protocolIdentity,
          mode: 'serverStream'
        }),
        payloadBytes: Buffer.from('stream payload')
      },
      2000,
      {
        onStart: () => {},
        onChunk: () => {},
        onEnd: () => {},
        closeFromPendingTerminal: (terminal) => {
          closedTerminalSource = terminal.source;
        }
      }
    );
    const request = await requestPromise;
    const cancelPromise = waitForRuntimeCancel(
      runtime,
      request.header.requestId,
      'stream protocol error cancel'
    );
    runtime.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'response.chunk',
          requestId: request.header.requestId,
          seq: 0
        },
        Buffer.from('chunk-before-start')
      )
    );

    await expect(dispatch).rejects.toMatchObject({
      code: 'StreamProtocolError',
      message: 'response.chunk received before response.start'
    });
    await expect(cancelPromise).resolves.toMatchObject({
      message: {
        type: 'request.cancel',
        requestId: request.header.requestId,
        reason: 'protocol_error'
      }
    });
    expect(closedTerminalSource).toBe('protocol_error');
    expect(dispatcher.pendingLifecycleCounters()).toMatchObject({
      pendingStream: 0
    });
  });


  it('rejects response.chunk during unary dispatch', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-unary-chunk',
      serviceId: manifest.service.id,
      revisionId: 'revision-unary-chunk',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-unary-chunk');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-unary-chunk',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    );
    const request = await requestPromise;

    runtime.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'response.chunk',
        requestId: request.header.requestId,
        seq: 0
      })
    );

    await expect(dispatch).rejects.toMatchObject({
      statusCode: 502,
      code: 'UnexpectedChunk',
      details: {
        runtimeError: {
          code: 'UnexpectedChunk',
          message: 'response.chunk is only valid for serverStream dispatch'
        }
      }
    });
    expect(findRuntime(registry.snapshot(), 'runtime-unary-chunk')).toMatchObject({
      revisionState: 'active',
      inFlightCount: 0
    });
  });


  it('closes runtime sockets with explicit validation errors for invalid envelopes', async () => {
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });

    const ws = new WebSocket(registryListen.url);
    trackResource({ close: () => ws.close() });
    await onceWithTimeout(ws, 'open', 'invalid runtime socket open');

    ws.send(JSON.stringify({ type: 'response.end' }));

    const [code, reason] = (await onceWithTimeout(
      ws,
      'close',
      'invalid runtime envelope close'
    )) as [number, Buffer];
    expect(code).toBe(1011);
    expect(reason.toString()).toBe(
      'text JSON runtime protocol messages are not supported; use typed binary runtime frames'
    );
  });


  it('closes runtime sockets without accepting text JSON response errors for binary dispatch', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-text-response-error',
      serviceId: manifest.service.id,
      revisionId: 'revision-text-response-error',
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [target]
    });
    const requestPromise = waitForRuntimeRequestFrame(runtime, 'request-text-response-error');
    const dispatch = dispatchBinaryJson(dispatcher,
      createRequestStart({
        requestId: 'request-text-response-error',
        target,
        serviceProtocolIdentity: manifest.service.protocolIdentity
      }),
      2000
    ).catch((error: unknown) => error);
    const request = await requestPromise;

    runtime.send(
      JSON.stringify({
        type: 'response.error',
        requestId: request.header.requestId,
        error: {
          code: 'LegacyTextError',
          message: 'legacy text response.error should not reject the request directly'
        }
      })
    );

    const [code, reason] = (await onceWithTimeout(
      runtime,
      'close',
      'text response.error runtime close'
    )) as [number, Buffer];
    expect(code).toBe(1011);
    expect(reason.toString()).toBe(
      'text JSON runtime protocol messages are not supported; use typed binary runtime frames'
    );
    await expect(dispatch).resolves.toMatchObject({
      code: 'std.service.ProviderUnavailableError',
      message: 'Runtime disconnected before responding'
    });
  });


  it('removes disconnected runtimes from live health results', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.http({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-health-disconnect',
      revisionId: 'revision-health-disconnect',
      targets: ['service.skiff~run~~hello.HelloApi.hello']
    });

    const controlUrl = harness.registryListen!.url.replace('ws://', 'http://').replace('/runtime', '');
    expect(hasRuntime(await readHealth(controlUrl), 'runtime-health-disconnect')).toBe(true);
    await closeSocket(runtime.ws, 'health runtime close');
    await waitForRuntimeAbsent(controlUrl, 'runtime-health-disconnect');
  });


  it('round-robins across live runtime instances in the same active revision', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';

    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-same-revision-a',
      revisionId: 'revision-shared',
      targets: [target]
    });
    runtimeA.respondWithBinaryRuntimeId('runtime-same-revision-a');

    const runtimeB = await harness.registerRuntime({
      runtimeId: 'runtime-same-revision-b',
      revisionId: 'revision-shared',
      targets: [target]
    });
    runtimeB.respondWithBinaryRuntimeId('runtime-same-revision-b');

    const responses = [];
    for (const requestId of ['rr-1', 'rr-2', 'rr-3', 'rr-4']) {
      responses.push(
        await dispatchBinaryJson(dispatcher,
          createRequestStart({
            requestId,
            target,
            serviceProtocolIdentity: manifest.service.protocolIdentity
          }),
          2000
        )
      );
    }

    expect(responses).toEqual([
      { runtimeId: 'runtime-same-revision-a' },
      { runtimeId: 'runtime-same-revision-b' },
      { runtimeId: 'runtime-same-revision-a' },
      { runtimeId: 'runtime-same-revision-b' }
    ]);

    await closeSocket(runtimeA.ws, 'same revision runtime A close');
    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'rr-after-close',
          target,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-same-revision-b' });
  });


  it('replaces only overlapping targets when a new revision registers a partial target set', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const helloTarget = 'service.skiff~run~~hello.HelloApi.hello';
    const echoTarget = 'service.skiff~run~~hello.HelloApi.echo';

    const runtimeV1 = await harness.registerRuntime({
      runtimeId: 'runtime-partial-v1',
      revisionId: 'revision-partial-v1',
      targets: [helloTarget, echoTarget]
    });
    runtimeV1.respondWithBinaryRuntimeId('runtime-partial-v1');

    const runtimeV2 = await harness.registerRuntime({
      runtimeId: 'runtime-partial-v2',
      revisionId: 'revision-partial-v2',
      targets: [helloTarget]
    });
    runtimeV2.respondWithBinaryRuntimeId('runtime-partial-v2');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'partial-hello',
          target: helloTarget,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-partial-v2' });
    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'partial-echo',
          target: echoTarget,
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-partial-v1' });

    expect(findRuntime(registry.snapshot(), 'runtime-partial-v1')).toMatchObject({
      revisionState: 'active',
      active: true
    });
  });


  // Different protocol identities mean different builds (protocol identity is
  // part of the build's identity closure), so they register under distinct
  // buildIds. Addressing is by build (selected here directly via the frozen
  // buildId; version addressing resolves to the same buildId). The boundary
  // check passes because each build's protocol identity matches the caller's
  // frozen expectation for that build.
  it('keeps distinct builds with different protocol identities routable for the same target', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const protocolA = manifest.service.protocolIdentity;
    const protocolB =
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004';
    const buildA =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000000aa';
    const buildB =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000000bb';

    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-protocol-a',
      revisionId: 'revision-a',
      buildId: buildA,
      serviceProtocolIdentity: protocolA,
      targets: [target]
    });
    runtimeA.respondWithBinaryRuntimeId('runtime-protocol-a');

    const runtimeB = await harness.registerRuntime({
      runtimeId: 'runtime-protocol-b',
      revisionId: 'revision-b',
      buildId: buildB,
      serviceProtocolIdentity: protocolB,
      targets: [target]
    });
    runtimeB.respondWithBinaryRuntimeId('runtime-protocol-b');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-protocol-a',
          target,
          buildId: buildA,
          serviceProtocolIdentity: protocolA
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-protocol-a' });
    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-protocol-b',
          target,
          buildId: buildB,
          serviceProtocolIdentity: protocolB
        }),
        2000
      )
    ).resolves.toEqual({ runtimeId: 'runtime-protocol-b' });

    expect(findRuntime(registry.snapshot(), 'runtime-protocol-a')).toMatchObject({
      revisionState: 'active',
      active: true
    });
    expect(findRuntime(registry.snapshot(), 'runtime-protocol-b')).toMatchObject({
      revisionState: 'active',
      active: true
    });
  });

  // Version is the addressing key: the router resolves (serviceId, version) to
  // the current build via setServiceVersionIndex and routes there, regardless
  // of the buildId the caller froze at publish time.
  it('addresses cross-service calls by service id + version', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const serviceId = manifest.service.id;
    const protocol = manifest.service.protocolIdentity;
    const currentBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000000cc';
    const staleBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000000dd';

    registry.setServiceVersionIndex(
      new Map([[serviceId, new Map([['0.1.0', { buildId: currentBuild }]])]])
    );

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-current',
      revisionId: 'revision-current',
      serviceId,
      version: '0.1.0',
      buildId: currentBuild,
      serviceProtocolIdentity: protocol,
      targets: [target]
    });

    // Caller froze a stale buildId at publish time; version addressing must
    // still resolve to the current build.
    const requestFrame = runtime.waitForRequestFrame('request-by-version');
    const dispatch = dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-by-version',
          target,
          serviceId,
          version: '0.1.0',
          buildId: staleBuild,
          serviceProtocolIdentity: protocol
        }),
        2000
    );

    const frame = await requestFrame;
    expect(frame.header).toMatchObject({
      requestId: 'request-by-version',
      serviceId,
      version: '0.1.0',
      buildId: currentBuild
    });
    runtime.sendBinaryJsonResponse(frame.header.requestId, {
      runtimeId: 'runtime-current'
    });
    await expect(dispatch).resolves.toEqual({ runtimeId: 'runtime-current' });
  });

  // Boundary check: when the resolved current build's protocol identity no
  // longer satisfies the caller's frozen expectation, the call must fail with a
  // specific error rather than route to an incompatible build.
  it('rejects cross-service calls when the current build breaks the caller boundary', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const serviceId = manifest.service.id;
    const callerExpectation = manifest.service.protocolIdentity;
    const incompatibleProtocol =
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000009';
    const currentBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000000ee';

    registry.setServiceVersionIndex(
      new Map([[serviceId, new Map([['0.1.0', { buildId: currentBuild }]])]])
    );

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-incompatible',
      revisionId: 'revision-incompatible',
      serviceId,
      version: '0.1.0',
      buildId: currentBuild,
      serviceProtocolIdentity: incompatibleProtocol,
      targets: [target]
    });
    runtime.respondWithBinaryRuntimeId('runtime-incompatible');

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-boundary-break',
          target,
          serviceId,
          version: '0.1.0',
          serviceProtocolIdentity: callerExpectation
        }),
        2000
      )
    ).rejects.toMatchObject({
      code: 'std.service.ProtocolError',
      statusCode: 502
    });
  });

  // A version with no published pointer record for an indexed service is
  // unavailable, not a silent route to an unindexed build.
  it('fails cross-service calls for an unpublished version', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const harness = await RouterHarness.create({ manifest });
    const { dispatcher, registry } = harness;
    const target = 'service.skiff~run~~hello.HelloApi.hello';
    const serviceId = manifest.service.id;
    const publishedBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000000ff';

    // The service is indexed (it has a published version), but not 9.9.9.
    registry.setServiceVersionIndex(
      new Map([[serviceId, new Map([['0.1.0', { buildId: publishedBuild }]])]])
    );

    await expect(
      dispatchBinaryJson(dispatcher,
        createRequestStart({
          requestId: 'request-unpublished-version',
          target,
          serviceId,
          version: '9.9.9',
          serviceProtocolIdentity: manifest.service.protocolIdentity
        }),
        2000
      )
    ).rejects.toMatchObject({
      code: 'std.service.ProviderUnavailableError'
    });
  });


  it('accepts expanded request.cancel reasons without closing the runtime socket', async () => {
    const manifest = await loadManifestFile('fixtures/hello/manifest.json');
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });

    const ws = new WebSocket(registryListen.url);
    trackResource({ close: () => ws.close() });
    await onceWithTimeout(ws, 'open', 'runtime socket open');

    ws.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'request.cancel',
        requestId: 'missing-request',
        reason: 'drain'
      })
    );
    const registered = onceWithTimeout(ws, 'message', 'runtime cancel compatible registration');
    ws.send(
      encodeRuntimeFrame({
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'runtime.register',
        runtimeId: 'runtime-cancel-compatible',
        serviceId: manifest.service.id,
        revisionId: manifest.service.revisionId,
        buildId: DEFAULT_TEST_BUILD_ID,
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        targets: manifest.operations.map((operation) => operation.target)
      })
    );

    const [registeredData, registeredIsBinary] = await registered;
    expect(registeredIsBinary).toBe(true);
    expect(decodeRuntimeFrame(registeredData as WebSocket.RawData).header).toEqual({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'runtime.registered',
      runtimeId: 'runtime-cancel-compatible'
    });
    expect(ws.readyState).toBe(WebSocket.OPEN);
  });


  it('rejects text JSON runtime protocol messages before forwarding', async () => {
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    let forwarded = false;
    const unsubscribe = endpoint.onConnectionSend(() => {
      forwarded = true;
    });
    trackResource({ close: () => unsubscribe() });

    const ws = new WebSocket(registryListen.url);
    trackResource({ close: () => ws.close() });
    await onceWithTimeout(ws, 'open', 'runtime socket open');

    ws.send(
      JSON.stringify({
        type: 'connection.send',
        serviceId: 'example.com/websocket_fixture',
        connectionId: 'connection-1',
        data: {
          tag: 'text',
          text: '{}'
        }
      })
    );

    const [code, reason] = await onceWithTimeout(ws, 'close', 'text connection.send close');
    expect(code).toBe(1011);
    expect(Buffer.from(reason as Buffer).toString('utf8')).toBe(
      'text JSON runtime protocol messages are not supported; use typed binary runtime frames'
    );
    expect(forwarded).toBe(false);
  });

  it('forwards identity connection.send from runtimes registered for that service', async () => {
    const manifest = loadWebSocketManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const forwarded: unknown[] = [];
    const unsubscribe = endpoint.onConnectionSend((message) => {
      forwarded.push(message);
    });
    trackResource({ close: () => unsubscribe() });

    const ws = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-actor-connection-send',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });

    ws.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'connection.send',
          serviceId: manifest.service.id,
          websocketEntryId: 'client',
          businessIdentity: 'actor-1',
          payloadKind: 'text'
        },
        Buffer.from('{}', 'utf8')
      )
    );

    await delay(10);
    expect(forwarded).toEqual([
      {
        type: 'connection.send',
        serviceId: manifest.service.id,
        websocketEntryId: 'client',
        businessIdentity: 'actor-1',
        payloadKind: 'text',
        payloadBytes: Buffer.from('{}', 'utf8')
      }
    ]);
  });

  it('forwards typed binary identity connection.send payloads as text or binary', async () => {
    const manifest = loadWebSocketManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const forwarded: unknown[] = [];
    const unsubscribe = endpoint.onConnectionSend((message) => {
      forwarded.push(message);
    });
    trackResource({ close: () => unsubscribe() });

    const textRuntime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-text-kind-connection-send',
      serviceId: manifest.service.id,
      revisionId: `${manifest.service.revisionId}-text-kind`,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });
    const binaryRuntime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-default-binary-kind-connection-send',
      serviceId: manifest.service.id,
      revisionId: `${manifest.service.revisionId}-binary-kind`,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });

    textRuntime.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'connection.send',
          serviceId: manifest.service.id,
          websocketEntryId: 'client',
          businessIdentity: 'actor-text-kind',
          payloadKind: 'text'
        },
        Buffer.from('hello typed text', 'utf8')
      )
    );
    binaryRuntime.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'connection.send',
          serviceId: manifest.service.id,
          websocketEntryId: 'client',
          businessIdentity: 'actor-binary-kind'
        },
        new Uint8Array([4, 5, 6])
      )
    );

    await delay(10);
    expect(forwarded).toEqual([
      {
        type: 'connection.send',
        serviceId: manifest.service.id,
        websocketEntryId: 'client',
        businessIdentity: 'actor-text-kind',
        payloadKind: 'text',
        payloadBytes: Buffer.from('hello typed text', 'utf8')
      },
      {
        type: 'connection.send',
        serviceId: manifest.service.id,
        websocketEntryId: 'client',
        businessIdentity: 'actor-binary-kind',
        payloadKind: 'binary',
        payloadBytes: Buffer.from([4, 5, 6])
      }
    ]);
  });

  it('closes typed text connection.send frames with invalid UTF-8 payloads', async () => {
    const manifest = loadWebSocketManifest();
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });

    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-invalid-text-kind-connection-send',
      serviceId: manifest.service.id,
      revisionId: `${manifest.service.revisionId}-invalid-text-kind`,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });

    runtime.send(
      encodeRuntimeFrame(
        {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: 'connection.send',
          serviceId: manifest.service.id,
          websocketEntryId: 'client',
          businessIdentity: 'actor-invalid-text-kind',
          payloadKind: 'text'
        },
        Buffer.from([0xff])
      )
    );

    const [code, reason] = await onceWithTimeout(
      runtime,
      'close',
      'invalid text connection.send close'
    );
    expect(code).toBe(1011);
    expect(Buffer.from(reason as Buffer).toString('utf8')).toBe(
      'connection.send text payload must be valid UTF-8'
    );
  });
});

async function dispatchBinaryJson(
  dispatcher: RuntimeDispatcher,
  request: RequestStartEnvelope,
  timeoutMs: number
): Promise<unknown> {
  const { type: _type, args: _args, ...metadata } = request;
  const header: RequestStartFrameHeader = {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    ...metadata
  };
  const response = await dispatcher.dispatchBinary(
    {
      header,
      payloadBytes: Buffer.from('opaque test payload')
    },
    timeoutMs
  );
  if (response.payloadBytes.byteLength === 0) {
    return null;
  }
  return JSON.parse(Buffer.from(response.payloadBytes).toString('utf8'));
}

function serviceRequestStart(input: {
  requestId: string;
  callerTarget: string;
  callerKind?: 'gateway' | 'service';
  mode?: DispatchMode;
  target: string;
  operationAbiId?: string;
  serviceId: string;
  version?: string;
  serviceProtocolIdentity: string;
  buildId?: string;
  timeoutMs?: number;
}): RequestStartFrameHeader {
  const timeoutMs = input.timeoutMs ?? 2000;
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: 'request.start',
    requestId: input.requestId,
    mode: input.mode ?? 'unary',
    caller: {
      kind: input.callerKind ?? 'service',
      target: input.callerTarget
    },
    target: input.target,
    operationAbiId: input.operationAbiId ?? `operation:test:${input.target}`,
    selector: `operation:${input.operationAbiId ?? `operation:test:${input.target}`}`,
    serviceId: input.serviceId,
    ...(input.version !== undefined ? { version: input.version } : {}),
    buildId: input.buildId ?? DEFAULT_TEST_BUILD_ID,
    serviceProtocolIdentity: input.serviceProtocolIdentity,
    deadline: {
      timeoutMs,
      expiresAt: new Date(Date.now() + timeoutMs).toISOString()
    },
    trace: {
      traceId: `${input.requestId}-trace`,
      spanId: `${input.requestId}-span`
    }
  };
}

function waitForAnyRuntimeRequestFrame(
  ws: WebSocket,
  label: string
): Promise<RuntimeRequestFrame> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'request.start') {
        return;
      }
      cleanup();
      resolve(frame as RuntimeRequestFrame);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

function waitForRuntimeResponseEnd(
  ws: WebSocket,
  requestId: string,
  label: string
): Promise<{
  header: ResponseEndFrameHeader;
  payload: string;
}> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'response.end' || frame.header.requestId !== requestId) {
        return;
      }
      cleanup();
      resolve({
        header: frame.header,
        payload: Buffer.from(frame.payloadBytes).toString('utf8')
      });
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

function waitForRuntimeResponseError(
  ws: WebSocket,
  requestId: string,
  label: string
): Promise<ResponseErrorFrameHeader> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.requestId !== requestId) {
        return;
      }
      if (frame.header.type === 'response.error') {
        cleanup();
        resolve(frame.header);
        return;
      }
      if (['response.start', 'response.chunk', 'response.end'].includes(frame.header.type)) {
        cleanup();
        reject(new Error(`received ${frame.header.type} before ${label}`));
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

function collectRuntimeResponseFrames(
  ws: WebSocket,
  requestId: string,
  count: number,
  label: string
): Promise<Array<{
  header: ResponseStartFrameHeader | ResponseChunkFrameHeader | ResponseEndFrameHeader;
  payload: string;
}>> {
  return new Promise((resolve, reject) => {
    const frames: Array<{
      header: ResponseStartFrameHeader | ResponseChunkFrameHeader | ResponseEndFrameHeader;
      payload: string;
    }> = [];
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (
        frame.header.requestId !== requestId ||
        !['response.start', 'response.chunk', 'response.end'].includes(frame.header.type)
      ) {
        return;
      }
      frames.push({
        header: frame.header as ResponseStartFrameHeader | ResponseChunkFrameHeader | ResponseEndFrameHeader,
        payload: Buffer.from(frame.payloadBytes).toString('utf8')
      });
      if (frames.length !== count) {
        return;
      }
      cleanup();
      resolve(frames);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}

function waitForRuntimeCancel(
  ws: WebSocket,
  requestId: string,
  label: string
): Promise<{
  isBinary: boolean;
  message: RequestCancelFrameHeader;
  payloadByteLength?: number;
}> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`timed out waiting for ${label}`));
    }, 1000);
    const onMessage = (data: WebSocket.RawData, isBinary: boolean) => {
      if (isBinary) {
        let frame: ReturnType<typeof decodeRuntimeFrame>;
        try {
          frame = decodeRuntimeFrame(data);
        } catch {
          return;
        }
        if (frame.header.type !== 'request.cancel' || frame.header.requestId !== requestId) {
          return;
        }
        cleanup();
        resolve({
          isBinary,
          message: frame.header,
          payloadByteLength: frame.payloadBytes.byteLength
        });
      }
    };
    const cleanup = () => {
      clearTimeout(timeout);
      ws.off('message', onMessage);
    };
    ws.on('message', onMessage);
  });
}
