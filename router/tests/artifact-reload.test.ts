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
  respondWithRawHttpRuntime,
  trackResource,
  waitForRouterControl
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

describe('router artifact reload', () => {

  it('sends artifact roots control metadata when a runtime connects', async () => {
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
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


  it('reloads active artifacts without restarting HTTP dispatch and broadcasts runtime control', async () => {
    const protocolV1 =
      'skiff-protocol-v1:sha256:5555555555555555555555555555555555555555555555555555555555555555';
    const protocolV2 =
      'skiff-protocol-v1:sha256:6666666666666666666666666666666666666666666666666666666666666666';
    // A protocol-identity change is a build change: V1 and V2 are distinct
    // builds with distinct buildIds. The HTTP gateway addresses each request by
    // the active manifest's build, and the boundary check passes because each
    // build's runtime carries the matching protocol identity.
    const buildV1 =
      'skiff-service-build-v1:sha256:0000000000000000000000000000000000000000000000000000000000005555';
    const buildV2 =
      'skiff-service-build-v1:sha256:0000000000000000000000000000000000000000000000000000000000006666';
    const manifestV1 = loadRawHttpManifest({
      protocolIdentity: protocolV1,
      buildId: buildV1
    });
    const manifestV2 = loadRawHttpManifest({
      protocolIdentity: protocolV2,
      buildId: buildV2
    });
    const snapshotV1: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        generation: 'generation-1',
        fingerprint: 'sha256:control-1',
        telemetry: telemetryControl
      },
      manifest: manifestV1
    };
    const snapshotV2: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        generation: 'generation-2',
        fingerprint: 'sha256:control-2',
        telemetry: telemetryControl
      },
      manifest: manifestV2
    };
    const snapshotStore = new RouterActiveSnapshotStore(snapshotV1);
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore,
      reloadArtifacts: async () => snapshotV2
    });
    const registryListen = await endpoint.listen({
      port: 0,
      control: snapshotV1.control!,
      controlPlane
    });

    const gateway = new HttpGateway({
      manifest: manifestV1,
      dispatcher,
      snapshotStore,
      port: 0,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();
    const controlUrl = registryListen.url.replace('ws://', 'http://').replace('/runtime', '');

    const runtimeV1 = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-reload-v1',
      serviceId: manifestV1.service.id,
      revisionId: 'revision-reload-v1',
      buildId: buildV1,
      serviceProtocolIdentity: protocolV1,
      targets: manifestV1.operations.map((operation) => operation.target)
    });
    respondWithRawHttpRuntime(runtimeV1, 'runtime-reload-v1');
    const runtimeV2 = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-reload-v2',
      serviceId: manifestV2.service.id,
      revisionId: 'revision-reload-v2',
      buildId: buildV2,
      serviceProtocolIdentity: protocolV2,
      targets: manifestV2.operations.map((operation) => operation.target)
    });
    respondWithRawHttpRuntime(runtimeV2, 'runtime-reload-v2');

    const before = await requestHttp({
      url: `${gatewayListen.url}/before-reload?service=skiff.run/sample`,
      headers: {
        Host: 'sample.local',
      }
    });
    expect(JSON.parse(before.body)).toEqual({
      buildId: buildV1,
      protocolIdentity: protocolV1,
      runtimeId: 'runtime-reload-v1'
    });

    const broadcastControl = waitForRouterControl(runtimeV1, 'sha256:control-2');
    const publicReload = await requestHttp({
      url: `${gatewayListen.url}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    expect(publicReload.status).toBe(404);
    const reload = await requestHttp({
      url: `${controlUrl}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    expect(reload.status).toBe(200);
    expect(JSON.parse(reload.body)).toMatchObject({
      ok: true,
      artifact: {
        devReload: true,
        generation: 'generation-2',
        fingerprint: 'sha256:control-2'
      },
      manifest: {
        protocolIdentity: protocolV2
      }
    });
    await expect(broadcastControl).resolves.toMatchObject({
      type: 'router.control',
      fingerprint: 'sha256:control-2',
      telemetry: telemetryControl
    });

    const healthResponse = await fetch(`${controlUrl}/__router/health`);
    expect(healthResponse.status).toBe(200);
    await expect(healthResponse.json()).resolves.toMatchObject({
      artifact: {
        devReload: true,
        generation: 'generation-2',
        fingerprint: 'sha256:control-2',
        telemetry: {
          enabled: true,
          endpointConfigured: true,
          protocol: TELEMETRY_PROTOCOL,
          topics: [...TELEMETRY_TOPICS],
          queueMaxEvents: 10000,
          batchMaxEvents: 200,
          batchMaxBytes: 262144,
          flushIntervalMs: 1000
        }
      },
      manifest: {
        protocolIdentity: protocolV2
      }
    });

    const after = await requestHttp({
      url: `${gatewayListen.url}/after-reload?service=skiff.run/sample`,
      headers: {
        Host: 'sample.local',
      }
    });
    expect(JSON.parse(after.body)).toEqual({
      buildId: buildV2,
      protocolIdentity: protocolV2,
      runtimeId: 'runtime-reload-v2'
    });
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


  it('shares one in-flight artifact reload across concurrent reload requests', async () => {
    const protocolV1 =
      'skiff-protocol-v1:sha256:7777777777777777777777777777777777777777777777777777777777777777';
    const protocolV2 =
      'skiff-protocol-v1:sha256:8888888888888888888888888888888888888888888888888888888888888888';
    const snapshotV1: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        generation: 'generation-1',
        fingerprint: 'sha256:control-1'
      },
      manifest: loadRawHttpManifest({ protocolIdentity: protocolV1 })
    };
    const snapshotV2: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        generation: 'generation-2',
        fingerprint: 'sha256:control-2'
      },
      manifest: loadRawHttpManifest({ protocolIdentity: protocolV2 })
    };
    const snapshotStore = new RouterActiveSnapshotStore(snapshotV1);
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    let reloadCalls = 0;
    let resolveReload!: (snapshot: RouterActiveSnapshot) => void;
    let markReloadStarted!: () => void;
    const reloadStarted = new Promise<void>((resolve) => {
      markReloadStarted = resolve;
    });
    const reloadGate = new Promise<RouterActiveSnapshot>((resolve) => {
      resolveReload = resolve;
    });

    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore,
      reloadArtifacts: async () => {
        reloadCalls += 1;
        markReloadStarted();
        return reloadGate;
      }
    });
    const registryListen = await endpoint.listen({ port: 0, controlPlane });
    const gateway = new HttpGateway({
      manifest: snapshotV1.manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();
    void gatewayListen;
    const controlUrl = registryListen.url.replace('ws://', 'http://').replace('/runtime', '');

    const firstReload = requestHttp({
      url: `${controlUrl}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    await reloadStarted;
    const secondReload = requestHttp({
      url: `${controlUrl}/__skiff/reload-artifacts`,
      method: 'POST'
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    resolveReload(snapshotV2);

    const [first, second] = await Promise.all([firstReload, secondReload]);
    expect(reloadCalls).toBe(1);
    expect(first.status).toBe(200);
    expect(second.status).toBe(200);
    expect(JSON.parse(first.body)).toMatchObject({
      artifact: {
        fingerprint: 'sha256:control-2'
      }
    });
    expect(JSON.parse(second.body)).toMatchObject({
      artifact: {
        fingerprint: 'sha256:control-2'
      }
    });
  });

  it('passes explicit artifact reload overrides to the control loader', async () => {
    const snapshotV1: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts'],
        devReload: true,
        generation: 'generation-1',
        fingerprint: 'sha256:control-1'
      },
      manifest: loadRawHttpManifest()
    };
    const snapshotV2: RouterActiveSnapshot = {
      activationByServiceOperation: buildActivationLookup([]),
      control: {
        artifactRoots: ['/tmp/skiff-artifacts', '/tmp/skiff-test-artifacts'],
        devReload: true,
        generation: 'generation-test',
        fingerprint: 'sha256:control-test'
      },
      manifest: loadRawHttpManifest({
        protocolIdentity:
          'skiff-protocol-v1:sha256:9999999999999999999999999999999999999999999999999999999999999999'
      })
    };
    const snapshotStore = new RouterActiveSnapshotStore(snapshotV1);
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const seenOverrides: unknown[] = [];
    const controlPlane = new RouterControlPlane({
      controlBroadcaster: endpoint,
      dispatcher,
      registry,
      snapshotStore,
      reloadArtifacts: async (overrides) => {
        seenOverrides.push(overrides);
        return snapshotV2;
      }
    });
    const registryListen = await endpoint.listen({ port: 0, controlPlane });
    const controlUrl = registryListen.url.replace('ws://', 'http://').replace('/runtime', '');

    const response = await requestHttp({
      url: `${controlUrl}/__skiff/reload-artifacts`,
      method: 'POST',
      headers: {
        'content-type': 'application/json'
      },
      body: JSON.stringify({
        artifactRoots: ['/tmp/skiff-test-artifacts', '/tmp/skiff-ephemeral-artifacts'],
        configProfile: 'test',
        serviceDb: {
          mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test'
        }
      })
    });

    expect(response.status).toBe(200);
    expect(seenOverrides).toEqual([
      {
        artifactRoots: ['/tmp/skiff-test-artifacts', '/tmp/skiff-ephemeral-artifacts'],
        configProfile: 'test',
        serviceDb: {
          mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test'
        }
      }
    ]);
    expect(JSON.parse(response.body)).toMatchObject({
      artifact: {
        artifactRoots: ['/tmp/skiff-artifacts', '/tmp/skiff-test-artifacts'],
        generation: 'generation-test',
        fingerprint: 'sha256:control-test'
      }
    });
  });
});
