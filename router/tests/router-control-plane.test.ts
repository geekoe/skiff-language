import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import {
  TELEMETRY_PROTOCOL,
  TELEMETRY_TOPICS,
  RUNTIME_FRAME_SCHEMA_VERSION,
  decodeRuntimeFrame
} from '../src/protocol/envelope.js';
import {
  RouterActiveSnapshotStore,
  type RouterActiveSnapshot
} from '../src/router/activeSnapshot.js';
import { buildActivationLookup } from '../src/artifacts/activationLookup.js';
import { HttpGateway } from '../src/router/httpGateway.js';
import { RouterControlPlane } from '../src/router/controlPlane.js';
import { onceWithTimeout } from './helpers/events.js';
import { hasRuntime, readHealth } from './helpers/health.js';
import { loadRawHttpManifest } from './helpers/manifests.js';
import { requestHttp } from './helpers/request.js';
import {
  closeTrackedResources,
  createRuntimeRouter,
  openRegisteredRuntime,
  trackResource
} from './helpers/runtime.js';

afterEach(closeTrackedResources);

const telemetryControl = {
  endpoint: 'ws://127.0.0.1:4002/telemetry',
  protocol: TELEMETRY_PROTOCOL,
  topics: [...TELEMETRY_TOPICS],
  queueMaxEvents: 10000,
  batchMaxEvents: 200,
  batchMaxBytes: 262144,
  flushIntervalMs: 1000,
  enabled: true
};

describe('router control listener', () => {

  it('sends artifact roots control metadata when a runtime connects', async () => {
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { endpoint } = runtimeRouter;
    const registryListen = await endpoint.listen({
      port: 0,
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        generation: 'generation-1',
        fingerprint: 'sha256:control',
        telemetry: telemetryControl
      }
    });

    const ws = new WebSocket(registryListen.url);
    trackResource({ close: () => ws.close() });
    const messagePromise = onceWithTimeout(ws, 'message', 'runtime control envelope');
    await onceWithTimeout(ws, 'open', 'runtime control socket open');
    const [data] = await messagePromise;

    const frame = decodeRuntimeFrame(data as WebSocket.RawData);
    expect(frame.header).toEqual({
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'router.control',
      artifactRoots: ['/tmp/skiff-artifacts'],
      generation: 'generation-1',
      fingerprint: 'sha256:control',
      telemetry: telemetryControl
    });
    expect(frame.payloadBytes.byteLength).toBe(0);
  });

  it('rejects the retired reload-artifacts control path on both listeners', async () => {
    const manifest = loadRawHttpManifest();
    const snapshot: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        generation: 'generation-1',
        fingerprint: 'sha256:control'
      },
      manifest
    };
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore
    });
    const registryListen = await endpoint.listen({
      port: 0,
      control: snapshot.control!,
      controlPlane
    });
    const gateway = new HttpGateway({
      manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      maxRequestBytes: 67108864,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();
    const controlUrl = registryListen.url.replace('ws://', 'http://').replace('/runtime', '');

    const controlReload = await requestHttp({
      url: `${controlUrl}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    expect(controlReload.status).toBe(404);
    const publicReload = await requestHttp({
      url: `${gatewayListen.url}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    expect(publicReload.status).toBe(404);
  });

  it('prunes runtime registrations from the control listener current snapshot', async () => {
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const manifest = loadRawHttpManifest();
    const staleBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000040aa';
    const currentBuild =
      'skiff-service-build-v1:sha256:00000000000000000000000000000000000000000000000000000000000040bb';
    const snapshot: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        mode: 'dev',
        serviceBuilds: [{
          buildId: currentBuild,
          serviceId: manifest.service.id,
          sourcePath: '/tmp/skiff-artifacts/dev/services/sample.json',
          version: '0.1.0'
        }]
      },
      manifest
    };
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore
    });
    const registryListen = await endpoint.listen({ port: 0, controlPlane });
    const gateway = new HttpGateway({
      manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      maxRequestBytes: 67108864,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();
    const controlUrl = registryListen.url.replace('ws://', 'http://').replace('/runtime', '');

    await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-control-prune-stale',
      serviceId: manifest.service.id,
      revisionId: 'revision-control-prune-stale',
      buildId: staleBuild,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });
    await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-control-prune-current',
      serviceId: manifest.service.id,
      revisionId: 'revision-control-prune-current',
      buildId: currentBuild,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });

    const publicPrune = await requestHttp({
      url: `${gatewayListen.url}/__router/prune-runtimes`,
      method: 'POST'
    });
    expect(publicPrune.status).toBe(404);

    const prune = await requestHttp({
      url: `${controlUrl}/__router/prune-runtimes`,
      method: 'POST'
    });
    expect(prune.status).toBe(200);
    expect(JSON.parse(prune.body)).toMatchObject({
      ok: true,
      deletedCount: 1,
      keptCount: 1,
      keep: [{
        buildId: currentBuild,
        serviceId: manifest.service.id
      }],
      deleted: [{
        runtimeId: 'runtime-control-prune-stale',
        buildId: staleBuild
      }],
      kept: [{
        runtimeId: 'runtime-control-prune-current',
        buildId: currentBuild
      }]
    });

    const health = await readHealth(controlUrl);
    expect(hasRuntime(health, 'runtime-control-prune-stale')).toBe(false);
    expect(hasRuntime(health, 'runtime-control-prune-current')).toBe(true);
  });
});
