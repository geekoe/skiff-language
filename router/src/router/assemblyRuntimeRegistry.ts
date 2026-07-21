import WebSocket from 'ws';

import type {
  AssemblyActivationControl
} from '../protocol/assemblyActivationProtocol.js';
import type {
  RequestStartFrameHeader,
  RuntimeHealthCounters
} from '../protocol/envelope.js';
import { ProviderUnavailableError, ServiceProtocolBoundaryError } from './errors.js';
import type {
  RuntimeDispatchConnection,
  RuntimeDispatchFrameHeader,
  RuntimeDispatchRuntimeIdentity,
  RuntimeInFlightCounter,
  RuntimeInFlightRequest
} from './runtimeRegistry.js';
import {
  canonicalIngressHost,
  RouterActiveAssemblySnapshotStore,
  runtimeAssemblyIngressKey,
  type RouterActiveAssemblySnapshot
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
}

export interface AssemblyReplicaSnapshot {
  replicaId: string;
  environment: string;
  generation: number;
  assemblyIdentity: string;
  state: AssemblyReplicaState;
  connected: boolean;
  inFlightCount: number;
  registeredAt: string;
  lastHealthAt?: string;
  healthCounters?: RuntimeHealthCounters;
}

export class AssemblyRuntimeRegistry {
  private readonly replicas = new Map<string, AssemblyReplica>();
  private readonly replicaIdByConnection = new Map<WebSocket, string>();
  private inFlightCounter: RuntimeInFlightCounter | undefined;
  private nextReplicaCursor = 0;

  constructor(private readonly snapshots: RouterActiveAssemblySnapshotStore) {}

  setInFlightCounter(counter: RuntimeInFlightCounter | undefined): void {
    this.inFlightCounter = counter;
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

  replicaIdForConnection(ws: WebSocket): string | undefined {
    return this.replicaIdByConnection.get(ws);
  }

  assertReplicaConnection(ws: WebSocket, replicaId: string): void {
    this.registeredReplicaForConnection(ws, replicaId);
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
    if (request.type === 'package-test.start') {
      return new ProviderUnavailableError(
        'package test dispatch is not part of the active RuntimeAssembly registry'
      );
    }
    const active = this.snapshots.get();
    const mismatch = validateAssemblyRequest(request, active);
    if (mismatch !== undefined) {
      return mismatch;
    }
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
  request: RequestStartFrameHeader,
  active: RouterActiveAssemblySnapshot
): ServiceProtocolBoundaryError | undefined {
  if (
    request.assemblyIdentity !== active.assembly.assemblyIdentity ||
    request.assemblyGeneration !== active.generation ||
    request.ingress === undefined ||
    request.contractOperationId === undefined
  ) {
    return new ServiceProtocolBoundaryError(
      'request does not carry the exact committed RuntimeAssembly generation and ingress identity'
    );
  }
  let canonicalIngress: typeof request.ingress;
  try {
    canonicalIngress = {
      ...request.ingress,
      host: canonicalIngressHost(request.ingress.host),
      method:
        request.ingress.method === null ? null : request.ingress.method.toUpperCase()
    };
    if (
      canonicalIngress.host !== request.ingress.host ||
      canonicalIngress.method !== request.ingress.method
    ) {
      throw new Error('request ingress is not canonical');
    }
    runtimeAssemblyIngressKey(canonicalIngress);
  } catch {
    return new ServiceProtocolBoundaryError(
      'request does not carry a canonical RuntimeAssembly ingress identity'
    );
  }
  const binding = active.ingress.get(canonicalIngress);
  if (
    binding === undefined ||
    binding.contractOperationId !== request.contractOperationId ||
    request.target !== request.contractOperationId ||
    request.operationAbiId !== request.contractOperationId ||
    binding.contract.serviceProtocolIdentity !== request.serviceProtocolIdentity
  ) {
    return new ServiceProtocolBoundaryError(
      `request canonical ingress ${runtimeAssemblyIngressKey(request.ingress)} does not match the committed assembly`
    );
  }
  return undefined;
}
