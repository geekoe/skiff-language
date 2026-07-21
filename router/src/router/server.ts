import { parseArgs } from 'node:util';

import { AssemblyWebSocketGateway } from '../gateway/assemblyWebSocketGateway.js';
import { AssemblyActivationCoordinator } from './assemblyActivationCoordinator.js';
import { FileAssemblyActivationStateStore } from './assemblyActivationStateStore.js';
import { AssemblyControlPlane } from './assemblyControlPlane.js';
import { AssemblyHttpGateway } from './assemblyHttpGateway.js';
import { AssemblyRuntimeEndpoint } from './assemblyRuntimeEndpoint.js';
import { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import { loadRouterConfig, type RouterConfigOverrides } from './config.js';
import { RuntimeDispatcher } from './runtimeDispatcher.js';
import {
  FileRuntimeAssemblySnapshotLoader,
  RouterActiveAssemblySnapshotStore
} from './runtimeAssemblySnapshot.js';

const args = parseArgs({
  options: {
    config: { type: 'string', default: 'router.yml' },
    'artifact-root': { type: 'string', multiple: true },
    environment: { type: 'string' },
    host: { type: 'string' },
    'http-body-limit-bytes': { type: 'string' },
    'http-port': { type: 'string' },
    'request-timeout-ms': { type: 'string' },
    'runtime-path': { type: 'string' },
    'runtime-port': { type: 'string' }
  }
});

const overrides: RouterConfigOverrides = {};
if (args.values['artifact-root'] !== undefined) {
  overrides.artifactRoots = args.values['artifact-root'];
}
if (args.values.environment !== undefined) {
  overrides.environment = args.values.environment;
}
if (args.values.host !== undefined) {
  overrides.host = args.values.host;
}
if (args.values['http-body-limit-bytes'] !== undefined) {
  overrides.httpBodyLimitBytes = args.values['http-body-limit-bytes'];
}
if (args.values['http-port'] !== undefined) {
  overrides.httpPort = args.values['http-port'];
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

const config = await loadRouterConfig(args.values.config, overrides);
if (config.environment === undefined) {
  throw new Error('router config environment is required for active RuntimeAssembly routing');
}
if (config.artifactRoots?.length !== 1) {
  throw new Error('router active RuntimeAssembly mode requires exactly one artifactRoot');
}
if (config.rewrite.length > 0) {
  throw new Error('router rewrite-to-service rules are not supported by RuntimeAssembly ingress');
}
const artifactRoot = config.artifactRoots[0]!;
const snapshots = new RouterActiveAssemblySnapshotStore();
const assemblyLoader = new FileRuntimeAssemblySnapshotLoader(artifactRoot);
const stateStore = new FileAssemblyActivationStateStore(artifactRoot);
const registry = new AssemblyRuntimeRegistry(snapshots);
const runtimeEndpoint = new AssemblyRuntimeEndpoint({ registry });
const coordinator = new AssemblyActivationCoordinator({
  environment: config.environment,
  stateStore,
  assemblyLoader,
  snapshots,
  registry,
  controlSender: runtimeEndpoint,
  prepareTimeoutMs: config.requestTimeoutMs
});
runtimeEndpoint.setCoordinator(coordinator);
await coordinator.initialize();

const dispatcher = new RuntimeDispatcher({ registry, frameSender: runtimeEndpoint });
runtimeEndpoint.setDispatcher(dispatcher);
const controlPlane = new AssemblyControlPlane({ coordinator, registry, snapshots });
const runtimeServer = await runtimeEndpoint.listen({
  controlPlane,
  host: config.host,
  port: config.runtimePort,
  path: config.runtimePath
});
const httpGateway = new AssemblyHttpGateway({
  snapshots,
  dispatcher,
  host: config.host,
  port: config.httpPort,
  requestTimeoutMs: config.requestTimeoutMs,
  ...(config.httpBodyLimitBytes !== undefined
    ? { bodyLimitBytes: config.httpBodyLimitBytes }
    : {})
});
const httpServer = await httpGateway.listen();
const webSocketGateway = new AssemblyWebSocketGateway({
  snapshots,
  dispatcher,
  registry,
  runtimeConnectionSend: runtimeEndpoint,
  server: httpServer.server,
  host: config.host,
  requestTimeoutMs: config.requestTimeoutMs
});
const webSocketServer = await webSocketGateway.listen();
const active = snapshots.get();

console.log(
  JSON.stringify(
    {
      event: 'router.started',
      environment: active.environment,
      activeAssembly: active.assembly.assemblyIdentity,
      generation: active.generation,
      http: httpServer.url,
      websocket: webSocketServer.url,
      runtime: runtimeServer.url,
      control: `http://${runtimeServer.host}:${runtimeServer.port}`
    },
    null,
    2
  )
);

async function shutdown(): Promise<void> {
  await webSocketGateway.close();
  await httpGateway.close();
  await runtimeEndpoint.close();
}

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => {
    shutdown()
      .then(() => process.exit(0))
      .catch((error: unknown) => {
        console.error(error);
        process.exit(1);
      });
  });
}
