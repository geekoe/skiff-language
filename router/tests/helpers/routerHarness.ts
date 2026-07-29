import type { LoadedManifest } from '../../src/manifest/types.js';
import type {
  RuntimeCapabilitiesMetadata,
  RuntimeRegisterEnvelope
} from '../../src/protocol/envelope.js';
import {
  RouterActiveSnapshotStore,
  type RouterActiveSnapshot
} from '../../src/router/activeSnapshot.js';
import { RouterControlPlane } from '../../src/router/controlPlane.js';
import {
  HttpGateway,
  type HttpGatewayListenResult,
  type HttpGatewayOptions
} from '../../src/router/httpGateway.js';
import type {
  RuntimeEndpointListenOptions,
  RuntimeEndpointListenResult
} from '../../src/router/runtimeEndpoint.js';
import {
  RuntimeRegistry
} from '../../src/router/runtimeRegistry.js';
import { buildActivationLookup } from '../../src/artifacts/activationLookup.js';
import type { ActivationLookup } from '../../src/artifacts/activationLookup.js';

import {
  DEFAULT_TEST_BUILD_ID,
  loadRawHttpManifest,
  withBuildId
} from './manifests.js';
import { requestHttp } from './request.js';
import {
  MockRuntime,
  createRuntimeRouter,
  trackResource,
  type RuntimeRouter
} from './runtime.js';

export class RouterHarness {
  readonly dispatcher: RuntimeRouter['dispatcher'];
  readonly endpoint: RuntimeRouter['endpoint'];
  readonly registry: RuntimeRegistry;
  registryListen: RuntimeEndpointListenResult | undefined;
  httpGateway: HttpGateway | undefined;
  httpListen: HttpGatewayListenResult | undefined;

  private constructor(
    readonly manifest: LoadedManifest,
    private readonly runtime: RuntimeRouter
  ) {
    trackResource(runtime);
    this.dispatcher = runtime.dispatcher;
    this.endpoint = runtime.endpoint;
    this.registry = runtime.registry;
  }

  static async create(input: {
    manifest: LoadedManifest;
    registryControl?: RuntimeEndpointListenOptions['control'];
  }): Promise<RouterHarness> {
    const harness = new RouterHarness(withBuildId(input.manifest), createRuntimeRouter());
    await harness.listenRegistry({ control: input.registryControl });
    return harness;
  }

  static async rawHttp(input: {
    manifest?: LoadedManifest;
    activationByServiceOperation?: ActivationLookup;
  } = {}): Promise<RouterHarness> {
    const harness = await RouterHarness.create({
      manifest: input.manifest ?? loadRawHttpManifest()
    });
    await harness.listenHttp(
      input.activationByServiceOperation
        ? { activationByServiceOperation: input.activationByServiceOperation }
        : {}
    );
    return harness;
  }

  static async http(input: {
    manifest: LoadedManifest;
    activationByServiceOperation?: ActivationLookup;
  }): Promise<RouterHarness> {
    const harness = await RouterHarness.create({ manifest: input.manifest });
    await harness.listenHttp(
      input.activationByServiceOperation
        ? { activationByServiceOperation: input.activationByServiceOperation }
        : {}
    );
    return harness;
  }

  async listenRegistry(input: {
    control?: RuntimeEndpointListenOptions['control'];
    controlPlane?: RuntimeEndpointListenOptions['controlPlane'];
  } = {}): Promise<RuntimeEndpointListenResult> {
    const options: RuntimeEndpointListenOptions = { port: 0 };
    if (input.control) {
      options.control = input.control;
    }
    options.controlPlane =
      input.controlPlane ??
      new RouterControlPlane({
        controlBroadcaster: this.endpoint,
        dispatcher: this.dispatcher,
        registry: this.registry,
        snapshotStore: new RouterActiveSnapshotStore({
          activationByServiceOperation: buildActivationLookup([]),
          manifest: this.manifest
        })
      });
    this.registryListen = await this.endpoint.listen(options);
    return this.registryListen;
  }

  async listenHttp(input: {
    activationByServiceOperation?: ActivationLookup;
    backpressureDrainTimeoutMs?: HttpGatewayOptions['backpressureDrainTimeoutMs'];
    snapshotStore?: RouterActiveSnapshotStore;
    rewrite?: HttpGatewayOptions['rewrite'];
    telemetry?: HttpGatewayOptions['telemetry'];
  } = {}): Promise<HttpGatewayListenResult> {
    const options: HttpGatewayOptions = {
      manifest: this.manifest,
      dispatcher: this.dispatcher,
      port: 0,
      maxRequestBytes: 64 * 1024 * 1024,
      requestTimeoutMs: 2000
    };
    if (input.activationByServiceOperation) {
      options.activationByServiceOperation = input.activationByServiceOperation;
    }
    if (input.backpressureDrainTimeoutMs) {
      options.backpressureDrainTimeoutMs = input.backpressureDrainTimeoutMs;
    }
    if (input.snapshotStore) {
      options.snapshotStore = input.snapshotStore;
    }
    if (input.rewrite) {
      options.rewrite = input.rewrite;
    }
    if (input.telemetry) {
      options.telemetry = input.telemetry;
    }
    this.httpGateway = trackResource(new HttpGateway(options));
    this.httpListen = await this.httpGateway.listen();
    return this.httpListen;
  }

  async registerRuntime(input: {
    runtimeId: string;
    serviceId?: string;
    version?: string;
    revisionId?: string;
    buildId?: string;
    serviceProtocolIdentity?: string;
    activationIdentity?: string;
    targets?: string[];
    gatewayEntryIdentities?: string[];
    runtimeVersion?: string;
    codeRevisionId?: string;
    artifactIdentity?: string;
    capabilities?: RuntimeCapabilitiesMetadata;
  }): Promise<MockRuntime> {
    if (!this.registryListen) {
      throw new Error('runtime registry is not listening');
    }
    const register: RuntimeRegisterEnvelope = {
      type: 'runtime.register',
      runtimeId: input.runtimeId,
      serviceId: input.serviceId ?? this.manifest.service.id,
      revisionId: input.revisionId ?? this.manifest.service.revisionId,
      buildId: input.buildId ?? DEFAULT_TEST_BUILD_ID,
      serviceProtocolIdentity:
        input.serviceProtocolIdentity ?? this.manifest.service.protocolIdentity,
      targets: input.targets ?? this.manifest.operations.map((operation) => operation.target)
    };
    if (input.version) {
      register.version = input.version;
    }
    if (input.activationIdentity) {
      register.activationIdentity = input.activationIdentity;
    }
    if (input.gatewayEntryIdentities) {
      register.gatewayEntryIdentities = input.gatewayEntryIdentities;
    }
    if (input.runtimeVersion) {
      register.runtimeVersion = input.runtimeVersion;
    }
    if (input.codeRevisionId) {
      register.codeRevisionId = input.codeRevisionId;
    }
    if (input.artifactIdentity) {
      register.artifactIdentity = input.artifactIdentity;
    }
    if (input.capabilities !== undefined) {
      register.capabilities = input.capabilities;
    }
    return await MockRuntime.register(this.registryListen.url, register, this.manifest);
  }

  httpUrl(path: string): string {
    if (!this.httpListen) {
      throw new Error('HTTP gateway is not listening');
    }
    return `${this.httpListen.url}${path}`;
  }

  async requestHttp(input: {
    path: string;
    method?: string;
    headers?: Record<string, string | string[]>;
    body?: string | Buffer;
  }): ReturnType<typeof requestHttp> {
    const request = {
      url: this.httpUrl(input.path)
    } as {
      url: string;
      method?: string;
      headers?: Record<string, string | string[]>;
      body?: string | Buffer;
    };
    if (input.method !== undefined) {
      request.method = input.method;
    }
    if (input.headers !== undefined) {
      request.headers = input.headers;
    }
    if (input.body !== undefined) {
      request.body = input.body;
    }
    return await requestHttp(request);
  }

  createControlPlane(input: {
    reloadArtifacts?: () => Promise<RouterActiveSnapshot>;
    snapshotStore: RouterActiveSnapshotStore;
  }): RouterControlPlane {
    return new RouterControlPlane({
      controlBroadcaster: this.endpoint,
      dispatcher: this.dispatcher,
      registry: this.registry,
      snapshotStore: input.snapshotStore,
      ...(input.reloadArtifacts ? { reloadArtifacts: input.reloadArtifacts } : {})
    });
  }
}
