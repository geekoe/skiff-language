import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { ActivationLookup } from '../src/artifacts/activationLookup.js';
import { decodeRuntimeFrame } from '../src/protocol/envelope.js';
import { RouterActiveSnapshotStore } from '../src/router/activeSnapshot.js';
import { RouterControlPlane } from '../src/router/controlPlane.js';

import { onceWithTimeout } from './helpers/events.js';
import {
  DEFAULT_TEST_BUILD_ID,
  loadRawHttpManifest
} from './helpers/manifests.js';
import { requestHttp } from './helpers/request.js';
import {
  closeTrackedResources,
  collectRuntimeRequestFrames,
  createRuntimeRouter,
  sendRuntimeBinaryResponse,
  trackResource
} from './helpers/runtime.js';

afterEach(closeTrackedResources);

describe('router test dispatch lazy runtime routing', () => {
  it('dispatches a complete missing-target request body to a lazy runtime connection', async () => {
    const manifest = loadRawHttpManifest();
    const target = 'skiff.test.lazy';
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
    const lazyRuntime = new WebSocket(listen.url);
    trackResource({ close: () => lazyRuntime.close() });
    await onceWithTimeout(lazyRuntime, 'open', 'lazy runtime socket open');
    lazyRuntime.on('message', (data) => {
      let frame: ReturnType<typeof decodeRuntimeFrame>;
      try {
        frame = decodeRuntimeFrame(data);
      } catch {
        return;
      }
      if (frame.header.type !== 'request.start') {
        return;
      }
      sendRuntimeBinaryResponse(lazyRuntime, frame.header.requestId, 'lazy response');
    });
    const requestFrames = collectRuntimeRequestFrames(
      lazyRuntime,
      1,
      'lazy test dispatch request'
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
        serviceProtocolIdentity: manifest.service.protocolIdentity,
        target,
        operationAbiId: `operation:test:${target}`,
        mode: 'unary',
        timeoutMs: 1234
      })
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toMatchObject({
      ok: true,
      header: {
        type: 'response.end',
        payloadPresent: true
      },
      payloadBase64: Buffer.from('lazy response').toString('base64')
    });
    const [requestFrame] = await requestFrames;
    expect(requestFrame!.header).toMatchObject({
      target,
      mode: 'unary',
      serviceId: manifest.service.id,
      buildId: DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity: manifest.service.protocolIdentity
    });
    expect(requestFrame!.header).not.toHaveProperty('activationIdentity');
    expect(requestFrame!.header.deadline?.timeoutMs).toBe(1234);
  });
});
