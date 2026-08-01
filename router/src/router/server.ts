import { parseArgs } from 'node:util';

import { AssemblyActivationCoordinator } from './assemblyActivationCoordinator.js';
import { AssemblyControlPlane } from './assemblyControlPlane.js';
import { AssemblyHttpGateway } from './assemblyHttpGateway.js';
import { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import {
  loadRouterConfig,
  runtimeBootstrapForRouterConfig,
  type RouterConfigOverrides
} from './config.js';
import { FilesystemRuntimeAssemblySnapshotLoader } from './filesystemRuntimeAssemblySnapshotLoader.js';
import { connectMongoAssemblyActivationStateStore } from './mongoAssemblyActivationStateStore.js';
import { RuntimeDispatcher } from './runtimeDispatcher.js';
import { RuntimeEndpoint } from './runtimeEndpoint.js';
import { RuntimeRegistry } from './runtimeRegistry.js';
import { RouterActiveAssemblySnapshotStore } from './runtimeAssemblySnapshot.js';
import { WebSocketGenerationLifecycleRouter } from './webSocketGenerationLifecycleRouter.js';
import { ActorRuntimeDisconnectController } from './actorRuntimeDisconnectController.js';
import { ProductionActorMethodRouter } from './productionActorMethodRouter.js';
import { RuntimeAssemblyActorMethodCatalog } from './runtimeAssemblyActorMethodCatalog.js';
import { ActorOwnerLeaseIdleController } from './actorOwnerLeaseIdleController.js';
import { ActorManager } from '../actor/index.js';
import { ActorSpawnRuntimeControl } from './actorSpawnRuntimeControl.js';
import { ActorGetCreateActivationCoordinator } from './actorGetCreateActivationCoordinator.js';
import { AssemblyWebSocketGateway } from '../gateway/webSocketGateway.js';
import { WebSocketRpcBridge } from '../gateway/webSocketRpcBridge.js';

const args = parseArgs({
  options: {
    config: { type: 'string', default: 'router.yml' },
    'activation-prepare-timeout-ms': { type: 'string' },
    'artifacts-path': { type: 'string' },
    environment: { type: 'string' },
    host: { type: 'string' },
    'http-max-request-bytes': { type: 'string' },
    'http-max-response-bytes': { type: 'string' },
    'http-port': { type: 'string' },
    'request-timeout-ms': { type: 'string' },
    'runtime-path': { type: 'string' },
    'runtime-port': { type: 'string' }
  }
});

const overrides: RouterConfigOverrides = {};
if (args.values['activation-prepare-timeout-ms'] !== undefined) {
  overrides.activationPrepareTimeoutMs =
    args.values['activation-prepare-timeout-ms'];
}
if (args.values['artifacts-path'] !== undefined) {
  overrides.artifactsPath = args.values['artifacts-path'];
}
if (args.values.environment !== undefined) {
  overrides.environment = args.values.environment;
}
if (args.values.host !== undefined) {
  overrides.host = args.values.host;
}
if (args.values['http-max-request-bytes'] !== undefined) {
  overrides.httpMaxRequestBytes = args.values['http-max-request-bytes'];
}
if (args.values['http-max-response-bytes'] !== undefined) {
  overrides.httpMaxResponseBytes = args.values['http-max-response-bytes'];
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
if (config.rewrite.length > 0) {
  throw new Error('router rewrite-to-service rules are not supported by RuntimeAssembly ingress');
}
const snapshots = new RouterActiveAssemblySnapshotStore();
const activation = await connectMongoAssemblyActivationStateStore({
  mongoUrl: config.serviceDb.mongoUrl
});
await activation.store.ensureIndexes();
const assemblyLoader = new FilesystemRuntimeAssemblySnapshotLoader(config.artifactsPath);
const registry = new AssemblyRuntimeRegistry(snapshots);
const actorManager = new ActorManager();
const actorSpawnControl = new ActorSpawnRuntimeControl({
  actorManager,
  actorOwnerLeaseTtlMs: 120_000,
});
const actorGetCreateControl = new ActorGetCreateActivationCoordinator({
  actorManager,
  runtimeDirectory: {
    actorRuntimeCandidates: (serviceId) => registry.actorRuntimeCandidates(serviceId),
    runtimeConnection: (runtimeId) => {
      const ws = registry.connectionForReplica(runtimeId);
      return ws === undefined ? undefined : { runtimeId, ws };
    },
  },
  send: (ws, bytes) => ws.send(bytes),
  ownerLeaseTtlMs: 120_000,
});
const runtimeRegistry = new RuntimeRegistry({
  actorSpawnControl,
  actorGetCreateControl,
});
const actorDisconnect = new ActorRuntimeDisconnectController(
  runtimeRegistry.actorManager()
);
const runtimeEndpoint = new RuntimeEndpoint({
  registry: runtimeRegistry,
  actorGetCreateControl,
  assemblyRegistry: registry,
  bootstrap: () => {
    const active = snapshots.get();
    return runtimeBootstrapForRouterConfig(config, {
      environment: active.environment,
      generation: active.generation,
      assembly: { ...active.assembly },
      configSnapshot: { ...active.configSnapshot }
    });
  },
  actorRuntimeDisconnect: actorDisconnect,
});
const actorCatalog = new RuntimeAssemblyActorMethodCatalog(snapshots);
const actorMethods = new ProductionActorMethodRouter({
    registry: runtimeRegistry,
    actorGetCreateControl,
    runtimeDirectory: {
      actorRuntimeCandidates: (serviceId) =>
        registry.actorRuntimeCandidates(serviceId),
      runtimeConnection: (runtimeId) => {
        const ws = registry.connectionForReplica(runtimeId);
        return ws === undefined ? undefined : { runtimeId, ws };
      },
    },
    disconnectController: actorDisconnect,
    catalog: actorCatalog,
    send: (ws, bytes) => ws.send(bytes),
    ownerLeaseTtlMs: 120_000,
});
runtimeEndpoint.setActorMethods(actorMethods);
let actorIdle: ActorOwnerLeaseIdleController;
actorIdle = new ActorOwnerLeaseIdleController(
  runtimeRegistry.actorManager(),
  {
    sendIdleEviction: async ({ fence }) => {
      await actorMethods.evictIdleOwner(fence);
      await actorIdle.acknowledgeEviction({
        type: 'actor.owner.idle.evict.ack',
        fence,
      });
    },
  },
  { nowMilliseconds: Date.now },
  60_000
);
const actorIdleSweep = setInterval(() => {
  void actorIdle.sweep().catch((error: unknown) => {
    console.error({
      event: 'actor.owner_idle_sweep_error',
      error: error instanceof Error ? error.message : String(error),
    });
  });
}, 5_000);
actorIdleSweep.unref();
const coordinator = new AssemblyActivationCoordinator({
  environment: config.environment,
  stateStore: activation.store,
  assemblyLoader,
  snapshots,
  registry,
  participants: runtimeRegistry,
  controlSender: runtimeEndpoint,
  prepareTimeoutMs: config.activationPrepareTimeoutMs
});
runtimeEndpoint.setCoordinator(coordinator);
await coordinator.initialize();

const dispatcher = new RuntimeDispatcher({
  registry,
  frameSender: runtimeEndpoint,
  maxConcurrency: config.runtimeMaxConcurrency
});
runtimeEndpoint.setDispatcher(dispatcher);
const webSocketRpcBridge = new WebSocketRpcBridge({
  endpoint: runtimeEndpoint,
  dispatcher
});
const generationLifecycle = new WebSocketGenerationLifecycleRouter({
  dispatcher,
  sender: runtimeEndpoint
});
runtimeEndpoint.setWebSocketGenerationLifecycle(generationLifecycle);
registry.setConnectionPinCounter(generationLifecycle);
const controlPlane = new AssemblyControlPlane({
  coordinator,
  dispatcher,
  registry,
  runtimeRegistry,
  snapshots
});
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
  maxRequestBytes: config.httpMaxRequestBytes,
  maxResponseBytes: config.httpMaxResponseBytes
});
const httpServer = await httpGateway.listen();
const webSocketGateway = new AssemblyWebSocketGateway({
  server: httpServer.server,
  snapshots,
  dispatcher,
  rpcBridge: webSocketRpcBridge,
  generationLifecycle,
  runtimeConnectionSend: runtimeEndpoint,
  runtimeOwner: (sender, serviceId) => {
    const replicaId = registry.replicaIdForConnection(sender);
    const replica =
      replicaId === undefined
        ? undefined
        : registry.snapshot().find(
            (candidate) => candidate.replicaId === replicaId
          );
    return replica === undefined
      ? undefined
      : {
          serviceId,
          assemblyIdentity: replica.assemblyIdentity,
          assemblyGeneration: replica.generation,
          replicaId: replica.replicaId
        };
  },
  requestTimeoutMs: config.requestTimeoutMs
});
webSocketGateway.listen();
const active = snapshots.get();

console.log(
  JSON.stringify(
    {
      event: 'router.started',
      environment: active.environment,
      activeAssembly: active.assembly.assemblyIdentity,
      generation: active.generation,
      http: httpServer.url,
      websocket: httpServer.url.replace(/^http:/, 'ws:'),
      runtime: runtimeServer.url,
      control: `http://${runtimeServer.host}:${runtimeServer.port}`
    },
    null,
    2
  )
);

async function shutdown(): Promise<void> {
  clearInterval(actorIdleSweep);
  const failures: unknown[] = [];
  for (const close of [
    () => webSocketGateway.close(),
    () => webSocketRpcBridge.cleanup(),
    () => httpGateway.close(),
    () => runtimeEndpoint.close(),
    () => activation.client.close()
  ]) {
    try {
      await close();
    } catch (error) {
      failures.push(error);
    }
  }
  if (failures.length > 0) {
    throw new AggregateError(failures, 'router shutdown failed');
  }
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
