import { parseArgs } from 'node:util';

import { loadRouterArtifactRoot } from '../artifacts/loadArtifactRoot.js';
import { loadManifestFiles } from '../manifest/loadManifest.js';
import {
  RouterActiveSnapshotStore,
  snapshotFromArtifacts,
  snapshotFromManifest
} from './activeSnapshot.js';
import { loadRouterConfig, type RouterConfigOverrides } from './config.js';
import {
  RouterControlPlane,
  type ReloadArtifactsOverrides
} from './controlPlane.js';
import { HttpGateway } from './httpGateway.js';
import { RuntimeDispatcher } from './runtimeDispatcher.js';
import { RuntimeEndpoint } from './runtimeEndpoint.js';
import { RuntimeRegistry } from './runtimeRegistry.js';
import { RouterTelemetryProducer } from '../telemetry/producer.js';
import { WebSocketGateway } from '../gateway/webSocketGateway.js';

const args = parseArgs({
  options: {
    config: { type: 'string', default: 'router.yml' },
    'artifact-root': { type: 'string', multiple: true },
    devReload: { type: 'boolean' },
    'dev-reload': { type: 'boolean' },
    host: { type: 'string' },
    'http-body-limit-bytes': { type: 'string' },
    'http-port': { type: 'string' },
    'identity-cli-path': { type: 'string' },
    manifest: { type: 'string' },
    profile: { type: 'string' },
    releaseMode: { type: 'boolean' },
    'release-mode': { type: 'boolean' },
    'runtime-path': { type: 'string' },
    'runtime-port': { type: 'string' },
    'request-timeout-ms': { type: 'string' },
    'websocket-path': { type: 'string' }
  }
});

const overrides: RouterConfigOverrides = {};
if (args.values.host !== undefined) {
  overrides.host = args.values.host;
}
if (args.values['artifact-root'] !== undefined) {
  overrides.artifactRoots = args.values['artifact-root'];
}
const devReloadOverride = args.values.devReload ?? args.values['dev-reload'];
if (devReloadOverride !== undefined) {
  overrides.devReload = devReloadOverride;
}
if (args.values['http-port'] !== undefined) {
  overrides.httpPort = args.values['http-port'];
}
if (args.values['http-body-limit-bytes'] !== undefined) {
  overrides.httpBodyLimitBytes = args.values['http-body-limit-bytes'];
}
if (args.values['identity-cli-path'] !== undefined) {
  overrides.identityCliPath = args.values['identity-cli-path'];
}
if (args.values.manifest !== undefined) {
  overrides.manifest = args.values.manifest;
}
if (args.values.profile !== undefined) {
  overrides.profile = args.values.profile;
}
const releaseModeOverride = args.values.releaseMode ?? args.values['release-mode'];
if (releaseModeOverride !== undefined) {
  overrides.releaseMode = releaseModeOverride;
}
if (args.values['request-timeout-ms'] !== undefined) {
  overrides.requestTimeoutMs = args.values['request-timeout-ms'];
}
if (args.values['runtime-path'] !== undefined) {
  overrides.runtimePath = args.values['runtime-path'];
}
if (args.values['runtime-port'] !== undefined) {
  overrides.runtimePort = args.values['runtime-port'];
}
if (args.values['websocket-path'] !== undefined) {
  overrides.websocketPath = args.values['websocket-path'];
}

const config = await loadRouterConfig(args.values.config, overrides);

const routerArtifacts = config.artifactRoots
  ? await loadRouterArtifactRoot(config.artifactRoots, artifactLoadOptions())
  : undefined;
const initialSnapshot = routerArtifacts
  ? snapshotFromArtifacts(routerArtifacts)
  : snapshotFromManifest(await loadManifestFiles(config.manifests));
const snapshotStore = new RouterActiveSnapshotStore(initialSnapshot);
const registry = new RuntimeRegistry();
const runtimeEndpoint = new RuntimeEndpoint({ registry });
const dispatcher = new RuntimeDispatcher({
  registry,
  frameSender: runtimeEndpoint
});
runtimeEndpoint.setDispatcher(dispatcher);
registry.setServiceVersionIndex(initialSnapshot.versionByService);
registry.setActivationLookup(initialSnapshot.activationByServiceOperation);
const controlPlane = new RouterControlPlane({
  controlBroadcaster: runtimeEndpoint,
  dispatcher,
  registry,
  snapshotStore,
  requestTimeoutMs: config.requestTimeoutMs,
  ...(config.artifactRoots
    ? {
        reloadArtifacts: async (reloadOverrides?: ReloadArtifactsOverrides) =>
          snapshotFromArtifacts(
            await loadRouterArtifactRoot(
              reloadOverrides?.artifactRoots ?? config.artifactRoots!,
              artifactLoadOptions(reloadOverrides)
            )
          )
      }
    : {})
});
const runtimeListenOptions = {
  host: config.host,
  port: config.runtimePort,
  path: config.runtimePath,
  controlPlane
};
const runtimeServer = await runtimeEndpoint.listen(
  initialSnapshot.control
    ? {
        ...runtimeListenOptions,
        control: initialSnapshot.control
      }
    : runtimeListenOptions
);
const telemetryProducer = config.telemetry
  ? new RouterTelemetryProducer(config.telemetry)
  : undefined;
telemetryProducer?.start();

const gateway = new HttpGateway({
  manifest: initialSnapshot.manifest,
  dispatcher,
  ...(initialSnapshot.activationByServiceOperation.size > 0
    ? {
        activationByServiceOperation:
          initialSnapshot.activationByServiceOperation
      }
    : {}),
  snapshotStore,
  host: config.host,
  ...(config.httpBodyLimitBytes !== undefined
    ? { bodyLimitBytes: config.httpBodyLimitBytes }
    : {}),
  port: config.httpPort,
  requestTimeoutMs: config.requestTimeoutMs,
  rewrite: config.rewrite,
  ...(telemetryProducer ? { telemetry: telemetryProducer } : {})
});
const httpServer = await gateway.listen();

const webSocketGateway = initialSnapshot.manifest.websocketEntry
  ? new WebSocketGateway({
      manifest: initialSnapshot.manifest,
      dispatcher,
      runtimeConnectionSend: runtimeEndpoint,
      ...(initialSnapshot.activationByServiceOperation.size > 0
        ? {
            activationByServiceOperation:
              initialSnapshot.activationByServiceOperation
          }
        : {}),
      snapshotStore,
      server: httpServer.server,
      host: config.host,
      path: config.websocketPath,
      requestTimeoutMs: config.requestTimeoutMs,
      rewrite: config.rewrite
    })
  : undefined;
const webSocketServer = webSocketGateway ? await webSocketGateway.listen() : undefined;

controlPlane.setLoopRiskCounterSources({
  httpStream: () => gateway.streamLifecycleCounters(),
  websocketReceive: () =>
    webSocketGateway?.receiveLifecycleCounters() ?? {
      inFlight: 0,
      queued: 0,
      abortOnClose: 0
    }
});

console.log(
  JSON.stringify(
    {
      event: 'router.started',
      http: httpServer.url,
      websocket: webSocketServer?.url,
      runtime: runtimeServer.url,
      control: `http://${runtimeServer.host}:${runtimeServer.port}`,
      artifactRoots: initialSnapshot.control?.artifactRoots,
      devReload: config.devReload,
      releaseMode: config.releaseMode,
      serviceId: initialSnapshot.manifest.service.id,
      websocketReceive: initialSnapshot.manifest.websocketEntry
        ? {
            operation: initialSnapshot.manifest.websocketEntry.receive.operation
          }
        : undefined,
    },
    null,
    2
  )
);

async function shutdown(): Promise<void> {
  await webSocketGateway?.close();
  await gateway.close();
  await telemetryProducer?.shutdown();
  await runtimeEndpoint.close();
}

process.on('SIGINT', () => {
  shutdown()
    .then(() => process.exit(0))
    .catch((error: unknown) => {
      console.error(error);
      process.exit(1);
    });
});

process.on('SIGTERM', () => {
  shutdown()
    .then(() => process.exit(0))
    .catch((error: unknown) => {
      console.error(error);
      process.exit(1);
    });
});

function artifactLoadOptions(overrides?: ReloadArtifactsOverrides) {
  return {
    ...(config.devReload !== undefined ? { devReload: config.devReload } : {}),
    ...(config.identityCliPath !== undefined ? { identityCliPath: config.identityCliPath } : {}),
    ...(config.releaseMode !== undefined ? { releaseMode: config.releaseMode } : {}),
    ...(config.telemetry !== undefined ? { telemetry: config.telemetry } : {}),
    ...(config.fileBackend !== undefined ? { fileBackend: config.fileBackend } : {}),
    ...(overrides?.serviceDb !== undefined
      ? { serviceDb: overrides.serviceDb }
      : config.serviceDb !== undefined
        ? { serviceDb: config.serviceDb }
        : {}),
    configProfile: overrides?.configProfile ?? config.profile
  };
}
