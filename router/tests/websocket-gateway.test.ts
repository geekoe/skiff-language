import type { IncomingMessage } from 'node:http';

import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { loadManifest, mergeLoadedManifests } from '../src/manifest/loadManifest.js';
import type { LoadedManifest } from '../src/manifest/types.js';
import { buildActivationLookup } from '../src/artifacts/activationLookup.js';
import { RouterActiveSnapshotStore, type RouterActiveSnapshot } from '../src/router/activeSnapshot.js';
import {
  encodeRuntimeFrame,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type WebSocketConnectionPolicyFrameMetadata
} from '../src/protocol/envelope.js';
import { readRedactedHeadersForDiagnostics } from '../src/router/bind.js';
import { closeSocket, collectMessages, delay, onceWithTimeout } from './helpers/events.js';
import { readHealth } from './helpers/health.js';
import {
  DEFAULT_TEST_BUILD_ID,
  loadWebSocketManifest,
  loadWebSocketManifestForService,
  webSocketRuntimeGatewayEntryIdentities
} from './helpers/manifests.js';
import { RouterHarness } from './helpers/routerHarness.js';
import {
  closeTrackedResources,
  trackResource,
  type RequestStartEnvelope
} from './helpers/runtime.js';
import { webSocketManifestValue } from './helpers/websocketFixtures.js';
import { openClientWithUpgrade } from './helpers/websocket.js';

afterEach(closeTrackedResources);

describe('router websocket gateway', () => {
  it('redacts host auth headers for websocket diagnostics', () => {
    const request = {
      rawHeaders: [
        'Authorization',
        'HostAuth test_secret',
        'X-Skiff-Host-Activation',
        'skiff_host_secret',
        'X-Client-Test',
        'visible'
      ],
      headers: {
        authorization: 'HostAuth test_secret',
        'x-skiff-host-activation': 'skiff_host_secret',
        'x-client-test': 'visible'
      }
    } as unknown as IncomingMessage;

    expect(readRedactedHeadersForDiagnostics(request)).toEqual([
      { name: 'authorization', value: '[redacted]' },
      { name: 'x-skiff-host-activation', value: '[redacted]' },
      { name: 'x-client-test', value: 'visible' }
    ]);

  });

  it('connects a single-service websocket path without a service query and exposes request data', async () => {
    const manifestValue = webSocketManifestValue();
    manifestValue.operations[0]!.parameters = [
      {
        name: 'request',
        schema: { type: 'any' }
      }
    ] as typeof manifestValue.operations[0]['parameters'];
    manifestValue.gateway.websocket.connect.adapterArgs = [
      { param: 'request', source: { kind: 'websocket.connectRequest' } }
    ];
    const manifest = loadManifest(manifestValue);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-single-service',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    const connectRequestPromise = new Promise<RequestStartEnvelope>((resolve) => {
      runtime.onRequest((request) => {
        resolve(request);
        runtime.sendResponse(request.requestId, websocketAccept('single-user'));
      });
    });

    const client = new WebSocket(
      harness.webSocketUrl('?deviceId=single-user&platform=web&clientVersion=1.0.0&language=en'),
      {
        headers: websocketHeaders('session-from-cookie', {
          cookie: 'theme=dark',
          'x-client-test': 'visible'
        })
      }
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'single-service websocket open');

    const connectRequest = await connectRequestPromise;
    expect(connectRequest.websocketAdapter?.connectRequest).toMatchObject({
      connectionId: expect.any(String),
      query: expect.arrayContaining([
        { name: 'deviceId', value: 'single-user' },
        { name: 'platform', value: 'web' }
      ]),
      headers: expect.arrayContaining([
        { name: 'x-client-test', value: 'visible' }
      ]),
      cookies: expect.arrayContaining([
        { name: 'sessionId', value: 'session-from-cookie' },
        { name: 'theme', value: 'dark' }
      ])
    });
    expect(connectRequest.selector).toBe(
      `operation:${manifest.websocketEntry!.connect!.operationAbiId}`
    );
  });

  it('dispatches websocket connect and receive with typed service route targets', async () => {
    const manifestValue = webSocketManifestValue();
    const connectTarget = 'runtime.websocket_fixture.WebSocketFixtureConnection.connect';
    const receiveTarget = 'runtime.websocket_fixture.WebSocketFixtureConnection.receive';
    manifestValue.operations[0]!.target = connectTarget;
    manifestValue.operations[1]!.target = receiveTarget;
    const manifest = loadManifest(manifestValue);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-typed-routes',
      targets: [connectTarget, receiveTarget],
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    const connectRequests = runtime.collectRequests(1, 'typed route websocket connect');
    runtime.onRequest((request) => {
      runtime.sendResponse(
        request.requestId,
        request.target === connectTarget ? websocketAccept('typed-user') : null
      );
    });

    const client = new WebSocket(
      harness.webSocketUrl('?deviceId=typed-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('typed-route-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'typed route websocket open');
    const [connectRequest] = await connectRequests;
    expect(connectRequest).toMatchObject({
      target: connectTarget,
      selector: `operation:${manifest.websocketEntry!.connect!.operationAbiId}`
    });

    const receiveRequests = runtime.collectRequests(1, 'typed route websocket receive');
    client.send(JSON.stringify({ tag: 'typed_route', requestId: 'typed-1' }));
    const [receiveRequest] = await receiveRequests;

    expect(receiveRequest).toMatchObject({
      target: receiveTarget,
      selector: `operation:${manifest.websocketEntry!.receive.operationAbiId}`,
      buildId: manifest.websocketEntry!.buildId
    });
  });

  it('passes websocket receive platform metadata without decoding handler args', async () => {
    const manifestValue = webSocketManifestValue();
    const manifest = loadManifest(manifestValue);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-nullable-missing-bind',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    runtime.onRequest((request) => {
      runtime.sendResponse(
        request.requestId,
        request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect'
          ? websocketAccept('nullable-user')
          : null
      );
    });

    const client = new WebSocket(
      harness.webSocketUrl('?deviceId=nullable-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('nullable-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'nullable missing bind websocket open');

    const receiveRequests = runtime.collectRequests(1, 'nullable missing bind receive');
    client.send(JSON.stringify({ tag: 'nullable_probe' }));
    const [receiveRequest] = await receiveRequests;

    expect(receiveRequest).toMatchObject({
      target: 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.receive',
      args: {},
      websocketAdapter: {
        kind: 'receive',
        receiveEvent: {
          connectionId: expect.any(String),
          message: {
            tag: 'text',
            encoding: 'utf8'
          },
          payloadSegments: [
            { kind: 'websocket.context', offset: 0, length: expect.any(Number) },
            { kind: 'websocket.message', offset: expect.any(Number), length: expect.any(Number) }
          ]
        }
      }
    });
  });

  it('uses router-owned websocket path when the manifest omits deprecated path', async () => {
    const manifestValue = webSocketManifestValue();
    delete (manifestValue.gateway.websocket as { path?: string }).path;
    const manifest = loadManifest(manifestValue);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-router-owned-path',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    runtime.respondWebSocketAccept(() => ({ userId: 'router-path-user' }));

    const client = new WebSocket(
      harness.webSocketUrl('?deviceId=router-path-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('router-path-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'router-owned path websocket open');

    expect(harness.webSocketListen?.url.endsWith('/ws')).toBe(true);
  });

  it('rejects websocket routes in loaded manifests', () => {
    const manifestValue = webSocketManifestValue() as any;
    manifestValue.gateway.websocket.routes = [
      {
        path: '/club/create',
        operation: 'WebSocketFixtureConnection.receive',
        bind: {
          message: 'message.payload'
        }
      }
    ];

    expect(() => loadManifest(manifestValue)).toThrow(
      /websocket client\.routes are no longer supported/
    );
  });

  it('routes websocket version query and header selectors to the selected build', async () => {
    const baseManifest = loadWebSocketManifest();
    const buildA =
      'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const buildB =
      'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const snapshot = webSocketVersionSnapshot({
      manifest: baseManifest,
      builds: [buildA, buildB],
      versions: {
        'ios-1.0.0': buildA,
        'web-1.0.0': buildB
      }
    });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const harness = await RouterHarness.create({ manifest: snapshot.manifest });
    await harness.listenWebSocket({ snapshotStore });

    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-ws-build-a',
      buildId: buildA,
      targets: baseManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(baseManifest)
    });
    const runtimeB = await harness.registerRuntime({
      runtimeId: 'runtime-ws-build-b',
      buildId: buildB,
      targets: baseManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(baseManifest)
    });
    const buildARequests = runtimeA.collectRequests(1, 'websocket build A connect');
    const buildBRequests = runtimeB.collectRequests(1, 'websocket build B connect');
    runtimeA.respondWebSocketAccept(() => ({ userId: 'ios-user', deviceId: 'ios-user' }));
    runtimeB.respondWebSocketAccept(() => ({ userId: 'web-user', deviceId: 'web-user' }));

    const queryClient = new WebSocket(
      harness.webSocketUrl('?version=ios-1.0.0&deviceId=ios-user&platform=ios&clientVersion=1.0.0&language=en'),
      websocketOptions('ios-session')
    );
    const headerClient = new WebSocket(
      harness.webSocketUrl('?deviceId=web-user&platform=web&clientVersion=1.0.0&language=en'),
      {
        headers: websocketHeaders('web-session', {
          'x-skiff-version': 'web-1.0.0'
        })
      }
    );
    trackResource({ close: () => queryClient.close() });
    trackResource({ close: () => headerClient.close() });
    await onceWithTimeout(queryClient, 'open', 'version query websocket open');
    await onceWithTimeout(headerClient, 'open', 'version header websocket open');

    const [requestA] = await buildARequests;
    const [requestB] = await buildBRequests;
    expect(requestA!.buildId).toBe(buildA);
    expect(requestB!.buildId).toBe(buildB);
  });

  it('passes the selected version through websocket receive request metadata', async () => {
    const manifestValue = webSocketManifestValue();
    const baseManifest = loadManifest(manifestValue);
    const buildId =
      'skiff-service-build-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    const snapshot = webSocketVersionSnapshot({
      manifest: baseManifest,
      builds: [buildId],
      versions: {
        dev: buildId
      }
    });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const harness = await RouterHarness.create({ manifest: snapshot.manifest });
    await harness.listenWebSocket({ snapshotStore });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-version-receive',
      buildId,
      targets: baseManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(baseManifest)
    });
    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtime.sendResponse(request.requestId, websocketAccept('version-user'));
        return;
      }
      runtime.sendResponse(request.requestId, null);
    });

    const client = new WebSocket(
      harness.webSocketUrl('?version=dev&deviceId=version-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('version-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'version receive websocket open');

    const receiveRequests = runtime.collectRequests(1, 'version receive request');
    client.send(JSON.stringify({ tag: 'version_probe' }));
    const [receiveRequest] = await receiveRequests;

    expect(receiveRequest).toMatchObject({
      buildId,
      args: {},
      websocketEntryId: 'client',
      websocketAdapter: {
        receiveEvent: {
          connectionId: expect.any(String),
          message: {
            tag: 'text',
            encoding: 'utf8'
          }
        }
      }
    });
  });

  it('routes /ws service and version query selectors without a version header', async () => {
    const manifest = loadWebSocketManifestForService(
      'skiff.run/sample',
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004'
    );
    const buildDev =
      'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const buildProd =
      'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const snapshot = webSocketVersionSnapshot({
      manifest,
      builds: [buildDev, buildProd],
      versions: {
        dev: buildDev,
        prod: buildProd
      }
    });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const harness = await RouterHarness.create({ manifest: snapshot.manifest });
    await harness.listenWebSocket({ snapshotStore });

    const runtimeDev = await harness.registerRuntime({
      runtimeId: 'runtime-sample-dev-ws',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: buildDev,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    const connectRequests = runtimeDev.collectRequests(1, 'sample dev websocket connect');
    runtimeDev.respondWebSocketAccept(() => ({ userId: 'sample-user', deviceId: 'sample-user' }));

    const client = new WebSocket(
      harness.webSocketUrl('?service=skiff.run/sample&version=dev&deviceId=sample-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('sample-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'sample dev websocket open');

    const [connectRequest] = await connectRequests;
    expect(connectRequest).toMatchObject({
      buildId: buildDev,
      caller: {
        target: 'gateway.skiff~run~~sample.websocket.client.connect'
      },
      target: 'service.skiff~run~~sample.SampleConnection.connect',
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
  });

  it('keeps websocket receive dispatch on the connection gateway identity after a new entry revision registers', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });
    const entry = manifest.websocketEntry!;
    const gatewayEntryG2 =
      'skiff-gateway-v1:sha256:2222222222222222222222222222222222222222222222222222222222222222';

    const runtimeG1 = await harness.registerRuntime({
      runtimeId: 'runtime-ws-g1',
      revisionId: 'revision-g1',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: [
        entry.connect!.gatewayEntryIdentity,
        entry.receive.gatewayEntryIdentity
      ]
    });
    const runtimeG1Requests: RequestStartEnvelope[] = [];
    runtimeG1.onRequest((request) => {
      runtimeG1Requests.push(request);
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtimeG1.sendResponse(request.requestId, websocketAccept('u-g1'));
        return;
      }
      runtimeG1.sendResponse(request.requestId, {
        tag: 'text',
        text: JSON.stringify({ runtimeId: 'runtime-ws-g1' })
      });
    });

    const client = new WebSocket(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=u-g1&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('sticky-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'sticky client open');

    const runtimeG2 = await harness.registerRuntime({
      runtimeId: 'runtime-ws-g2',
      revisionId: 'revision-g2',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: [gatewayEntryG2]
    });
    const runtimeG2Requests: RequestStartEnvelope[] = [];
    runtimeG2.onRequest((request) => {
      runtimeG2Requests.push(request);
      runtimeG2.sendResponse(request.requestId, {
        tag: 'text',
        text: JSON.stringify({ runtimeId: 'runtime-ws-g2' })
      });
    });

    const receiveRequestPromise = runtimeG1.collectRequests(1, 'sticky receive request');
    const unexpectedMessage = collectMessages(
      client,
      1,
      'unexpected sticky receive response',
      150
    ).then(
      () => true,
      () => false
    );
    client.send(JSON.stringify({ tag: 'fixture_ping', requestId: 'sticky-1' }));
    const [receiveRequest] = await receiveRequestPromise;

    await expect(unexpectedMessage).resolves.toBe(false);
    expect(receiveRequest!.gatewayEntryIdentity).toBe(entry.receive.gatewayEntryIdentity);
    expect(runtimeG1Requests.map((request) => request.gatewayEntryIdentity)).toContain(
      entry.receive.gatewayEntryIdentity
    );
    expect(runtimeG2Requests).toHaveLength(0);
  });


  it('dispatches websocket connect and raw receive to the selected client-facing service', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-test-ws-connection',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    const seenRequests: RequestStartEnvelope[] = [];
    runtime.onRequest((request) => {
      seenRequests.push(request);
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        const input = request.websocketAdapter?.connectRequest;
        runtime.sendResponse(
          request.requestId,
          websocketAccept(nameValue(input?.query, 'deviceId') ?? 'u1')
        );
        return;
      }

      sendRuntimeTextConnection(runtime.ws, {
        serviceId: manifest.service.id,
        connectionId: String(request.websocketAdapter?.receiveEvent?.connectionId),
        text: JSON.stringify({ tag: 'user_data', userId: request.businessIdentity })
      });
      runtime.sendResponse(request.requestId, {
        tag: 'text',
        text: JSON.stringify({
          tag: 'fixture_ping_response',
          requestId: 'app-1',
          ok: true
        })
      });
    });

    const client = new WebSocket(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=u1&platform=web&clientVersion=1.0.0&language=en&userId=evil'),
      {
        headers: websocketHeaders('u1-session', {
          'x-test-user-id': 'evil'
        })
      }
    );
    trackResource({ close: () => client.close() });

    await onceWithTimeout(client, 'open', 'client open');

    const messagesPromise = collectMessages(client, 1, 'connection.send push');
    const unexpectedResponse = collectMessages(
      client,
      2,
      'unexpected receive auto response',
      150
    ).then(
      () => true,
      () => false
    );
    client.send(JSON.stringify({ tag: 'fixture_ping', requestId: 'app-1', input: { name: 'A' } }));

    const messages = (await messagesPromise).map((data) => JSON.parse(String(data)));
    expect(messages[0]).toEqual({ tag: 'user_data', userId: 'u1' });
    await expect(unexpectedResponse).resolves.toBe(false);

    expect(seenRequests).toHaveLength(2);
    expect(seenRequests[0]).toMatchObject({
      target: 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect',
      websocketAdapter: {
        connectRequest: {
          query: expect.arrayContaining([
            { name: 'deviceId', value: 'u1' },
            { name: 'userId', value: 'evil' }
          ])
        }
      }
    });
    expect(seenRequests[1]).toMatchObject({
      businessIdentity: 'u1',
      target: 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.receive',
      args: {},
      websocketAdapter: {
        receiveEvent: {
        connectionId: expect.any(String),
        message: {
          tag: 'text',
            encoding: 'utf8'
          }
        }
      }
    });
  });

  it('forwards binary runtime connection.send frames to websocket clients', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-test-ws-binary-connection-send',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtime.sendResponse(request.requestId, websocketAccept('binary-user'));
        return;
      }

      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'connection.send',
            serviceId: manifest.service.id,
            websocketEntryId: 'client',
            businessIdentity: 'binary-user'
          },
          new Uint8Array([0, 255, 65])
        )
      );
      runtime.sendResponse(request.requestId, null);
    });

    const client = new WebSocket(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=binary-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('binary-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'binary client open');

    const messagesPromise = collectMessages(client, 1, 'binary connection.send push');
    client.send(JSON.stringify({ tag: 'fixture_ping', requestId: 'binary-1', input: { name: 'A' } }));
    const [message] = await messagesPromise;

    expect(Buffer.from(message as Buffer)).toEqual(Buffer.from([0, 255, 65]));
  });

  it('forwards typed text runtime connection.send frames to websocket clients', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-test-ws-text-binary-frame-connection-send',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtime.sendResponse(request.requestId, websocketAccept('text-frame-user'));
        return;
      }

      runtime.ws.send(
        encodeRuntimeFrame(
          {
            schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
            type: 'connection.send',
            serviceId: manifest.service.id,
            websocketEntryId: 'client',
            businessIdentity: 'text-frame-user',
            payloadKind: 'text'
          },
          Buffer.from(
            JSON.stringify({
              tag: 'typed_text_response',
              requestId: 'text-frame-1',
              ok: true
            }),
            'utf8'
          )
        )
      );
      runtime.sendResponse(request.requestId, null);
    });

    const client = new WebSocket(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=text-frame-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('text-frame-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'text frame client open');

    const messagesPromise = collectMessages(client, 1, 'typed text connection.send push');
    client.send(JSON.stringify({ tag: 'fixture_ping', requestId: 'text-frame-1', input: { name: 'A' } }));
    const [message] = await messagesPromise;

    expect(JSON.parse(String(message))).toEqual({
      tag: 'typed_text_response',
      requestId: 'text-frame-1',
      ok: true
    });
  });


  it('does not replay identity downlinks while the identity is offline', async () => {
    const manifestValue = webSocketManifestValue();
    manifestValue.operations[0]!.parameters = [{
      name: 'session',
      schema: { type: 'any' }
    }] as typeof manifestValue.operations[0]['parameters'];
    manifestValue.gateway.websocket.connect.adapterArgs = [
      { param: 'session', source: { kind: 'websocket.connectRequest' } }
    ];
    const manifest = loadManifest(manifestValue);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-test-ws-offline-cookie-connection-send',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    let connectCount = 0;
    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        connectCount += 1;
        runtime.sendResponse(
          request.requestId,
          websocketAccept(`offline-cookie-user-${connectCount}`)
        );
        return;
      }
      runtime.sendResponse(request.requestId, null);
    });

    const url = harness.webSocketUrl(
      '?service=example.com/websocket_fixture&platform=web&clientVersion=1.0.0&language=en'
    );
    const headers = { cookie: 'sessionId=offline-cookie-session' };
    const client = new WebSocket(url, { headers });
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'offline cookie client open');

    await closeSocket(client, 'offline cookie client close');
    sendRuntimeTextConnection(runtime.ws, {
      serviceId: manifest.service.id,
      identity: 'offline-cookie-user-1',
      text: JSON.stringify({
        tag: 'offline_cookie_response',
        requestId: 'offline-cookie-1',
        ok: true
      })
    });
    await delay(10);

    const reconnected = new WebSocket(url, { headers });
    trackResource({ close: () => reconnected.close() });
    const unexpectedReplay = collectMessages(
      reconnected,
      1,
      'unexpected offline cookie replay',
      150
    ).then(
      () => true,
      () => false
    );
    await onceWithTimeout(reconnected, 'open', 'offline cookie reconnect open');
    await expect(unexpectedReplay).resolves.toBe(false);
  });


  it('fans out identity downlinks to all open tabs for the identity', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-test-ws-offline-multitab-connection-send',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtime.sendResponse(request.requestId, websocketAccept('offline-multitab-user'));
        return;
      }
      runtime.sendResponse(request.requestId, null);
    });

    const url = harness.webSocketUrl(
      '?service=example.com/websocket_fixture&deviceId=offline-multitab-user&platform=web&clientVersion=1.0.0&language=en'
    );
    const headers = websocketHeaders('offline-multitab-session');
    const closedTab = new WebSocket(url, { headers });
    trackResource({ close: () => closedTab.close() });
    await onceWithTimeout(closedTab, 'open', 'offline multitab closed tab open');

    const olderOpenTab = new WebSocket(url, { headers });
    trackResource({ close: () => olderOpenTab.close() });
    await onceWithTimeout(olderOpenTab, 'open', 'offline multitab older open tab open');

    const latestOpenTab = new WebSocket(url, { headers });
    trackResource({ close: () => latestOpenTab.close() });
    await onceWithTimeout(latestOpenTab, 'open', 'offline multitab latest open tab open');

    await closeSocket(closedTab, 'offline multitab closed tab close');
    await delay(10);

    const olderMessage = collectMessages(olderOpenTab, 1, 'offline multitab older tab message');
    const latestMessage = collectMessages(latestOpenTab, 1, 'offline multitab latest tab message');

    sendRuntimeTextConnection(runtime.ws, {
      serviceId: manifest.service.id,
      identity: 'offline-multitab-user',
      text: JSON.stringify({
        tag: 'offline_multitab_response',
        ok: true
      })
    });

    const [olderData] = await olderMessage;
    const [latestData] = await latestMessage;
    expect(JSON.parse(String(olderData))).toEqual({
      tag: 'offline_multitab_response',
      ok: true
    });
    expect(JSON.parse(String(latestData))).toEqual({
      tag: 'offline_multitab_response',
      ok: true
    });
  });

  it('enforces identity close-oldest policy before immediate identity delivery', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-test-ws-close-oldest-policy',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtime.sendResponse(
          request.requestId,
            websocketAccept('host-identity', {
            maxConnections: 1,
            overflow: 'close-oldest',
            closeCode: 4009,
            closeReason: 'host connection replaced'
          })
        );
        return;
      }
      runtime.sendResponse(request.requestId, null);
    });

    const url = harness.webSocketUrl(
      '?service=example.com/websocket_fixture&deviceId=host-identity&platform=host&clientVersion=1.0.0&language=en'
    );
    const older = new WebSocket(url, websocketOptions('host-session-older'));
    trackResource({ close: () => older.close() });
    await onceWithTimeout(older, 'open', 'close-oldest older open');

    const olderClose = onceWithTimeout(older, 'close', 'close-oldest older close');
    const olderUnexpectedMessage = collectMessages(
      older,
      1,
      'close-oldest older unexpected identity message',
      150
    ).then(
      () => true,
      () => false
    );

    const latest = new WebSocket(url, websocketOptions('host-session-latest'));
    trackResource({ close: () => latest.close() });
    await onceWithTimeout(latest, 'open', 'close-oldest latest open');

    const latestMessage = collectMessages(latest, 1, 'close-oldest latest identity message');
    sendRuntimeTextConnection(runtime.ws, {
      serviceId: manifest.service.id,
      identity: 'host-identity',
      text: JSON.stringify({
        tag: 'host_identity_message',
        ok: true
      })
    });

    const [latestData] = await latestMessage;
    expect(JSON.parse(String(latestData))).toEqual({
      tag: 'host_identity_message',
      ok: true
    });
    await expect(olderUnexpectedMessage).resolves.toBe(false);

    const [closeCode, closeReason] = (await olderClose) as [number, Buffer];
    expect(closeCode).toBe(4009);
    expect(closeReason.toString()).toBe('host connection replaced');
  });

  it('applies identity connectionPolicy across builds for the same service id', async () => {
    const baseManifest = loadWebSocketManifest();
    const buildA =
      'skiff-service-build-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111';
    const buildB =
      'skiff-service-build-v1:sha256:2222222222222222222222222222222222222222222222222222222222222222';
    const snapshot = webSocketVersionSnapshot({
      manifest: baseManifest,
      builds: [buildA, buildB],
      versions: {
        'host-1.0.0': buildA,
        'host-2.0.0': buildB
      }
    });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const harness = await RouterHarness.create({ manifest: snapshot.manifest });
    await harness.listenWebSocket({ snapshotStore });

    const runtimeA = await harness.registerRuntime({
      runtimeId: 'runtime-ws-policy-build-a',
      buildId: buildA,
      targets: baseManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(baseManifest)
    });
    const runtimeB = await harness.registerRuntime({
      runtimeId: 'runtime-ws-policy-build-b',
      buildId: buildB,
      targets: baseManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(baseManifest)
    });
    const acceptHostPolicy = {
      userId: 'stable-host-identity',
      deviceId: 'stable-host-identity',
      connectionPolicy: {
        maxConnections: 1,
        overflow: 'close-oldest',
        closeCode: 4009,
        closeReason: 'host connection replaced'
      }
    } satisfies Parameters<typeof runtimeA.respondWebSocketAccept>[0];
    runtimeA.respondWebSocketAccept(acceptHostPolicy);
    runtimeB.respondWebSocketAccept(acceptHostPolicy);

    const older = new WebSocket(
      harness.webSocketUrl(
        '?version=host-1.0.0&deviceId=stable-host-identity&platform=host&clientVersion=1.0.0&language=en'
      ),
      websocketOptions('host-build-session-older')
    );
    trackResource({ close: () => older.close() });
    await onceWithTimeout(older, 'open', 'policy build older open');

    const olderClose = onceWithTimeout(older, 'close', 'policy build older close');
    const latest = new WebSocket(
      harness.webSocketUrl(
        '?version=host-2.0.0&deviceId=stable-host-identity&platform=host&clientVersion=2.0.0&language=en'
      ),
      websocketOptions('host-build-session-latest')
    );
    trackResource({ close: () => latest.close() });
    await onceWithTimeout(latest, 'open', 'policy build latest open');

    const [closeCode] = (await olderClose) as [number, Buffer];
    expect(closeCode).toBe(4009);
  });

  it.each([
    {
      name: 'policy without identity',
      expectedStatus: 502,
      payload: {
        tag: 'accept',
        context: {
          userId: 'invalid-policy-user',
          deviceId: 'invalid-policy-user',
          platform: 'web',
          clientVersion: '1.0.0',
          language: 'en'
        },
        connectionPolicy: {
          maxConnections: 1,
          overflow: 'close-oldest'
        }
      }
    },
    {
      name: 'unsupported scope',
      expectedStatus: 503,
      payload: websocketAccept('invalid-policy-user', {
        scope: 'identity',
        maxConnections: 1,
        overflow: 'close-oldest'
      } as WebSocketConnectionPolicyFrameMetadata)
    },
    {
      name: 'unsupported overflow',
      expectedStatus: 503,
      payload: websocketAccept('invalid-policy-user', {
        maxConnections: 1,
        overflow: 'drop-new'
      } as unknown as WebSocketConnectionPolicyFrameMetadata)
    },
    {
      name: 'invalid maxConnections',
      expectedStatus: 503,
      payload: websocketAccept('invalid-policy-user', {
        maxConnections: 0,
        overflow: 'close-oldest'
      })
    },
    {
      name: 'invalid closeCode',
      expectedStatus: 502,
      payload: websocketAccept('invalid-policy-user', {
        maxConnections: 1,
        overflow: 'close-oldest',
        closeCode: 2999
      })
    },
    {
      name: 'too long closeReason',
      expectedStatus: 502,
      payload: websocketAccept('invalid-policy-user', {
        maxConnections: 1,
        overflow: 'close-oldest',
        closeReason: 'x'.repeat(124)
      })
    }
  ])('rejects invalid websocket connectionPolicy: $name', async ({ name, payload, expectedStatus }) => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: `runtime-test-ws-invalid-policy-${name.replaceAll(' ', '-')}`,
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    runtime.onRequest((request) => {
      runtime.sendResponse(request.requestId, payload);
    });

    await expectWebSocketUpgradeRejected(
      harness.webSocketUrl(
        '?service=example.com/websocket_fixture&deviceId=invalid-policy-user&platform=web&clientVersion=1.0.0&language=en'
      ),
      websocketOptions('invalid-policy-session').headers,
      'invalid websocket connectionPolicy',
      expectedStatus
    );
  });


  it('uses the websocket service query to select from merged service manifests', async () => {
    const websocket_fixtureManifest = loadWebSocketManifest();
    const chatManifest = loadWebSocketManifestForService(
      'skiff.run/chat',
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004'
    );
    const manifest = mergeLoadedManifests([websocket_fixtureManifest, chatManifest]);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-chat-ws',
      serviceId: chatManifest.service.id,
      revisionId: chatManifest.service.revisionId,
      serviceProtocolIdentity: chatManifest.service.protocolIdentity,
      targets: chatManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: [
        chatManifest.websocketEntry!.connect!.gatewayEntryIdentity,
        chatManifest.websocketEntry!.receive.gatewayEntryIdentity
      ]
    });

    const connectRequestPromise = new Promise<RequestStartEnvelope>((resolve) => {
      runtime.onRequest((request) => {
        resolve(request);
        runtime.sendResponse(request.requestId, websocketAccept('chat-user'));
      });
    });

    const client = new WebSocket(
      harness.webSocketUrl('?service=skiff.run/chat&deviceId=chat-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('chat-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'multi-service chat client open');
    const connectRequest = await connectRequestPromise;

    expect(connectRequest).toMatchObject({
      caller: {
        target: 'gateway.skiff~run~~chat.websocket.client.connect'
      },
      target: 'service.skiff~run~~chat.ChatConnection.connect',
      serviceProtocolIdentity: chatManifest.service.protocolIdentity,
      gatewayEntryIdentity: chatManifest.websocketEntry!.connect!.gatewayEntryIdentity
    });
  });

  it('uses X-Skiff-Service as a websocket service selector', async () => {
    const websocket_fixtureManifest = loadWebSocketManifest();
    const chatManifest = loadWebSocketManifestForService(
      'skiff.run/chat',
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004'
    );
    const manifest = mergeLoadedManifests([websocket_fixtureManifest, chatManifest]);
    const harness = await RouterHarness.websocket({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-service-header',
      serviceId: chatManifest.service.id,
      revisionId: chatManifest.service.revisionId,
      serviceProtocolIdentity: chatManifest.service.protocolIdentity,
      targets: chatManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(chatManifest)
    });
    const connectRequestPromise = runtime.collectRequests(1, 'websocket service header connect');
    runtime.respondWebSocketAccept(() => ({ userId: 'chat-user', deviceId: 'chat-user' }));

    const client = new WebSocket(
      harness.webSocketUrl('?deviceId=chat-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('chat-session', {
        'x-skiff-service': 'skiff.run/chat'
      })
    );
    trackResource({ close: () => client.close() });

    await onceWithTimeout(client, 'open', 'websocket service header open');
    const [connectRequest] = await connectRequestPromise;
    expect(connectRequest).toMatchObject({
      caller: {
        target: 'gateway.skiff~run~~chat.websocket.client.connect'
      },
      target: 'service.skiff~run~~chat.ChatConnection.connect',
      serviceProtocolIdentity: chatManifest.service.protocolIdentity,
      serviceId: chatManifest.service.id
    });
  });

  it('uses host rewrite to select a websocket service without client selectors', async () => {
    const websocket_fixtureManifest = loadWebSocketManifest();
    const chatManifest = loadWebSocketManifestForService(
      'skiff.run/chat',
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004'
    );
    const manifest = mergeLoadedManifests([websocket_fixtureManifest, chatManifest]);
    const harness = await RouterHarness.create({ manifest });
    await harness.listenWebSocket({
      rewrite: [
        {
          host: 'chat.localhost',
          service: 'skiff.run/chat'
        }
      ]
    });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-host-rewrite',
      serviceId: chatManifest.service.id,
      revisionId: chatManifest.service.revisionId,
      serviceProtocolIdentity: chatManifest.service.protocolIdentity,
      targets: chatManifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(chatManifest)
    });
    const connectRequestPromise = runtime.collectRequests(1, 'websocket host rewrite connect');
    runtime.respondWebSocketAccept(() => ({ userId: 'chat-user', deviceId: 'chat-user' }));

    const client = new WebSocket(
      harness.webSocketUrl('?deviceId=chat-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('chat-session', {
        host: 'Chat.Localhost:4000'
      })
    );
    trackResource({ close: () => client.close() });

    await onceWithTimeout(client, 'open', 'websocket host rewrite open');
    const [connectRequest] = await connectRequestPromise;
    expect(connectRequest).toMatchObject({
      serviceId: chatManifest.service.id,
      target: 'service.skiff~run~~chat.ChatConnection.connect'
    });
  });

  it('uses websocket service and version headers before query parameters', async () => {
    const manifest = loadWebSocketManifestForService(
      'skiff.run/sample',
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004'
    );
    const buildDev =
      'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const buildProd =
      'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const snapshot = webSocketVersionSnapshot({
      manifest,
      builds: [buildDev, buildProd],
      versions: {
        dev: buildDev,
        prod: buildProd
      }
    });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const harness = await RouterHarness.create({ manifest: snapshot.manifest });
    await harness.listenWebSocket({ snapshotStore });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-header-selector',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: buildProd,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    const connectRequestPromise = runtime.collectRequests(1, 'websocket header selector connect');
    runtime.respondWebSocketAccept(() => ({ userId: 'header-user', deviceId: 'header-user' }));

    const client = new WebSocket(
      harness.webSocketUrl('?service=business-service&version=business-version&deviceId=header-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('header-selector-session', {
        'x-skiff-service': 'skiff.run/sample',
        'x-skiff-version': 'prod'
      })
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'websocket header selector open');

    const [connectRequest] = await connectRequestPromise;
    expect(connectRequest).toMatchObject({
      buildId: buildProd,
      serviceId: manifest.service.id
    });
  });

  it('does not treat websocket version query as a selector for a single-build service', async () => {
    const manifest = loadWebSocketManifestForService(
      'skiff.run/sample',
      'skiff-protocol-v1:sha256:0000000000000000000000000000000000000000000000000000000000000004'
    );
    const harness = await RouterHarness.websocket({ manifest });
    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-version-business-query',
      serviceId: manifest.service.id,
      revisionId: manifest.service.revisionId,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    const connectRequestPromise = runtime.collectRequests(1, 'websocket business version query connect');
    runtime.respondWebSocketAccept(() => ({ userId: 'query-user', deviceId: 'query-user' }));

    const client = new WebSocket(
      harness.webSocketUrl('?version=business-version&deviceId=query-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('business-version-query-session', {
        'x-skiff-service': 'skiff.run/sample'
      })
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'websocket business version query open');

    const [connectRequest] = await connectRequestPromise;
    expect(connectRequest!).toMatchObject({
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceId: manifest.service.id,
      websocketAdapter: {
        connectRequest: {
          query: expect.arrayContaining([
            { name: 'clientVersion', value: '1.0.0' }
          ])
        }
      }
    });
  });


  it('uses temporary client sessions without interpreting fixed session cookies', async () => {
    const manifestValue = webSocketManifestValue();
    manifestValue.operations[0]!.parameters = [{
      name: 'session',
      schema: { type: 'any' }
    }] as typeof manifestValue.operations[0]['parameters'];
    manifestValue.gateway.websocket.connect.adapterArgs = [
      { param: 'session', source: { kind: 'websocket.connectRequest' } }
    ];
    const manifest = loadManifest(manifestValue);
    const harness = await RouterHarness.websocket({ manifest });

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-ws-credential',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });
    const requestsPromise = runtime.collectRequests(3, 'temporary session connect requests');
    runtime.respondWebSocketAccept((request) => ({
      userId: String((request.clientSession as { id: string }).id),
      deviceId: 'device'
    }));

    const generated = await openClientWithUpgrade(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=missing-device&platform=web&clientVersion=1.0.0&language=en'),
      'generated session client'
    );
    const existing = await openClientWithUpgrade(
      harness.webSocketUrl('?service=example.com/websocket_fixture&platform=web&clientVersion=1.0.0&language=en'),
      'existing other cookie client',
      websocketHeaders('existing-session', { cookie: 'other=1' })
    );
    const cookieWithDeviceId = await openClientWithUpgrade(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=query-device&platform=ios&clientVersion=1.0.0&language=en'),
      'session cookie with device id client',
      websocketHeaders('cookie-session-wins')
    );
    trackResource({ close: () => generated.client.close() });
    trackResource({ close: () => existing.client.close() });
    trackResource({ close: () => cookieWithDeviceId.client.close() });

    expect(generated.upgrade.headers['set-cookie']).toBeUndefined();
    expect(existing.upgrade.headers['set-cookie']).toBeUndefined();
    expect(cookieWithDeviceId.upgrade.headers['set-cookie']).toBeUndefined();

    const requests = await requestsPromise;
    const sessionIds = requests.map((request) => (request.clientSession as { id: string }).id);
    expect(sessionIds).toHaveLength(3);
    expect(new Set(sessionIds).size).toBe(3);
    expect(sessionIds).not.toContain('existing-session');
    expect(sessionIds).not.toContain('cookie-session-wins');
    for (const sessionId of sessionIds) {
      expect(sessionId).toEqual(expect.any(String));
      expect(sessionId.length).toBeGreaterThan(0);
    }
  });


  it('attaches client websocket upgrades to the HTTP gateway listener', async () => {
    const manifest = loadWebSocketManifest();
    const harness = await RouterHarness.combinedHttpWebSocket({ manifest });

    expect(new URL(harness.webSocketListen!.url).port).toBe(new URL(harness.httpListen!.url).port);
    await expect(
      readHealth(harness.registryListen!.url.replace('ws://', 'http://').replace('/runtime', ''))
    ).resolves.toEqual([]);

    const runtime = await harness.registerRuntime({
      runtimeId: 'runtime-shared-http-upgrade',
      targets: manifest.operations.map((operation) => operation.target),
      gatewayEntryIdentities: webSocketRuntimeGatewayEntryIdentities(manifest)
    });

    runtime.onRequest((request) => {
      if (request.target === 'service.example~com~~websocket_fixture.WebSocketFixtureConnection.connect') {
        runtime.sendResponse(request.requestId, websocketAccept('shared-port-user'));
        return;
      }

      sendRuntimeTextConnection(runtime.ws, {
        serviceId: manifest.service.id,
        connectionId: String(request.websocketAdapter?.receiveEvent?.connectionId),
        text: JSON.stringify({
          tag: 'shared_port_response',
          requestId: 'shared-port-1',
          ok: true
        })
      });
      runtime.sendResponse(request.requestId, null);
    });

    const client = new WebSocket(
      harness.webSocketUrl('?service=example.com/websocket_fixture&deviceId=shared-port-user&platform=web&clientVersion=1.0.0&language=en'),
      websocketOptions('shared-port-session')
    );
    trackResource({ close: () => client.close() });
    await onceWithTimeout(client, 'open', 'shared HTTP websocket open');

    const responsePromise = collectMessages(client, 1, 'shared HTTP websocket response');
    client.send(JSON.stringify({ tag: 'shared_port', requestId: 'shared-port-1' }));
    const [response] = await responsePromise;

    expect(JSON.parse(String(response))).toEqual({
      tag: 'shared_port_response',
      requestId: 'shared-port-1',
      ok: true
    });
  });
});

function websocketOptions(
  sessionId: string,
  extraHeaders: Record<string, string> = {}
): { headers: Record<string, string> } {
  return {
    headers: websocketHeaders(sessionId, extraHeaders)
  };
}

function websocketHeaders(
  sessionId: string,
  extraHeaders: Record<string, string> = {}
): Record<string, string> {
  const headers = { ...extraHeaders };
  headers.cookie = headers.cookie
    ? `${headers.cookie}; sessionId=${sessionId}`
    : `sessionId=${sessionId}`;
  return headers;
}

function nameValue(
  items: Array<{ name: string; value: string }> | undefined,
  name: string
): string | undefined {
  return items?.find((item) => item.name === name)?.value;
}

async function expectWebSocketUpgradeRejected(
  url: string,
  headers: Record<string, string>,
  label: string,
  expectedStatus = 502
): Promise<void> {
  const client = new WebSocket(url, { headers });
  trackResource({ close: () => client.close() });
  const [error] = (await onceWithTimeout(client, 'error', `${label} error`)) as [Error];
  expect(error.message).toContain(`Unexpected server response: ${expectedStatus}`);
}

function websocketAccept(
  userId: string,
  connectionPolicy?: WebSocketConnectionPolicyFrameMetadata
) {
  return {
    tag: 'accept',
    context: {
      userId,
      deviceId: userId,
      platform: 'web',
      clientVersion: '1.0.0',
      language: 'en'
    },
    identity: userId,
    ...(connectionPolicy !== undefined ? { connectionPolicy } : {})
  };
}

function sendRuntimeTextConnection(
  ws: WebSocket,
  input: {
    serviceId: string;
    identity?: string;
    websocketEntryId?: string;
    connectionId?: string;
    text: string;
  }
): void {
  ws.send(
    encodeRuntimeFrame(
      {
        schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
        type: 'connection.send',
        serviceId: input.serviceId,
        ...(input.identity
          ? { businessIdentity: input.identity, websocketEntryId: input.websocketEntryId ?? 'client' }
          : {}),
        ...(input.connectionId ? { connectionId: input.connectionId } : {}),
        payloadKind: 'text'
      },
      Buffer.from(input.text, 'utf8')
    )
  );
}

function webSocketVersionSnapshot(input: {
  manifest: LoadedManifest;
  builds: string[];
  versions: Record<string, string>;
}): RouterActiveSnapshot {
  const entries = input.builds.map((buildId) => ({
    ...input.manifest.websocketEntry!,
    buildId
  }));
  const manifest: LoadedManifest = {
    ...input.manifest,
    websocketEntry: entries[0]!,
    websocketEntries: entries
  };
  return {
    activationByServiceOperation: buildActivationLookup([]),
    control: {
      artifactRoots: ['/tmp/skiff-artifacts'],
      mode: 'release'
    },
    manifest,
    versionByService: new Map([
      [
        input.manifest.service.id,
        new Map(
          Object.entries(input.versions).map(([version, buildId]) => [
            version,
            {
              buildId,
              serviceId: input.manifest.service.id,
              version
            }
          ])
        )
      ]
    ])
  };
}
