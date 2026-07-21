import WebSocket from 'ws';

import type {
  AssemblyActivationControl,
  AssemblyActivationRequest,
  EnvironmentActivationState,
  PendingActivation
} from '../protocol/assemblyActivationProtocol.js';
import type { AssemblyActivationStateStore } from './assemblyActivationStateStore.js';
import type { AssemblyRuntimeRegistry } from './assemblyRuntimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  snapshotFromCommittedActivation,
  type RouterActiveAssemblySnapshot,
  type RuntimeAssemblySnapshotLoader
} from './runtimeAssemblySnapshot.js';

export interface AssemblyActivationControlSender {
  sendAssemblyControl(ws: WebSocket, control: AssemblyActivationControl): void;
}

export interface AssemblyActivationCoordinatorOptions {
  environment: string;
  stateStore: AssemblyActivationStateStore;
  assemblyLoader: RuntimeAssemblySnapshotLoader;
  snapshots: RouterActiveAssemblySnapshotStore;
  registry: AssemblyRuntimeRegistry;
  controlSender: AssemblyActivationControlSender;
  prepareTimeoutMs?: number;
}

interface PendingTransaction {
  pending: PendingActivation;
  candidateSnapshot: RouterActiveAssemblySnapshot;
  preparedReplicaIds: Set<string>;
  prepareSentReplicaIds: Set<string>;
  completion: Promise<EnvironmentActivationState>;
  resolve(state: EnvironmentActivationState): void;
  reject(error: unknown): void;
  timeout: NodeJS.Timeout;
  settled: boolean;
}

type RuntimeActivationResponse = Readonly<{
  type: 'prepared' | 'reject';
  environment: string;
  activationId: string;
  expectedGeneration: number;
  candidateGeneration: number;
  assembly: { assemblyIdentity: string };
  replicaId: string;
  reason?: string;
}>;

export class AssemblyActivationCoordinator {
  private state: EnvironmentActivationState | undefined;
  private transaction: PendingTransaction | undefined;
  private mutation: Promise<void> = Promise.resolve();

  constructor(private readonly options: AssemblyActivationCoordinatorOptions) {}

  async initialize(): Promise<RouterActiveAssemblySnapshot> {
    return await this.enqueue(async () => {
      const state = await this.options.stateStore.read(this.options.environment);
      const snapshot = await snapshotFromCommittedActivation(state, this.options.assemblyLoader);
      this.options.snapshots.replace(snapshot);
      this.options.registry.activate(snapshot);
      this.state = state;
      if (state.pending !== null) {
        try {
          const candidateSnapshot = await snapshotFromPendingActivation(
            state,
            state.pending,
            this.options.assemblyLoader
          );
          this.installTransaction(state.pending, candidateSnapshot);
        } catch {
          this.state = await this.options.stateStore.abort(state.environment, state.pending);
        }
      }
      return snapshot;
    });
  }

  async activate(request: AssemblyActivationRequest): Promise<EnvironmentActivationState> {
    const transaction = await this.enqueue(async () => {
      this.assertInitialized();
      if (request.environment !== this.options.environment) {
        throw new Error(`router coordinates environment ${this.options.environment}`);
      }
      if (this.transaction !== undefined) {
        if (matchesActivationRequest(this.transaction.pending, request)) {
          return this.transaction;
        }
        throw new Error('a different assembly activation is already pending');
      }
      const current = this.state!;
      if (current.committed.generation !== request.expectedGeneration) {
        throw new Error('assembly activation expected generation is stale');
      }
      const candidateSnapshot = await snapshotFromRequestActivation(
        request,
        this.options.assemblyLoader
      );
      const participants = this.options.registry.healthyParticipantReplicaIds();
      if (participants.length === 0) {
        throw new Error('assembly activation requires at least one healthy participant replica');
      }
      const prepared = await this.options.stateStore.prepare(request, participants);
      const pending = prepared.pending;
      if (pending === null) {
        throw new Error('activation prepare CAS did not create durable pending state');
      }
      this.state = prepared;
      const installed = this.installTransaction(pending, candidateSnapshot);
      try {
        this.sendPrepareToConnectedParticipants(installed);
      } catch (error) {
        await this.abortTransaction(
          installed,
          error instanceof Error ? error : new Error(String(error))
        );
      }
      return installed;
    });
    return await transaction.completion;
  }

  handleRuntimeControl(ws: WebSocket, control: AssemblyActivationControl): void {
    void this.enqueue(async () => {
      if (control.type !== 'prepared' && control.type !== 'reject') {
        throw new Error(`runtime must not send assembly activation ${control.type}`);
      }
      const response = control as RuntimeActivationResponse;
      if (
        this.transaction === undefined &&
        response.type === 'prepared' &&
        this.matchesCommittedReplay(response)
      ) {
        this.options.controlSender.sendAssemblyControl(
          ws,
          responseTransitionControl('commit', response)
        );
        return;
      }
      const transaction = this.requireExactTransaction(response);
      this.options.registry.assertReplicaConnection(ws, response.replicaId);
      if (!transaction.pending.participantReplicaIds.includes(response.replicaId)) {
        throw new Error(`replica ${response.replicaId} is not a frozen activation participant`);
      }
      if (response.type === 'reject') {
        await this.abortTransaction(
          transaction,
          new Error(`replica ${response.replicaId} rejected activation during ${response.reason}`)
        );
        return;
      }
      transaction.preparedReplicaIds.add(response.replicaId);
      if (this.allParticipantsPrepared(transaction)) {
        await this.commitTransaction(transaction);
      }
    }).catch((error: unknown) => {
      ws.close(1008, error instanceof Error ? error.message : 'invalid activation control');
    });
  }

  handleReplicaRegistered(replicaId: string): void {
    void this.enqueue(async () => {
      const transaction = this.transaction;
      if (
        transaction === undefined ||
        !transaction.pending.participantReplicaIds.includes(replicaId)
      ) {
        return;
      }
      try {
        this.sendPrepare(transaction, replicaId);
      } catch (error) {
        await this.abortTransaction(
          transaction,
          error instanceof Error ? error : new Error(String(error))
        );
      }
    });
  }

  handleReplicaDisconnected(replicaId: string | undefined): void {
    if (replicaId === undefined) {
      return;
    }
    void this.enqueue(async () => {
      const transaction = this.transaction;
      if (
        transaction !== undefined &&
        transaction.pending.participantReplicaIds.includes(replicaId)
      ) {
        await this.abortTransaction(
          transaction,
          new Error(`activation participant ${replicaId} disconnected`)
        );
      }
    });
  }

  activationState(): EnvironmentActivationState {
    this.assertInitialized();
    return structuredClone(this.state!);
  }

  private installTransaction(
    pending: PendingActivation,
    candidateSnapshot: RouterActiveAssemblySnapshot
  ): PendingTransaction {
    let resolve!: (state: EnvironmentActivationState) => void;
    let reject!: (error: unknown) => void;
    const completion = new Promise<EnvironmentActivationState>((resolvePromise, rejectPromise) => {
      resolve = resolvePromise;
      reject = rejectPromise;
    });
    void completion.catch(() => undefined);
    const transaction: PendingTransaction = {
      pending,
      candidateSnapshot,
      preparedReplicaIds: new Set(),
      prepareSentReplicaIds: new Set(),
      completion,
      resolve,
      reject,
      settled: false,
      timeout: setTimeout(() => {
        void this.enqueue(async () => {
          if (this.transaction === transaction && !transaction.settled) {
            await this.abortTransaction(
              transaction,
              new Error('assembly activation prepare timed out')
            );
          }
        });
      }, this.options.prepareTimeoutMs ?? 20_000)
    };
    this.transaction = transaction;
    return transaction;
  }

  private sendPrepareToConnectedParticipants(transaction: PendingTransaction): void {
    for (const replicaId of transaction.pending.participantReplicaIds) {
      if (this.options.registry.isReplicaConnected(replicaId)) {
        this.sendPrepare(transaction, replicaId);
      }
    }
  }

  private sendPrepare(transaction: PendingTransaction, replicaId: string): void {
    if (transaction.prepareSentReplicaIds.has(replicaId)) {
      return;
    }
    const ws = this.options.registry.connectionForReplica(replicaId);
    if (ws === undefined) {
      return;
    }
    this.options.controlSender.sendAssemblyControl(
      ws,
      transitionControl('prepare', this.options.environment, transaction.pending, replicaId)
    );
    transaction.prepareSentReplicaIds.add(replicaId);
  }

  private async commitTransaction(transaction: PendingTransaction): Promise<void> {
    if (transaction.settled || this.transaction !== transaction) {
      return;
    }
    const connected = this.options.registry.connectedParticipantReplicaIds(
      transaction.pending.participantReplicaIds
    );
    if (connected.length !== transaction.pending.participantReplicaIds.length) {
      await this.abortTransaction(
        transaction,
        new Error('activation participant disconnected before commit CAS')
      );
      return;
    }
    const prepared = [...transaction.preparedReplicaIds].sort((left, right) =>
      Buffer.compare(Buffer.from(left), Buffer.from(right))
    );
    const state = await this.options.stateStore.commit(
      this.options.environment,
      transaction.pending,
      connected,
      prepared
    );
    this.state = state;
    this.options.snapshots.replace(transaction.candidateSnapshot);
    this.options.registry.activate(transaction.candidateSnapshot);
    transaction.settled = true;
    clearTimeout(transaction.timeout);
    this.transaction = undefined;
    transaction.resolve(state);
    for (const replicaId of transaction.pending.participantReplicaIds) {
      const ws = this.options.registry.connectionForReplica(replicaId);
      if (ws !== undefined) {
        try {
          this.options.controlSender.sendAssemblyControl(
            ws,
            transitionControl('commit', this.options.environment, transaction.pending, replicaId)
          );
        } catch {
          // The durable committed record is authoritative. A disconnected
          // replica converges from that record when it reconnects.
        }
      }
    }
  }

  private async abortTransaction(
    transaction: PendingTransaction,
    reason: Error
  ): Promise<void> {
    if (transaction.settled || this.transaction !== transaction) {
      return;
    }
    const state = await this.options.stateStore.abort(
      this.options.environment,
      transaction.pending
    );
    this.state = state;
    transaction.settled = true;
    clearTimeout(transaction.timeout);
    this.transaction = undefined;
    transaction.reject(reason);
    for (const replicaId of transaction.pending.participantReplicaIds) {
      const ws = this.options.registry.connectionForReplica(replicaId);
      if (ws !== undefined) {
        try {
          this.options.controlSender.sendAssemblyControl(
            ws,
            transitionControl('abort', this.options.environment, transaction.pending, replicaId)
          );
        } catch {
          // Abort is already durable and no staged registration is visible.
        }
      }
    }
  }

  private allParticipantsPrepared(transaction: PendingTransaction): boolean {
    return transaction.pending.participantReplicaIds.every((replicaId) =>
      transaction.preparedReplicaIds.has(replicaId)
    );
  }

  private requireExactTransaction(
    control: RuntimeActivationResponse
  ): PendingTransaction {
    const transaction = this.transaction;
    if (transaction === undefined || !matchesPendingControl(transaction.pending, control)) {
      throw new Error('runtime activation response does not match durable pending tuple');
    }
    if (control.environment !== this.options.environment) {
      throw new Error('runtime activation response environment mismatch');
    }
    return transaction;
  }

  private matchesCommittedReplay(control: RuntimeActivationResponse): boolean {
    const committed = this.state?.committed;
    return (
      control.environment === this.options.environment &&
      committed !== undefined &&
      control.candidateGeneration === committed.generation &&
      control.expectedGeneration + 1 === committed.generation &&
      control.assembly.assemblyIdentity === committed.assembly.assemblyIdentity
    );
  }

  private assertInitialized(): void {
    if (this.state === undefined) {
      throw new Error('assembly activation coordinator is not initialized');
    }
  }

  private enqueue<T>(action: () => Promise<T>): Promise<T> {
    const result = this.mutation.then(action, action);
    this.mutation = result.then(
      () => undefined,
      () => undefined
    );
    return result;
  }
}

async function snapshotFromRequestActivation(
  request: AssemblyActivationRequest,
  loader: RuntimeAssemblySnapshotLoader
): Promise<RouterActiveAssemblySnapshot> {
  const assembly = await loader.load(request.assembly);
  if (assembly.assemblyIdentity !== request.assembly.assemblyIdentity) {
    throw new Error('candidate RuntimeAssembly identity mismatch');
  }
  return {
    environment: request.environment,
    generation: request.expectedGeneration + 1,
    assembly: request.assembly,
    ingress: new RuntimeAssemblyIngressIndex(assembly.globalIngress)
  };
}

async function snapshotFromPendingActivation(
  state: EnvironmentActivationState,
  pending: PendingActivation,
  loader: RuntimeAssemblySnapshotLoader
): Promise<RouterActiveAssemblySnapshot> {
  return await snapshotFromRequestActivation(
    {
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment: state.environment,
      activationId: pending.activationId,
      expectedGeneration: pending.expectedGeneration,
      assembly: pending.assembly
    },
    loader
  );
}

function transitionControl(
  type: 'prepare' | 'commit' | 'abort',
  environment: string,
  pending: PendingActivation,
  replicaId: string
): AssemblyActivationControl {
  return {
    type,
    environment,
    activationId: pending.activationId,
    expectedGeneration: pending.expectedGeneration,
    candidateGeneration: pending.candidateGeneration,
    assembly: pending.assembly,
    replicaId
  };
}

function responseTransitionControl(
  type: 'commit',
  response: RuntimeActivationResponse
): AssemblyActivationControl {
  return {
    type,
    environment: response.environment,
    activationId: response.activationId,
    expectedGeneration: response.expectedGeneration,
    candidateGeneration: response.candidateGeneration,
    assembly: response.assembly,
    replicaId: response.replicaId
  };
}

function matchesActivationRequest(
  pending: PendingActivation,
  request: AssemblyActivationRequest
): boolean {
  return (
    pending.activationId === request.activationId &&
    pending.expectedGeneration === request.expectedGeneration &&
    pending.candidateGeneration === request.expectedGeneration + 1 &&
    pending.assembly.assemblyIdentity === request.assembly.assemblyIdentity
  );
}

function matchesPendingControl(
  pending: PendingActivation,
  control: RuntimeActivationResponse
): boolean {
  return (
    pending.activationId === control.activationId &&
    pending.expectedGeneration === control.expectedGeneration &&
    pending.candidateGeneration === control.candidateGeneration &&
    pending.assembly.assemblyIdentity === control.assembly.assemblyIdentity
  );
}
