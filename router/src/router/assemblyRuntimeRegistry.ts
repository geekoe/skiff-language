import WebSocket from 'ws';

import type {
  AssemblyActivationControl
} from '../protocol/assemblyActivationProtocol.js';
import type {
  ActorSpawnRuntimeRequestFrameHeader,
  RuntimeHealthCounters
} from '../protocol/envelope.js';
import type {
  RuntimeAssemblyRequestStartFrameHeader,
  RuntimeAssemblyRequestStartFrameWireHeader
} from '../protocol/runtimeAssemblyRequest.js';
import {
  validateRuntimeAssemblyRequestStartFrameWireHeader
} from '../protocol/runtimeProtocol.js';
import { ProviderUnavailableError, ServiceProtocolBoundaryError } from './errors.js';
import { isRuntimeAssemblyRequestDispatchHeader } from './runtimeRegistry.js';
import type {
  RuntimeDispatchConnection,
  RuntimeDispatchFrameHeader,
  RuntimeDispatchRuntimeIdentity,
  RuntimeInFlightCounter,
  RuntimeInFlightRequest
} from './runtimeRegistry.js';
import type { RuntimeControlSource } from './actorSpawnRuntimeControl.js';
import {
  canonicalHttpHost,
  RouterActiveAssemblySnapshotStore,
  runtimeAssemblyIngressKey,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblyIngressBinding
} from './runtimeAssemblySnapshot.js';

type AssemblyRegisterControl = Extract<AssemblyActivationControl, { type: 'register' }>;

export type AssemblyReplicaState = 'healthy' | 'draining' | 'disconnected';

interface AssemblyReplica extends RuntimeDispatchRuntimeIdentity {
  environment: string;
  generation: number;
  assemblyIdentity: string;
  replicaId: string;
  state: AssemblyReplicaState;
  registeredAt: string;
  lastHealthAt?: string;
  healthCounters?: RuntimeHealthCounters;
  deploymentBindingsByService: ReadonlyMap<string, AssemblyDeploymentBinding>;
}

interface AssemblyDeploymentBinding {
  deploymentRevision: string;
  packageBuildId: string;
  serviceProtocolIdentity: string;
  maxConcurrency: number;
  timeoutMs?: number;
}

export interface AssemblyReplicaSnapshot {
  replicaId: string;
  environment: string;
  generation: number;
  assemblyIdentity: string;
  state: AssemblyReplicaState;
  connected: boolean;
  inFlightCount: number;
  connectionPinCount: number;
  connectionReleaseAckCount: number;
  registeredAt: string;
  lastHealthAt?: string;
  healthCounters?: RuntimeHealthCounters;
}

export class AssemblyRuntimeRegistry {
  private readonly replicas = new Map<string, AssemblyReplica>();
  private readonly replicaIdByConnection = new Map<WebSocket, string>();
  private inFlightCounter: RuntimeInFlightCounter | undefined;
  private connectionPinCounter:
    | {
        connectionPinCount(ws: WebSocket): number;
        connectionReleaseAckCount(ws: WebSocket): number;
      }
    | undefined;
  private nextReplicaCursor = 0;

  constructor(private readonly snapshots: RouterActiveAssemblySnapshotStore) {}

  setInFlightCounter(counter: RuntimeInFlightCounter | undefined): void {
    this.inFlightCounter = counter;
  }

  setConnectionPinCounter(
    counter:
      | {
          connectionPinCount(ws: WebSocket): number;
          connectionReleaseAckCount(ws: WebSocket): number;
        }
      | undefined
  ): void {
    this.connectionPinCounter = counter;
  }

  register(ws: WebSocket, control: AssemblyRegisterControl): void {
    const active = this.snapshots.get();
    if (!matchesActiveSnapshot(control, active)) {
      throw new Error(
        `stale assembly registration ${control.replicaId} does not match committed generation`
      );
    }
    const connectionReplicaId = this.replicaIdByConnection.get(ws);
    if (connectionReplicaId !== undefined && connectionReplicaId !== control.replicaId) {
      throw new Error('runtime connection cannot change replica identity');
    }
    const existing = this.replicas.get(control.replicaId);
    if (existing !== undefined && existing.ws !== ws) {
      existing.state = 'disconnected';
      this.replicaIdByConnection.delete(existing.ws);
      existing.ws.close(1008, 'replica identity re-registered on a new connection');
    }
    this.replicas.set(control.replicaId, {
      replicaId: control.replicaId,
      runtimeId: control.replicaId,
      environment: control.environment,
      generation: control.generation,
      assemblyIdentity: control.assembly.assemblyIdentity,
      state: 'healthy',
      registeredAt: new Date().toISOString(),
      deploymentBindingsByService: deploymentBindingsByService(active),
      ws
    });
    this.replicaIdByConnection.set(ws, control.replicaId);
  }

  recordHealth(
    ws: WebSocket,
    replicaId: string,
    observedAt: string,
    counters: RuntimeHealthCounters
  ): void {
    const replica = this.registeredReplicaForConnection(ws, replicaId);
    replica.lastHealthAt = observedAt;
    replica.healthCounters = { ...counters };
  }

  activate(snapshot: RouterActiveAssemblySnapshot): void {
    for (const replica of this.replicas.values()) {
      replica.state = matchesActiveSnapshot(replica, snapshot) ? 'healthy' : 'draining';
    }
    this.nextReplicaCursor = 0;
  }

  healthyParticipantReplicaIds(): readonly string[] {
    return this.dispatchCandidates()
      .map((replica) => replica.replicaId)
      .sort((left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right)));
  }

  connectedParticipantReplicaIds(replicaIds: readonly string[]): readonly string[] {
    return replicaIds.filter((replicaId) => this.isReplicaConnected(replicaId));
  }

  isReplicaConnected(replicaId: string): boolean {
    const replica = this.replicas.get(replicaId);
    return (
      replica !== undefined &&
      replica.state !== 'disconnected' &&
      replica.ws.readyState === WebSocket.OPEN
    );
  }

  connectionForReplica(replicaId: string): WebSocket | undefined {
    const replica = this.replicas.get(replicaId);
    return replica !== undefined && replica.ws.readyState === WebSocket.OPEN
      ? replica.ws
      : undefined;
  }

  actorRuntimeCandidates(serviceId: string): RuntimeDispatchRuntimeIdentity[] {
    return this.dispatchCandidates()
      .filter((replica) => replica.deploymentBindingsByService.has(serviceId))
      .map((replica) => ({
        runtimeId: replica.runtimeId,
        ws: replica.ws
      }))
      .sort((left, right) =>
        Buffer.compare(Buffer.from(left.runtimeId), Buffer.from(right.runtimeId))
      );
  }

  replicaIdForConnection(ws: WebSocket): string | undefined {
    return this.replicaIdByConnection.get(ws);
  }

  assertReplicaConnection(ws: WebSocket, replicaId: string): void {
    this.registeredReplicaForConnection(ws, replicaId);
  }

  actorSpawnRuntimeControlSource(
    ws: WebSocket,
    header: ActorSpawnRuntimeRequestFrameHeader
  ): RuntimeControlSource | undefined {
    const replicaId = this.replicaIdByConnection.get(ws);
    const replica =
      replicaId === undefined ? undefined : this.replicas.get(replicaId);
    if (
      replica === undefined ||
      replica.ws !== ws ||
      replica.ws.readyState !== WebSocket.OPEN ||
      replica.state === 'disconnected' ||
      header.runtimeId !== replica.replicaId ||
      header.activationIdentity.runtimeReplicaId !== replica.replicaId ||
      header.activationIdentity.assemblyIdentity !== replica.assemblyIdentity ||
      header.activationIdentity.generation !== replica.generation ||
      !this.replicaCanUseActivation(replica)
    ) {
      return undefined;
    }

    const serviceId = actorSpawnServiceId(header);
    const binding = replica.deploymentBindingsByService.get(serviceId);
    if (
      binding === undefined ||
      binding.deploymentRevision !==
        header.activationIdentity.deploymentRevision ||
      (isSpawnControl(header) &&
        binding.serviceProtocolIdentity !== header.serviceProtocolIdentity)
    ) {
      return undefined;
    }

    return {
      runtimeId: replica.replicaId,
      serviceId,
      buildId: binding.packageBuildId,
      serviceProtocolIdentity: binding.serviceProtocolIdentity,
      inFlightCount:
        this.inFlightCounter?.countDeploymentInFlight?.(
          replica,
          serviceId,
          binding.deploymentRevision
        ) ?? this.countInFlight(replica),
      maxConcurrency: binding.maxConcurrency,
      ...(binding.timeoutMs === undefined ? {} : { timeoutMs: binding.timeoutMs }),
      activationIdentity: { ...header.activationIdentity }
    };
  }

  removeRuntimeConnection(ws: WebSocket): string | undefined {
    const replicaId = this.replicaIdByConnection.get(ws);
    if (replicaId === undefined) {
      return undefined;
    }
    this.replicaIdByConnection.delete(ws);
    const replica = this.replicas.get(replicaId);
    if (replica !== undefined && replica.ws === ws) {
      replica.state = 'disconnected';
    }
    return replicaId;
  }

  closeRuntimeConnections(): void {
    for (const replica of this.replicas.values()) {
      replica.state = 'disconnected';
      replica.ws.close();
    }
    this.replicaIdByConnection.clear();
  }

  registeredConnections(): Set<WebSocket> {
    return new Set(
      Array.from(this.replicas.values())
        .filter((replica) => replica.state !== 'disconnected')
        .map((replica) => replica.ws)
    );
  }

  snapshot(): AssemblyReplicaSnapshot[] {
    return Array.from(this.replicas.values()).map((replica) => ({
      replicaId: replica.replicaId,
      environment: replica.environment,
      generation: replica.generation,
      assemblyIdentity: replica.assemblyIdentity,
      state: replica.state,
      connected: replica.ws.readyState === WebSocket.OPEN,
      inFlightCount: this.countInFlight(replica),
      connectionPinCount:
        this.connectionPinCounter?.connectionPinCount(replica.ws) ?? 0,
      connectionReleaseAckCount:
        this.connectionPinCounter?.connectionReleaseAckCount(replica.ws) ?? 0,
      registeredAt: replica.registeredAt,
      ...(replica.lastHealthAt !== undefined ? { lastHealthAt: replica.lastHealthAt } : {}),
      ...(replica.healthCounters !== undefined
        ? { healthCounters: { ...replica.healthCounters } }
        : {})
    }));
  }

  pickDispatchConnection(
    request: RuntimeDispatchFrameHeader
  ): RuntimeDispatchConnection | ProviderUnavailableError | ServiceProtocolBoundaryError {
    const requestError = this.validateDispatchRequest(request);
    if (requestError !== undefined) {
      return requestError;
    }
    return this.pickHealthyDispatchConnection();
  }

  pickAssemblyTestDispatchConnection(
    request: RuntimeDispatchFrameHeader
  ): RuntimeDispatchConnection | ProviderUnavailableError | ServiceProtocolBoundaryError {
    if (!isRuntimeAssemblyRequestDispatchHeader(request)) {
      return new ServiceProtocolBoundaryError(
        'test RuntimeAssembly dispatch requires canonical nested routing'
      );
    }
    const requestError = validateAssemblyTestRequest(
      request,
      this.snapshots.get()
    );
    if (requestError !== undefined) {
      return requestError;
    }
    return this.pickHealthyDispatchConnection();
  }

  private pickHealthyDispatchConnection():
    | RuntimeDispatchConnection
    | ProviderUnavailableError {
    const candidates = this.dispatchCandidates();
    if (candidates.length === 0) {
      return new ProviderUnavailableError(
        'No healthy replica matches the committed RuntimeAssembly generation'
      );
    }
    const replica = candidates[this.nextReplicaCursor % candidates.length];
    this.nextReplicaCursor += 1;
    return replica === undefined
      ? new ProviderUnavailableError()
      : { runtimeId: replica.replicaId, ws: replica.ws };
  }

  validateDispatchRequest(
    request: RuntimeDispatchFrameHeader
  ): ProviderUnavailableError | ServiceProtocolBoundaryError | undefined {
    if (request.type === 'package-test.start') {
      return new ProviderUnavailableError(
        'package test dispatch is not part of the active RuntimeAssembly registry'
      );
    }
    if (!isRuntimeAssemblyRequestDispatchHeader(request)) {
      return new ServiceProtocolBoundaryError(
        'active RuntimeAssembly dispatch requires canonical nested routing'
      );
    }
    const active = this.snapshots.get();
    const mismatch = validateAssemblyRequest(request, active);
    if (mismatch !== undefined) {
      return mismatch;
    }
    return undefined;
  }

  refreshAllRuntimeStates(): void {
    for (const replica of this.replicas.values()) {
      if (replica.ws.readyState !== WebSocket.OPEN) {
        replica.state = 'disconnected';
      }
    }
  }

  refreshRuntimeStatesForRequest(_pending: RuntimeInFlightRequest | undefined): void {
    this.refreshAllRuntimeStates();
  }

  private dispatchCandidates(): AssemblyReplica[] {
    const active = this.snapshots.get();
    return Array.from(this.replicas.values()).filter(
      (replica) =>
        replica.state === 'healthy' &&
        replica.ws.readyState === WebSocket.OPEN &&
        matchesActiveSnapshot(replica, active)
    );
  }

  private registeredReplicaForConnection(ws: WebSocket, replicaId: string): AssemblyReplica {
    const replica = this.replicas.get(replicaId);
    if (
      replica === undefined ||
      replica.ws !== ws ||
      this.replicaIdByConnection.get(ws) !== replicaId
    ) {
      throw new Error(`runtime connection is not registered as replica ${replicaId}`);
    }
    return replica;
  }

  private countInFlight(replica: AssemblyReplica): number {
    return this.inFlightCounter?.countInFlight(replica) ?? 0;
  }

  private replicaCanUseActivation(replica: AssemblyReplica): boolean {
    if (replica.state === 'healthy') {
      return matchesActiveSnapshot(replica, this.snapshots.get());
    }
    return (
      replica.state === 'draining' &&
      (this.countInFlight(replica) > 0 ||
        (this.connectionPinCounter?.connectionPinCount(replica.ws) ?? 0) > 0)
    );
  }
}

function actorSpawnServiceId(
  header: ActorSpawnRuntimeRequestFrameHeader
): string {
  switch (header.type) {
    case 'actor.getOrCreate.request':
    case 'actor.replace.request':
    case 'actor.find.request':
    case 'actor.remove.request':
      return header.actorKey.serviceId;
    case 'spawn.submit.request':
      return header.serviceId;
  }
}

function isSpawnControl(
  header: ActorSpawnRuntimeRequestFrameHeader
): header is Extract<
  ActorSpawnRuntimeRequestFrameHeader,
  { type: 'spawn.submit.request' }
> {
  return header.type === 'spawn.submit.request';
}

function deploymentRevisionServiceSentinel(deploymentRevision: string): string {
  return `\u0000deployment:${deploymentRevision}`;
}

function deploymentBindingsByService(
  snapshot: RouterActiveAssemblySnapshot
): ReadonlyMap<string, AssemblyDeploymentBinding> {
  const bindings = new Map<string, AssemblyDeploymentBinding>();
  const contracts = new Map(
    (snapshot.resolvedContracts ?? []).map((contract) => [
      `${contract.serviceId}\u0000${contract.contractVersion}`,
      contract
    ])
  );
  const runtimeBindings = new Map(
    (snapshot.deploymentRuntimeBindings ?? []).map((binding) => [
      `${binding.deployment.serviceId}\u0000${binding.deployment.contractVersion}\u0000${binding.deployment.deploymentRevision}`,
      binding
    ])
  );
  for (const deployment of snapshot.resolvedDeployments ?? []) {
    const contract = contracts.get(
      `${deployment.serviceId}\u0000${deployment.contractVersion}`
    );
    const runtimeBinding = runtimeBindings.get(
      `${deployment.serviceId}\u0000${deployment.contractVersion}\u0000${deployment.deploymentRevision}`
    );
    if (contract === undefined || runtimeBinding === undefined) {
      continue;
    }
    setDeploymentBinding(bindings, contract.serviceId, {
      deploymentRevision: deployment.deploymentRevision,
      packageBuildId: runtimeBinding.packageBuildId,
      serviceProtocolIdentity: contract.serviceProtocolIdentity,
      maxConcurrency: runtimeBinding.maxConcurrency,
      ...(runtimeBinding.timeoutMs === undefined
        ? {}
        : { timeoutMs: runtimeBinding.timeoutMs })
    });
  }
  return bindings;
}

function setDeploymentBinding(
  bindings: Map<string, AssemblyDeploymentBinding>,
  serviceId: string,
  binding: AssemblyDeploymentBinding
): void {
  const existing = bindings.get(serviceId);
  if (
    existing !== undefined &&
    (existing.deploymentRevision !== binding.deploymentRevision ||
      existing.packageBuildId !== binding.packageBuildId ||
      existing.serviceProtocolIdentity !== binding.serviceProtocolIdentity ||
      existing.maxConcurrency !== binding.maxConcurrency ||
      existing.timeoutMs !== binding.timeoutMs)
  ) {
    throw new Error(
      `RuntimeAssembly has conflicting deployment bindings for ${serviceId}`
    );
  }
  bindings.set(serviceId, binding);
  bindings.set(
    deploymentRevisionServiceSentinel(binding.deploymentRevision),
    binding
  );
}

function matchesActiveSnapshot(
  value: {
    environment: string;
    generation: number;
    assembly?: { assemblyIdentity: string };
    assemblyIdentity?: string;
  },
  active: RouterActiveAssemblySnapshot
): boolean {
  return (
    value.environment === active.environment &&
    value.generation === active.generation &&
    (value.assembly?.assemblyIdentity ?? value.assemblyIdentity) ===
      active.assembly.assemblyIdentity
  );
}

function validateAssemblyRequest(
  candidate: RuntimeAssemblyRequestStartFrameWireHeader,
  active: RouterActiveAssemblySnapshot
): ServiceProtocolBoundaryError | undefined {
  const validation = validateRuntimeAssemblyRequestStartFrameWireHeader(candidate);
  if (!validation.ok) {
    return new ServiceProtocolBoundaryError(validation.error);
  }
  const request = validation.envelope;
  if (!('httpRequest' in request)) {
    return new ServiceProtocolBoundaryError(
      'active RuntimeAssembly dispatch does not accept internally derived spawn requests'
    );
  }
  if (request.testEffectsEnabled !== false) {
    return new ServiceProtocolBoundaryError(
      'active RuntimeAssembly dispatch rejects test effect controls'
    );
  }
  return validateAssemblyRequestFacts(request, active);
}

function validateAssemblyTestRequest(
  candidate: RuntimeAssemblyRequestStartFrameWireHeader,
  active: RouterActiveAssemblySnapshot
): ServiceProtocolBoundaryError | undefined {
  const validation = validateRuntimeAssemblyRequestStartFrameWireHeader(candidate);
  if (!validation.ok) {
    return new ServiceProtocolBoundaryError(validation.error);
  }
  const request = validation.envelope;
  if (!('httpRequest' in request)) {
    return new ServiceProtocolBoundaryError(
      'test RuntimeAssembly root dispatch requires gateway HTTP routing'
    );
  }
  if (request.testEffectsEnabled !== true) {
    return new ServiceProtocolBoundaryError(
      'test RuntimeAssembly dispatch requires test effects enabled'
    );
  }
  return validateAssemblyRequestFacts(request, active);
}

function validateAssemblyRequestFacts(
  request: RuntimeAssemblyRequestStartFrameHeader,
  active: RouterActiveAssemblySnapshot
): ServiceProtocolBoundaryError | undefined {
  if (
    request.routing.assemblyIdentity !== active.assembly.assemblyIdentity ||
    request.routing.assemblyGeneration !== active.generation
  ) {
    return new ServiceProtocolBoundaryError(
      'request does not carry the exact committed RuntimeAssembly generation and ingress identity'
    );
  }
  let canonicalIngress: typeof request.routing.ingress;
  try {
    canonicalIngress = {
      ...request.routing.ingress,
      method: request.routing.ingress.method.toUpperCase()
    };
    if (canonicalIngress.method !== request.routing.ingress.method) {
      throw new Error('request ingress is not canonical');
    }
    runtimeAssemblyIngressKey(canonicalIngress);
  } catch {
    return new ServiceProtocolBoundaryError(
      'request does not carry canonical RuntimeAssembly ingress metadata'
    );
  }
  const binding = active.ingress.get(
    request.routing.deployment,
    canonicalIngress
  );
  if (
    binding === undefined ||
    binding.selector.protocol !== request.routing.ingress.protocol ||
    binding.selector.method !== request.routing.ingress.method ||
    binding.selector.path !== request.routing.ingress.path ||
    !sameDeployment(binding.deployment, request.routing.deployment) ||
    binding.gatewayEntryIdentity !== request.routing.gatewayEntryIdentity
  ) {
    return new ServiceProtocolBoundaryError(
      `request canonical ingress ${runtimeAssemblyIngressKey(request.routing.ingress)} does not match the committed assembly`
    );
  }
  if (request.mode !== binding.operationMode) {
    return new ServiceProtocolBoundaryError(
      'request mode does not match the exact committed gateway binding'
    );
  }
  if (
    binding.operationMode === 'serverStream' &&
    binding.adapterKind !== 'rawHttp'
  ) {
    return new ServiceProtocolBoundaryError(
      'only rawHttp gateway bindings may use serverStream mode'
    );
  }
  return validateAssemblyHttpRequest(request, canonicalIngress);
}

function validateAssemblyHttpRequest(
  request: RuntimeAssemblyRequestStartFrameHeader,
  ingress: RuntimeAssemblyRequestStartFrameHeader['routing']['ingress']
): ServiceProtocolBoundaryError | undefined {
  try {
    const requestUrl = new URL(request.httpRequest.url);
    if (
      request.httpRequest.method !== ingress.method ||
      request.httpRequest.path !== ingress.path ||
      requestUrl.protocol !== 'http:' ||
      requestUrl.username !== '' ||
      requestUrl.password !== '' ||
      requestUrl.hash !== '' ||
      requestUrl.pathname !== ingress.path ||
      canonicalHttpHost(requestUrl.host) !== requestUrl.host
    ) {
      throw new Error('HTTP request metadata does not match routing ingress');
    }
  } catch {
    return new ServiceProtocolBoundaryError(
      'request does not carry matching canonical RuntimeAssembly HTTP ingress metadata'
    );
  }
  return undefined;
}

function sameDeployment(
  left: RuntimeAssemblyIngressBinding['deployment'],
  right: RuntimeAssemblyIngressBinding['deployment']
): boolean {
  return (
    left.serviceId === right.serviceId &&
    left.contractVersion === right.contractVersion &&
    left.deploymentRevision === right.deploymentRevision &&
    left.deploymentArtifactIdentity === right.deploymentArtifactIdentity
  );
}
