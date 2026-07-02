import { afterEach, describe, expect, it } from 'vitest';

import { loadManifest as loadRuntimeManifest } from '../src/manifest/loadManifest.js';
import { RouterActiveSnapshotStore, type RouterActiveSnapshot } from '../src/router/activeSnapshot.js';
import { buildActivationLookup } from '../src/artifacts/activationLookup.js';
import { HttpGateway } from '../src/router/httpGateway.js';
import { httpRequestSchema, httpResponseSchema, loadRawHttpManifest } from './helpers/manifests.js';
import { requestHttp } from './helpers/request.js';
import {
  closeTrackedResources,
  collectRuntimeRequests,
  createRuntimeRouter,
  openRegisteredRuntime,
  respondWithRawHttpRuntime,
  trackResource
} from './helpers/runtime.js';

afterEach(closeTrackedResources);

function loadManifest(value: unknown) {
  addDefaultOperationAbiIds(value);
  return loadRuntimeManifest(value);
}

function addDefaultOperationAbiIds(value: unknown): void {
  if (typeof value !== 'object' || value === null || !Array.isArray((value as any).operations)) {
    return;
  }
  (value as any).operations.forEach((operation: any, index: number) => {
    if (typeof operation !== 'object' || operation === null || typeof operation.operationAbiId === 'string') {
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

describe('router version routing', () => {
  it('fails closed for unknown versions', async () => {
    const manifest = loadRawHttpManifest();
    const snapshot = versionSnapshot({
      manifest,
      versions: {
        'sample-ios-1.3.7':
          'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
      }
    });
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const gateway = new HttpGateway({
      manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();

    const response = await requestHttp({
      url: `${gatewayListen.url}/api?service=skiff.run/sample&version=sample-ios-9.9.9`
    });

    expect(response.status).toBe(404);
    expect(JSON.parse(response.body)).toEqual({
      message: 'No version sample-ios-9.9.9 is loaded for service skiff.run/sample',
      detail: null
    });
  });

  it('adds buildId to request.start and dispatches to the matching build runtime', async () => {
    const manifest = loadRawHttpManifest();
    const buildA =
      'skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const buildB =
      'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const snapshot = versionSnapshot({
      manifest,
      versions: {
        'sample-ios-1.3.7': buildB
      }
    });
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const gateway = new HttpGateway({
      manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();

    const runtimeA = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-build-a',
      serviceId: manifest.service.id,
      revisionId: 'revision-build-a',
      buildId: buildA,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });
    respondWithRawHttpRuntime(runtimeA, 'runtime-build-a');
    const runtimeB = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-build-b',
      serviceId: manifest.service.id,
      revisionId: 'revision-build-b',
      buildId: buildB,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });
    respondWithRawHttpRuntime(runtimeB, 'runtime-build-b');

    const response = await requestHttp({
      url: `${gatewayListen.url}/api?service=skiff.run/sample&version=sample-ios-1.3.7`
    });

    expect(response.status).toBe(200);
    expect(JSON.parse(response.body)).toEqual({
      buildId: buildB,
      protocolIdentity: manifest.service.protocolIdentity,
      runtimeId: 'runtime-build-b'
    });
  });

  it('keeps release build selection on typed service route targets', async () => {
    const typedTarget = 'runtime.sample.SampleHttpApi.handle';
    const manifest = loadManifest({
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
    });
    const buildId =
      'skiff-service-build-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    const snapshot = versionSnapshot({
      manifest,
      versions: {
        'sample-ios-typed': buildId
      }
    });
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const gateway = new HttpGateway({
      manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();

    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-typed-release',
      serviceId: manifest.service.id,
      revisionId: 'revision-typed-release',
      buildId,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: [typedTarget]
    });
    const requestsPromise = collectRuntimeRequests(runtime, 1, 'typed release route request');
    respondWithRawHttpRuntime(runtime, 'runtime-typed-release');

    const response = await requestHttp({
      url: `${gatewayListen.url}/api?service=skiff.run/sample&version=sample-ios-typed`
    });
    const [request] = await requestsPromise;

    expect(response.status).toBe(200);
    expect(request).toMatchObject({
      buildId,
      target: typedTarget
    });
  });

  it('uses service and version headers before compatibility query selectors', async () => {
    const manifest = loadRawHttpManifest();
    const buildId =
      'skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const snapshot = versionSnapshot({
      manifest,
      versions: {
        'sample-ios-1.3.7': buildId
      }
    });
    const runtimeRouter = trackResource(createRuntimeRouter());
    const { dispatcher, endpoint, registry } = runtimeRouter;
    const registryListen = await endpoint.listen({ port: 0 });
    const snapshotStore = new RouterActiveSnapshotStore(snapshot);
    const gateway = new HttpGateway({
      manifest,
      dispatcher,
      snapshotStore,
      port: 0,
      requestTimeoutMs: 2000
    });
    trackResource(gateway);
    const gatewayListen = await gateway.listen();

    const runtime = await openRegisteredRuntime(registryListen.url, {
      type: 'runtime.register',
      runtimeId: 'runtime-build-release-header',
      serviceId: manifest.service.id,
      revisionId: 'revision-build-release-header',
      buildId,
      serviceProtocolIdentity: manifest.service.protocolIdentity,
      targets: manifest.operations.map((operation) => operation.target)
    });
    respondWithRawHttpRuntime(runtime, 'runtime-build-release-header');

    const releaseHeaderResponse = await requestHttp({
      url: `${gatewayListen.url}/api`,
      headers: {
        'X-Skiff-Service': 'skiff.run/sample',
        'X-Skiff-Release': 'sample-ios-1.3.7'
      }
    });
    expect(releaseHeaderResponse.status).toBe(200);
    expect(JSON.parse(releaseHeaderResponse.body)).toEqual({
      buildId,
      protocolIdentity: manifest.service.protocolIdentity,
      runtimeId: 'runtime-build-release-header'
    });

    const duplicateVersionQueryResponse = await requestHttp({
      url: `${gatewayListen.url}/api?service=skiff.run/sample&version=sample-ios-1.3.7&version=sample-ios-1.3.7`
    });
    expect(duplicateVersionQueryResponse.status).toBe(400);
    expect(JSON.parse(duplicateVersionQueryResponse.body)).toEqual({
      message: 'version query parameter must be singular',
      detail: null
    });

    const headerOverridesQueryResponse = await requestHttp({
      url: `${gatewayListen.url}/api?service=business-query&version=business-version`,
      headers: {
        'X-Skiff-Service': 'skiff.run/sample',
        'X-Skiff-Version': 'sample-ios-1.3.7'
      }
    });
    expect(headerOverridesQueryResponse.status).toBe(200);
    expect(JSON.parse(headerOverridesQueryResponse.body)).toEqual({
      buildId,
      protocolIdentity: manifest.service.protocolIdentity,
      runtimeId: 'runtime-build-release-header'
    });

    const duplicateQueryWithHeadersResponse = await requestHttp({
      url: `${gatewayListen.url}/api?service=business-a&service=business-b&version=business-a&version=business-b`,
      headers: {
        'X-Skiff-Service': 'skiff.run/sample',
        'X-Skiff-Version': 'sample-ios-1.3.7'
      }
    });
    expect(duplicateQueryWithHeadersResponse.status).toBe(200);
    expect(JSON.parse(duplicateQueryWithHeadersResponse.body)).toEqual({
      buildId,
      protocolIdentity: manifest.service.protocolIdentity,
      runtimeId: 'runtime-build-release-header'
    });
  });
});

function versionSnapshot(input: {
  manifest: ReturnType<typeof loadRawHttpManifest>;
  versions: Record<string, string>;
}): RouterActiveSnapshot {
  const firstBuildId = Object.values(input.versions)[0];
  const manifest =
    firstBuildId === undefined
      ? input.manifest
      : {
          ...input.manifest,
          rawHttpEntries: input.manifest.rawHttpEntries.map((entry) => ({
            ...entry,
            buildId: firstBuildId
          }))
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
