import WebSocket from 'ws';

import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  type ActorSpawnRuntimeRequestFrameHeader,
  type PackageTestStartFrameHeader,
  type RequestStartFrameHeader,
  type RouterToRuntimeFrameHeader,
  type RuntimeCapabilitiesEnvelope,
  type RuntimeCapabilitiesMetadata,
  type RuntimeRegisterEnvelope
} from '../protocol/envelope.js';
import type {
  ActorExecutionTerminalState,
  ActorKey
} from '../actor/index.js';
import {
  ActorSpawnRuntimeControl,
  type ActorSpawnRuntimeControlOptions,
  type RuntimeControlSource
} from './actorSpawnRuntimeControl.js';
import {
  buildActivationLookup,
  type ActivationLookup
} from '../artifacts/activationLookup.js';
import {
  GatewayError,
  ProviderUnavailableError,
  ServiceProtocolBoundaryError
} from './errors.js';

export interface RuntimeRegistryDependencies extends ActorSpawnRuntimeControlOptions {
  actorSpawnControl?: ActorSpawnRuntimeControl;
  activationByServiceOperation?: ActivationLookup;
}

export type RuntimeRevisionState =
  | 'registered'
  | 'active'
  | 'draining'
  | 'retained'
  | 'retired';

export interface RuntimeSnapshot {
  runtimeId: string;
  serviceId: string;
  version?: string;
  revisionId: string;
  activationIdentity?: string;
  buildId: string;
  serviceProtocolIdentity: string;
  targets: string[];
  revisionState: RuntimeRevisionState;
  active: boolean;
  draining: boolean;
  inFlightCount: number;
  registeredAt: string;
  protocolVersion?: string;
  runtimeVersion?: string;
  codeRevisionId?: string;
  artifactIdentity?: string;
  gatewayEntryIdentities?: string[];
  capabilities?: RuntimeCapabilitiesMetadata;
}

export interface RuntimePruneKeep {
  serviceId: string;
  buildId: string;
}

export interface RuntimePruneOptions {
  keep: readonly RuntimePruneKeep[];
  serviceIds?: readonly string[];
}

export interface RuntimePruneResult {
  deleted: RuntimeSnapshot[];
  kept: RuntimeSnapshot[];
}

export type RuntimeDispatchFrameHeader = RequestStartFrameHeader | PackageTestStartFrameHeader;

export interface RuntimeDispatchConnection {
  runtimeId?: string;
  dispatchBuildId?: string;
  ws: WebSocket;
}

export interface RuntimeInFlightRequest {
  runtimeId?: string;
  request: RuntimeDispatchFrameHeader;
  ws: WebSocket;
}

export interface RuntimeRegistryRuntime {
  runtimeId: string;
  serviceId: string;
  version?: string;
  revisionId: string;
  activationIdentity?: string;
  buildId: string;
  serviceProtocolIdentity: string;
  targets: ReadonlySet<string>;
  revisionState: RuntimeRevisionState;
  registeredAt: Date;
  protocolVersion?: string;
  runtimeVersion?: string;
  codeRevisionId?: string;
  artifactIdentity?: string;
  gatewayEntryIdentities?: ReadonlySet<string>;
  capabilities?: RuntimeCapabilitiesMetadata;
  ws: WebSocket;
}

export interface RuntimeInFlightCounter {
  countInFlight(runtime: RuntimeRegistryRuntime): number;
}

export interface RuntimeConnectionProvider {
  runtimeConnections(): Iterable<WebSocket>;
}

export interface RuntimeActorExecution {
  executionId: string;
  actorKey: ActorKey;
  entryEpoch: number;
  ownerLeaseId: string;
}

export interface RuntimeControlFrameResponse {
  header: RouterToRuntimeFrameHeader;
  payloadBytes: Uint8Array;
}

interface RegisteredRuntime extends RuntimeRegistryRuntime {
  targets: Set<string>;
  gatewayEntryIdentities?: Set<string>;
}

interface RuntimeCapabilityRegistration {
  runtimeId: string;
  capabilities: RuntimeCapabilitiesMetadata;
  registeredAt: Date;
  ws: WebSocket;
}

export class RuntimeRegistry {
  private readonly runtimes = new Map<string, RegisteredRuntime>();
  private readonly runtimeCapabilitiesByConnection = new Map<
    WebSocket,
    RuntimeCapabilityRegistration
  >();
  private readonly activeRevisionByRoute = new Map<string, string>();
  private readonly roundRobinCursorByRoute = new Map<string, number>();
  private readonly actorSpawnControl: ActorSpawnRuntimeControl;
  private connectionProvider: RuntimeConnectionProvider | undefined;
  private inFlightCounter: RuntimeInFlightCounter | undefined;
  // Authoritative version -> current buildId index, derived from on-disk
  // service-version pointer records at artifact load/reload. serviceId ->
  // version -> current buildId. There is exactly one current build per
  // (serviceId, version): the loader rejects a version that resolves to two
  // builds, so this map is the single source of truth for version addressing.
  private serviceVersionBuildIds: ReadonlyMap<
    string,
    ReadonlyMap<string, string>
  > = new Map();
  private activationByServiceOperation: ActivationLookup = buildActivationLookup([]);

  constructor(dependencies: RuntimeRegistryDependencies = {}) {
    const { actorSpawnControl, activationByServiceOperation, ...controlOptions } = dependencies;
    this.actorSpawnControl =
      actorSpawnControl ?? new ActorSpawnRuntimeControl(controlOptions);
    if (activationByServiceOperation !== undefined) {
      this.activationByServiceOperation = activationByServiceOperation;
    }
  }

  setInFlightCounter(counter: RuntimeInFlightCounter | undefined): void {
    this.inFlightCounter = counter;
    this.refreshAllRuntimeStates();
  }

  setRuntimeConnectionProvider(provider: RuntimeConnectionProvider | undefined): void {
    this.connectionProvider = provider;
  }

  /**
   * Install the authoritative version -> current buildId index, derived from
   * the active artifact snapshot's service-version pointer records. Called at
   * startup and on every artifact reload so cross-service addressing always
   * resolves a version to the build that the platform currently publishes.
   */
  setServiceVersionIndex(
    versionByService:
      | ReadonlyMap<string, ReadonlyMap<string, { buildId: string }>>
      | undefined
  ): void {
    if (versionByService === undefined) {
      this.serviceVersionBuildIds = new Map();
      return;
    }
    const next = new Map<string, ReadonlyMap<string, string>>();
    for (const [serviceId, versions] of versionByService) {
      const byVersion = new Map<string, string>();
      for (const [version, binding] of versions) {
        byVersion.set(version, binding.buildId);
      }
      next.set(serviceId, byVersion);
    }
    this.serviceVersionBuildIds = next;
  }

  setActivationLookup(activationByServiceOperation: ActivationLookup | undefined): void {
    this.activationByServiceOperation =
      activationByServiceOperation ?? buildActivationLookup([]);
  }

  snapshot(): RuntimeSnapshot[] {
    return Array.from(this.runtimes.values()).map((runtime) =>
      this.snapshotRuntime(runtime)
    );
  }

  registerRuntime(
    ws: WebSocket,
    envelope: RuntimeRegisterEnvelope
  ): RouterToRuntimeFrameHeader {
    if (!Array.isArray(envelope.targets) || envelope.targets.length === 0) {
      throw new Error('runtime.register.targets must be a non-empty array');
    }

    const runtime: RegisteredRuntime = {
      runtimeId: envelope.runtimeId,
      serviceId: envelope.serviceId,
      ...(envelope.version !== undefined ? { version: envelope.version } : {}),
      revisionId: envelope.revisionId,
      ...(envelope.activationIdentity !== undefined
        ? { activationIdentity: envelope.activationIdentity }
        : {}),
      buildId: envelope.buildId,
      serviceProtocolIdentity: envelope.serviceProtocolIdentity,
      targets: new Set(envelope.targets),
      revisionState: 'registered',
      registeredAt: new Date(),
      ...(envelope.protocolVersion !== undefined
        ? { protocolVersion: envelope.protocolVersion }
        : {}),
      ...(envelope.runtimeVersion !== undefined ? { runtimeVersion: envelope.runtimeVersion } : {}),
      ...(envelope.codeRevisionId !== undefined
        ? { codeRevisionId: envelope.codeRevisionId }
        : {}),
      ...(envelope.artifactIdentity !== undefined
        ? { artifactIdentity: envelope.artifactIdentity }
        : {}),
      ...(Array.isArray(envelope.gatewayEntryIdentities) &&
      envelope.gatewayEntryIdentities.length > 0
        ? { gatewayEntryIdentities: new Set(envelope.gatewayEntryIdentities) }
        : {}),
      ...(envelope.capabilities !== undefined ? { capabilities: envelope.capabilities } : {}),
      ws
    };

    this.runtimes.set(runtime.runtimeId, runtime);
    for (const key of this.runtimeRouteKeys(runtime)) {
      this.activeRevisionByRoute.set(key, runtime.revisionId);
    }
    runtime.revisionState = 'active';
    this.refreshAllRuntimeStates();
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: 'runtime.registered',
      runtimeId: runtime.runtimeId
    };
  }

  registerRuntimeCapabilities(
    ws: WebSocket,
    envelope: RuntimeCapabilitiesEnvelope
  ): void {
    this.runtimeCapabilitiesByConnection.set(ws, {
      runtimeId: envelope.runtimeId,
      capabilities: envelope.capabilities,
      registeredAt: new Date(),
      ws
    });
  }

  pruneRuntimes(options: RuntimePruneOptions): RuntimePruneResult {
    const keep = new Set(
      options.keep.map((entry) => runtimeBuildKey(entry.serviceId, entry.buildId))
    );
    const scopedServiceIds =
      options.serviceIds !== undefined ? new Set(options.serviceIds) : undefined;
    const affectedRouteKeys = new Set<string>();
    const deleted: RuntimeSnapshot[] = [];
    const kept: RuntimeSnapshot[] = [];

    for (const [runtimeId, runtime] of this.runtimes.entries()) {
      const inScope =
        scopedServiceIds === undefined || scopedServiceIds.has(runtime.serviceId);
      const shouldKeep =
        !inScope || keep.has(runtimeBuildKey(runtime.serviceId, runtime.buildId));
      const snapshot = this.snapshotRuntime(runtime);
      if (shouldKeep) {
        kept.push(snapshot);
        continue;
      }

      runtime.revisionState = 'retired';
      for (const key of this.runtimeRouteKeys(runtime)) {
        affectedRouteKeys.add(key);
      }
      this.runtimes.delete(runtimeId);
      deleted.push(snapshot);
    }

    this.pruneActiveRoutes(affectedRouteKeys);
    this.roundRobinCursorByRoute.clear();
    this.refreshAllRuntimeStates();
    return { deleted, kept };
  }

  removeRuntimeConnection(ws: WebSocket): void {
    this.runtimeCapabilitiesByConnection.delete(ws);
    const affectedRouteKeys = new Set<string>();
    for (const [runtimeId, runtime] of this.runtimes.entries()) {
      if (runtime.ws === ws) {
        for (const key of this.runtimeRouteKeys(runtime)) {
          affectedRouteKeys.add(key);
        }
        this.runtimes.delete(runtimeId);
      }
    }

    this.pruneActiveRoutes(affectedRouteKeys);
    this.refreshAllRuntimeStates();
  }

  closeRuntimeConnections(): void {
    for (const runtime of this.runtimes.values()) {
      runtime.revisionState = 'retired';
      runtime.ws.close();
    }
    this.runtimeCapabilitiesByConnection.clear();
    this.activeRevisionByRoute.clear();
    this.roundRobinCursorByRoute.clear();
    this.runtimes.clear();
  }

  registeredConnections(): Set<WebSocket> {
    return new Set(Array.from(this.runtimes.values()).map((runtime) => runtime.ws));
  }

  isConnectionRegisteredForService(ws: WebSocket, serviceId: string): boolean {
    return Array.from(this.runtimes.values()).some(
      (runtime) =>
        runtime.ws === ws &&
        runtime.ws.readyState === WebSocket.OPEN &&
        runtime.serviceId === serviceId
    );
  }

  pickDispatchConnection(
    request: RuntimeDispatchFrameHeader
  ): RuntimeDispatchConnection | null | GatewayError {
    if (request.type === 'package-test.start') {
      return this.pickPackageTestDispatchConnection(request);
    }

    const effectiveBuildId = this.resolveEffectiveBuildId(request);
    if (effectiveBuildId instanceof GatewayError) {
      return effectiveBuildId;
    }

    const activationIdentity = this.resolveActivationIdentity(
      request,
      effectiveBuildId
    );
    const runtime = this.pickRegisteredRuntime(
      request,
      effectiveBuildId,
      activationIdentity
    );
    if (runtime instanceof GatewayError) {
      return runtime;
    }
    if (runtime) {
      return {
        dispatchBuildId: effectiveBuildId,
        runtimeId: runtime.runtimeId,
        ws: runtime.ws
      };
    }

    return this.pickLazyRuntimeConnection(
      request,
      effectiveBuildId,
      activationIdentity
    );
  }

  validateRuntimeRequestStartSource(
    ws: WebSocket,
    request: RequestStartFrameHeader
  ): void {
    if (request.caller.kind !== 'service') {
      throw new Error('runtime-originated request.start requires caller.kind service');
    }

    const registeredForCaller = Array.from(this.runtimes.values()).some(
      (runtime) =>
        runtime.ws === ws &&
        runtime.ws.readyState === WebSocket.OPEN &&
        runtime.revisionState !== 'retired' &&
        runtime.targets.has(request.caller.target)
    );
    if (!registeredForCaller) {
      throw new Error(
        'runtime-originated request.start requires a registered runtime for the caller target'
      );
    }
  }

  async handleActorSpawnRuntimeControlFrame(
    ws: WebSocket,
    header: Parameters<ActorSpawnRuntimeControl['handle']>[0],
    payloadBytes: Uint8Array
  ): Promise<RuntimeControlFrameResponse> {
    const source =
      this.runtimeControlSource(ws, header.runtimeId) ??
      this.packageTestRuntimeControlSource(ws, header);
    if (source === undefined) {
      return {
        header: {
          schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
          type: actorSpawnRuntimeControlErrorType(header.type),
          rpcId: header.rpcId,
          error: {
            code: 'RuntimeNotRegistered',
            message: `runtime control frame requires a registered runtime connection for ${header.runtimeId}`,
            status: 403
          }
        },
        payloadBytes: new Uint8Array()
      };
    }

    const response = await this.actorSpawnControl.handle(header, payloadBytes, source);
    return {
      header: response.header,
      payloadBytes: response.payloadBytes ?? new Uint8Array()
    };
  }

  finishActorExecution(
    actorExecution: RuntimeActorExecution | undefined,
    terminalState: ActorExecutionTerminalState,
    terminalReason?: string
  ): void {
    if (actorExecution === undefined) {
      return;
    }
    void this.actorSpawnControl.actorDispatchManager().finishExecution({
      executionId: actorExecution.executionId,
      actorKey: actorExecution.actorKey,
      entryEpoch: actorExecution.entryEpoch,
      ownerLeaseId: actorExecution.ownerLeaseId,
      terminalState,
      ...(terminalReason === undefined ? {} : { terminalReason }),
      now: this.actorSpawnControl.nowDate()
    });
  }

  refreshRuntimeStatesForRequest(pending: RuntimeInFlightRequest | undefined): void {
    if (!pending) {
      return;
    }
    for (const runtime of this.runtimes.values()) {
      if (this.inFlightRequestBelongsToRuntime(pending, runtime)) {
        this.refreshRuntimeState(runtime);
      }
    }
  }

  refreshAllRuntimeStates(): void {
    for (const runtime of this.runtimes.values()) {
      this.refreshRuntimeState(runtime);
    }
  }

  private resolveCurrentBuildId(
    serviceId: string,
    version: string
  ): string | undefined {
    return this.serviceVersionBuildIds.get(serviceId)?.get(version);
  }

  private resolveEffectiveBuildId(
    request: RequestStartFrameHeader
  ): string | GatewayError {
    // Addressing key. When the caller carries a published version (service-to-
    // service calls), resolve the current build for (serviceId, version) from
    // the authoritative pointer index and address by that build. The request's
    // own buildId is the caller's frozen, publish-time expectation and is NOT
    // the selector; it is demoted to a boundary-compatibility witness.
    //
    // Two version cases are deliberately distinct:
    //   - The service has authoritative version records but not this version:
    //     the version is genuinely unpublished -> unavailable (never fall
    //     through to an unindexed build).
    //   - The service has no artifact/version authority to consult, so address
    //     by the caller's frozen buildId.
    if (
      request.version !== undefined &&
      request.serviceId !== undefined &&
      this.serviceVersionBuildIds.has(request.serviceId)
    ) {
      const resolved = this.resolveCurrentBuildId(
        request.serviceId,
        request.version
      );
      if (resolved === undefined) {
        return new ProviderUnavailableError(
          `No published build is registered for ${request.serviceId} version ${request.version}`
        );
      }
      return resolved;
    }

    return request.buildId;
  }

  private runtimeControlSource(
    ws: WebSocket,
    runtimeId: string
  ): RuntimeControlSource | undefined {
    const runtime = this.runtimes.get(runtimeId);
    if (
      runtime === undefined ||
      runtime.ws !== ws ||
      runtime.ws.readyState !== WebSocket.OPEN ||
      runtime.revisionState === 'retired'
    ) {
      return undefined;
    }
    return {
      runtimeId: runtime.runtimeId,
      serviceId: runtime.serviceId,
      buildId: runtime.buildId,
      serviceProtocolIdentity: runtime.serviceProtocolIdentity,
      targets: runtime.targets,
      inFlightCount: this.countInFlight(runtime),
      ...(runtime.activationIdentity === undefined
        ? {}
        : { activationIdentity: runtime.activationIdentity })
    };
  }

  private packageTestRuntimeControlSource(
    ws: WebSocket,
    header: ActorSpawnRuntimeRequestFrameHeader
  ): RuntimeControlSource | undefined {
    const capability = this.runtimeCapabilitiesByConnection.get(ws);
    if (
      capability === undefined ||
      capability.runtimeId !== header.runtimeId ||
      capability.ws !== ws ||
      capability.ws.readyState !== WebSocket.OPEN ||
      !runtimeSupportsPackageTestDispatch(capability)
    ) {
      return undefined;
    }

    switch (header.type) {
      case 'spawn.submit.request':
        return {
          runtimeId: header.runtimeId,
          serviceId: header.serviceId,
          buildId: header.buildId ?? packageTestRuntimeControlBuildId(header.runtimeId),
          serviceProtocolIdentity: header.serviceProtocolIdentity,
          targets: new Set([header.target]),
          inFlightCount: 0,
          ...(header.activationIdentity === undefined
            ? {}
            : { activationIdentity: header.activationIdentity })
        };
      case 'spawn.claim.request':
        return {
          runtimeId: header.runtimeId,
          serviceId: header.serviceId,
          buildId: header.buildId ?? packageTestRuntimeControlBuildId(header.runtimeId),
          serviceProtocolIdentity: header.serviceProtocolIdentity,
          targets: new Set(header.supportedTargets),
          inFlightCount: 0
        };
      case 'spawn.renew.request':
      case 'spawn.complete.request':
      case 'spawn.fail.request':
        return {
          runtimeId: header.runtimeId,
          serviceId: '__skiff.package-test',
          buildId: packageTestRuntimeControlBuildId(header.runtimeId),
          serviceProtocolIdentity: '__skiff.package-test',
          targets: new Set(),
          inFlightCount: 0
        };
      default:
        return undefined;
    }
  }

  private pickPackageTestDispatchConnection(
    request: PackageTestStartFrameHeader
  ): RuntimeDispatchConnection | ProviderUnavailableError {
    const candidates: RuntimeDispatchConnection[] = [];
    const candidateConnections = new Set<WebSocket>();

    for (const runtime of this.runtimes.values()) {
      if (
        runtime.ws.readyState === WebSocket.OPEN &&
        runtime.revisionState !== 'retired' &&
        runtimeSupportsPackageTestDispatch(runtime)
      ) {
        candidates.push({
          runtimeId: runtime.runtimeId,
          ws: runtime.ws
        });
        candidateConnections.add(runtime.ws);
      }
    }

    for (const runtime of this.runtimeCapabilitiesByConnection.values()) {
      if (
        runtime.ws.readyState === WebSocket.OPEN &&
        !candidateConnections.has(runtime.ws) &&
        runtimeSupportsPackageTestDispatch(runtime)
      ) {
        candidates.push({
          runtimeId: runtime.runtimeId,
          ws: runtime.ws
        });
        candidateConnections.add(runtime.ws);
      }
    }

    if (candidates.length === 0) {
      return new ProviderUnavailableError(
        'No runtime with packageTestDispatch capability is registered for package test dispatch'
      );
    }

    const cursorKey = packageTestSelectionCursorKey(
      request.packageId,
      request.testBuildIdentity
    );
    const cursor = this.roundRobinCursorByRoute.get(cursorKey) ?? 0;
    const connection = candidates[cursor % candidates.length];
    if (!connection) {
      return new ProviderUnavailableError(
        'No runtime with packageTestDispatch capability is registered for package test dispatch'
      );
    }
    this.roundRobinCursorByRoute.set(cursorKey, cursor + 1);
    return connection;
  }

  private pickRegisteredRuntime(
    request: RequestStartFrameHeader,
    effectiveBuildId: string,
    activationIdentity: string | undefined
  ): RegisteredRuntime | null | GatewayError {
    if (request.gatewayEntryIdentity) {
      return this.pickRuntimeForRoute({
        request,
        effectiveBuildId,
        gatewayEntryIdentity: request.gatewayEntryIdentity,
        activationIdentity
      });
    }

    return this.pickRuntimeForRoute({
      request,
      effectiveBuildId,
      gatewayEntryIdentity: undefined,
      activationIdentity
    });
  }

  private pickRuntimeForRoute(input: {
    request: RequestStartFrameHeader;
    effectiveBuildId: string;
    gatewayEntryIdentity: string | undefined;
    activationIdentity: string | undefined;
  }): RegisteredRuntime | null | GatewayError {
    let candidates = Array.from(this.runtimes.values()).filter((runtime) => {
      if (
        runtime.ws.readyState !== WebSocket.OPEN ||
        runtime.revisionState === 'retired' ||
        !runtime.targets.has(input.request.target)
      ) {
        return false;
      }

      if (
        input.request.serviceId !== undefined &&
        runtime.serviceId !== input.request.serviceId
      ) {
        return false;
      }

      if (runtime.buildId !== input.effectiveBuildId) {
        return false;
      }

      const hasGatewayEntryIdentityIndex =
        (runtime.gatewayEntryIdentities?.size ?? 0) > 0;
      const routeGatewayEntryIdentity =
        input.gatewayEntryIdentity !== undefined && hasGatewayEntryIdentityIndex
          ? input.gatewayEntryIdentity
          : undefined;
      if (routeGatewayEntryIdentity !== undefined) {
        if (!runtime.gatewayEntryIdentities?.has(routeGatewayEntryIdentity)) {
          return false;
        }
      }

      const key = runtimeRouteKey({
        buildId: input.effectiveBuildId,
        serviceId: runtime.serviceId,
        serviceProtocolIdentity: runtime.serviceProtocolIdentity,
        target: input.request.target,
        gatewayEntryIdentity: routeGatewayEntryIdentity
      });
      return this.activeRevisionByRoute.get(key) === runtime.revisionId;
    });

    // Boundary compatibility check. Protocol identity is no longer a selector;
    // it is a witness. After version addressing has chosen the current build,
    // verify that build's protocol identity satisfies the caller's frozen,
    // publish-time expectation. A mismatch means the published version moved to
    // an incompatible boundary since the caller was built: fail loudly rather
    // than route to an incompatible build.
    const boundaryMismatch = candidates.find(
      (runtime) =>
        runtime.serviceProtocolIdentity !==
        input.request.serviceProtocolIdentity
    );
    if (boundaryMismatch !== undefined) {
      const coordinate =
        input.request.version !== undefined
          ? `${input.request.serviceId ?? '<service>'} version ${input.request.version}`
          : `build ${input.effectiveBuildId}`;
      return new ServiceProtocolBoundaryError(
        `Current build for ${coordinate} has protocol identity ${boundaryMismatch.serviceProtocolIdentity} which does not satisfy caller expectation ${input.request.serviceProtocolIdentity}`,
        {
          serviceId: input.request.serviceId,
          version: input.request.version,
          expectedProtocolIdentity: input.request.serviceProtocolIdentity,
          resolvedProtocolIdentity: boundaryMismatch.serviceProtocolIdentity,
          resolvedBuildId: input.effectiveBuildId
        }
      );
    }

    if (input.activationIdentity !== undefined) {
      candidates = candidates.filter(
        (runtime) => runtime.activationIdentity === input.activationIdentity
      );
    } else {
      const activationContexts = new Set(
        candidates.map((runtime) => runtime.activationIdentity)
      );
      if (activationContexts.size > 1) {
        return new ProviderUnavailableError(
          'Multiple runtime activations match request; activationIdentity is required'
        );
      }
    }

    if (candidates.length === 0) {
      return null;
    }

    const cursorKey = selectionCursorKey(
      input.request.serviceId,
      input.effectiveBuildId,
      input.request.serviceProtocolIdentity,
      input.request.target,
      input.gatewayEntryIdentity,
      input.activationIdentity
    );
    const cursor = this.roundRobinCursorByRoute.get(cursorKey) ?? 0;
    const runtime = candidates[cursor % candidates.length];
    if (!runtime) {
      return null;
    }
    this.roundRobinCursorByRoute.set(cursorKey, cursor + 1);
    return runtime;
  }

  private resolveActivationIdentity(
    request: RequestStartFrameHeader,
    effectiveBuildId: string
  ): string | undefined {
    if (request.activationIdentity !== undefined) {
      return request.activationIdentity;
    }
    if (request.serviceId === undefined) {
      return undefined;
    }
    return this.activationByServiceOperation.get({
      serviceId: request.serviceId,
      target: request.target,
      buildId: effectiveBuildId
    });
  }

  private pickLazyRuntimeConnection(
    request: RequestStartFrameHeader,
    effectiveBuildId: string,
    activationIdentity: string | undefined
  ): RuntimeDispatchConnection | null {
    if (request.serviceId === undefined) {
      return null;
    }

    const clients = this.registeredAndLazyClients().filter(
      (ws) =>
        ws.readyState === WebSocket.OPEN &&
        !this.hasRegisteredTargetBuildOnConnection(
          ws,
          request,
          effectiveBuildId,
          activationIdentity
        )
    );
    if (clients.length === 0) {
      return null;
    }

    const cursorKey = lazySelectionCursorKey(
      request.serviceId,
      effectiveBuildId,
      request.serviceProtocolIdentity,
      request.target,
      request.gatewayEntryIdentity,
      activationIdentity
    );
    const cursor = this.roundRobinCursorByRoute.get(cursorKey) ?? 0;
    const ws = clients[cursor % clients.length];
    if (!ws) {
      return null;
    }
    this.roundRobinCursorByRoute.set(cursorKey, cursor + 1);
    return { dispatchBuildId: effectiveBuildId, ws };
  }

  private registeredAndLazyClients(): WebSocket[] {
    const clients = new Set<WebSocket>();
    for (const client of this.connectionProvider?.runtimeConnections() ?? []) {
      clients.add(client);
    }
    for (const runtime of this.runtimes.values()) {
      clients.add(runtime.ws);
    }
    for (const capability of this.runtimeCapabilitiesByConnection.values()) {
      clients.add(capability.ws);
    }
    return Array.from(clients);
  }

  private hasRegisteredTargetBuildOnConnection(
    ws: WebSocket,
    request: RequestStartFrameHeader,
    effectiveBuildId: string,
    activationIdentity: string | undefined
  ): boolean {
    return Array.from(this.runtimes.values()).some(
      (runtime) =>
        runtime.ws === ws &&
        runtime.ws.readyState === WebSocket.OPEN &&
        runtime.revisionState !== 'retired' &&
        runtime.serviceId === request.serviceId &&
        runtime.buildId === effectiveBuildId &&
        runtime.serviceProtocolIdentity === request.serviceProtocolIdentity &&
        runtime.targets.has(request.target) &&
        this.runtimeAcceptsGatewayEntry(runtime, request.gatewayEntryIdentity) &&
        (activationIdentity === undefined ||
          runtime.activationIdentity === activationIdentity)
    );
  }

  private refreshRuntimeState(runtime: RegisteredRuntime): void {
    if (runtime.revisionState === 'retired') {
      return;
    }

    if (this.hasActiveTarget(runtime)) {
      runtime.revisionState = 'active';
      return;
    }

    runtime.revisionState =
      this.countInFlight(runtime) > 0 ? 'draining' : 'retained';
  }

  private hasActiveTarget(runtime: RegisteredRuntime): boolean {
    for (const key of this.runtimeRouteKeys(runtime)) {
      if (this.activeRevisionByRoute.get(key) === runtime.revisionId) {
        return true;
      }
    }
    return false;
  }

  private runtimeRouteKeys(runtime: RegisteredRuntime): Set<string> {
    const keys = new Set<string>();
    for (const target of runtime.targets) {
      keys.add(
        runtimeRouteKey({
          buildId: runtime.buildId,
          serviceId: runtime.serviceId,
          serviceProtocolIdentity: runtime.serviceProtocolIdentity,
          target,
          gatewayEntryIdentity: undefined
        })
      );
      for (const gatewayEntryIdentity of runtime.gatewayEntryIdentities ?? []) {
        keys.add(
          runtimeRouteKey({
            buildId: runtime.buildId,
            serviceId: runtime.serviceId,
            serviceProtocolIdentity: runtime.serviceProtocolIdentity,
            target,
            gatewayEntryIdentity
          })
        );
      }
    }
    return keys;
  }

  private pruneActiveRoutes(routeKeys: Set<string>): void {
    for (const key of routeKeys) {
      const activeRevisionId = this.activeRevisionByRoute.get(key);
      if (!activeRevisionId) {
        continue;
      }
      const hasLiveActiveRevision = Array.from(this.runtimes.values()).some(
        (runtime) =>
          runtime.revisionId === activeRevisionId && this.runtimeRouteKeys(runtime).has(key)
      );
      if (!hasLiveActiveRevision) {
        this.activeRevisionByRoute.delete(key);
      }
    }
  }

  private countInFlight(runtime: RegisteredRuntime): number {
    return this.inFlightCounter?.countInFlight(runtime) ?? 0;
  }

  private inFlightRequestBelongsToRuntime(
    pending: RuntimeInFlightRequest,
    runtime: RegisteredRuntime
  ): boolean {
    if (pending.runtimeId !== undefined) {
      return pending.runtimeId === runtime.runtimeId;
    }
    if (pending.ws !== runtime.ws) {
      return false;
    }
    const request = pending.request;
    if (request.type === 'package-test.start') {
      return false;
    }
    if (request.serviceId !== undefined && request.serviceId !== runtime.serviceId) {
      return false;
    }
    return (
      runtime.buildId === request.buildId &&
      runtime.serviceProtocolIdentity === request.serviceProtocolIdentity &&
      runtime.targets.has(request.target) &&
      this.runtimeAcceptsGatewayEntry(runtime, request.gatewayEntryIdentity) &&
      (request.activationIdentity === undefined ||
        runtime.activationIdentity === request.activationIdentity)
    );
  }

  private runtimeAcceptsGatewayEntry(
    runtime: RegisteredRuntime,
    gatewayEntryIdentity: string | undefined
  ): boolean {
    const hasGatewayEntryIdentityIndex = (runtime.gatewayEntryIdentities?.size ?? 0) > 0;
    return (
      gatewayEntryIdentity === undefined ||
      !hasGatewayEntryIdentityIndex ||
      runtime.gatewayEntryIdentities?.has(gatewayEntryIdentity) === true
    );
  }

  private snapshotRuntime(runtime: RegisteredRuntime): RuntimeSnapshot {
    const inFlightCount = this.countInFlight(runtime);
    const active = this.hasActiveTarget(runtime);
    const snapshot: RuntimeSnapshot = {
      runtimeId: runtime.runtimeId,
      serviceId: runtime.serviceId,
      revisionId: runtime.revisionId,
      buildId: runtime.buildId,
      serviceProtocolIdentity: runtime.serviceProtocolIdentity,
      targets: Array.from(runtime.targets),
      revisionState: runtime.revisionState,
      active,
      draining: runtime.revisionState === 'draining',
      inFlightCount,
      registeredAt: runtime.registeredAt.toISOString()
    };
    if (runtime.version !== undefined) {
      snapshot.version = runtime.version;
    }
    if (runtime.protocolVersion !== undefined) {
      snapshot.protocolVersion = runtime.protocolVersion;
    }
    if (runtime.runtimeVersion !== undefined) {
      snapshot.runtimeVersion = runtime.runtimeVersion;
    }
    if (runtime.codeRevisionId !== undefined) {
      snapshot.codeRevisionId = runtime.codeRevisionId;
    }
    if (runtime.activationIdentity !== undefined) {
      snapshot.activationIdentity = runtime.activationIdentity;
    }
    if (runtime.artifactIdentity !== undefined) {
      snapshot.artifactIdentity = runtime.artifactIdentity;
    }
    if (runtime.gatewayEntryIdentities !== undefined) {
      snapshot.gatewayEntryIdentities = Array.from(runtime.gatewayEntryIdentities);
    }
    if (runtime.capabilities !== undefined) {
      snapshot.capabilities = runtime.capabilities;
    }
    return snapshot;
  }
}

function runtimeBuildKey(serviceId: string, buildId: string): string {
  return `${serviceId}\u0000${buildId}`;
}

function runtimeRouteKey(input: {
  buildId: string;
  serviceId: string;
  serviceProtocolIdentity: string;
  target: string;
  gatewayEntryIdentity: string | undefined;
}): string {
  return [
    input.serviceId,
    input.buildId,
    input.serviceProtocolIdentity,
    input.target,
    input.gatewayEntryIdentity ?? ''
  ].join('\u0000');
}

function selectionCursorKey(
  serviceId: string | undefined,
  buildId: string,
  serviceProtocolIdentity: string,
  target: string,
  gatewayEntryIdentity: string | undefined,
  activationIdentity: string | undefined
): string {
  return [
    serviceId ?? '',
    buildId,
    serviceProtocolIdentity,
    target,
    gatewayEntryIdentity ?? '',
    activationIdentity ?? ''
  ].join('\u0000');
}

function lazySelectionCursorKey(
  serviceId: string,
  buildId: string,
  serviceProtocolIdentity: string,
  target: string,
  gatewayEntryIdentity: string | undefined,
  activationIdentity: string | undefined
): string {
  return [
    'lazy',
    serviceId,
    buildId,
    serviceProtocolIdentity,
    target,
    gatewayEntryIdentity ?? '',
    activationIdentity ?? ''
  ].join('\u0000');
}

function packageTestSelectionCursorKey(
  packageId: string,
  testBuildIdentity: string
): string {
  return ['package-test', packageId, testBuildIdentity].join('\u0000');
}

function runtimeSupportsPackageTestDispatch(runtime: {
  capabilities?: RuntimeCapabilitiesMetadata;
}): boolean {
  return runtime.capabilities?.packageTestDispatch === true;
}

function packageTestRuntimeControlBuildId(runtimeId: string): string {
  return `skiff-package-test-runtime-control:${runtimeId}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function actorSpawnRuntimeControlErrorType(
  requestType: Parameters<ActorSpawnRuntimeControl['handle']>[0]['type']
):
  | 'actor.put.error'
  | 'actor.find.error'
  | 'actor.remove.error'
  | 'spawn.submit.error'
  | 'spawn.claim.error'
  | 'spawn.renew.error'
  | 'spawn.complete.error'
  | 'spawn.fail.error' {
  switch (requestType) {
    case 'actor.put.request':
      return 'actor.put.error';
    case 'actor.find.request':
      return 'actor.find.error';
    case 'actor.remove.request':
      return 'actor.remove.error';
    case 'spawn.submit.request':
      return 'spawn.submit.error';
    case 'spawn.claim.request':
      return 'spawn.claim.error';
    case 'spawn.renew.request':
      return 'spawn.renew.error';
    case 'spawn.complete.request':
      return 'spawn.complete.error';
    case 'spawn.fail.request':
      return 'spawn.fail.error';
  }
}
