import {
  decodeEnvironmentActivationState,
  type AssemblyActivationRequest,
  type EnvironmentActivationState,
  type PendingActivation
} from '../protocol/assemblyActivationProtocol.js';

export function prepareActivationState(
  current: EnvironmentActivationState,
  request: AssemblyActivationRequest,
  participantReplicaIds: readonly string[]
): EnvironmentActivationState {
  assertEnvironment(current, request.environment);
  if (current.committed.generation !== request.expectedGeneration) {
    throw new Error('activation prepare expected generation is stale');
  }
  const pending: PendingActivation = {
    activationId: request.activationId,
    expectedGeneration: request.expectedGeneration,
    candidateGeneration: request.expectedGeneration + 1,
    assembly: request.assembly,
    participantReplicaIds: canonicalReplicaIds(participantReplicaIds)
  };
  if (current.pending !== null) {
    if (samePendingActivation(current.pending, pending)) {
      return current;
    }
    throw new Error('a different assembly activation is already pending');
  }
  return decodeEnvironmentActivationState({ ...current, pending });
}

export function abortActivationState(
  current: EnvironmentActivationState,
  environment: string,
  pending: PendingActivation
): EnvironmentActivationState {
  assertEnvironment(current, environment);
  if (current.pending === null) {
    if (current.committed.generation !== pending.expectedGeneration) {
      throw new Error('activation abort expected generation is stale');
    }
    return current;
  }
  assertPendingTuple(current, pending);
  return decodeEnvironmentActivationState({ ...current, pending: null });
}

export function commitActivationState(
  current: EnvironmentActivationState,
  environment: string,
  pending: PendingActivation,
  connectedReplicaIds: readonly string[],
  preparedReplicaIds: readonly string[]
): EnvironmentActivationState {
  assertEnvironment(current, environment);
  if (
    current.pending === null &&
    current.committed.generation === pending.candidateGeneration &&
    current.committed.assembly.assemblyIdentity === pending.assembly.assemblyIdentity
  ) {
    return current;
  }
  assertPendingTuple(current, pending);
  const participants = canonicalReplicaIds(pending.participantReplicaIds);
  const connected = new Set(canonicalReplicaIds(connectedReplicaIds));
  const prepared = canonicalReplicaIds(preparedReplicaIds);
  if (
    prepared.length !== participants.length ||
    prepared.some((replicaId, index) => replicaId !== participants[index]) ||
    participants.some((replicaId) => !connected.has(replicaId))
  ) {
    throw new Error('activation commit requires every frozen participant connected and prepared');
  }
  return decodeEnvironmentActivationState({
    ...current,
    committed: {
      generation: pending.candidateGeneration,
      assembly: pending.assembly
    },
    pending: null
  });
}

export function canonicalReplicaIds(replicaIds: readonly string[]): readonly string[] {
  if (replicaIds.length === 0) {
    throw new Error('activation participant replica set must not be empty');
  }
  const sorted = [...replicaIds].sort((left, right) =>
    Buffer.compare(Buffer.from(left), Buffer.from(right))
  );
  if (new Set(sorted).size !== sorted.length) {
    throw new Error('activation participant replica ids must be unique');
  }
  return sorted;
}

function assertEnvironment(state: EnvironmentActivationState, environment: string): void {
  if (state.environment !== environment) {
    throw new Error(`unknown activation environment ${environment}`);
  }
}

function samePendingActivation(left: PendingActivation, right: PendingActivation): boolean {
  return (
    left.activationId === right.activationId &&
    left.expectedGeneration === right.expectedGeneration &&
    left.candidateGeneration === right.candidateGeneration &&
    left.assembly.assemblyIdentity === right.assembly.assemblyIdentity &&
    left.participantReplicaIds.length === right.participantReplicaIds.length &&
    left.participantReplicaIds.every(
      (replicaId, index) => replicaId === right.participantReplicaIds[index]
    )
  );
}

function assertPendingTuple(
  state: EnvironmentActivationState,
  pending: PendingActivation
): void {
  if (state.pending === null || !samePendingActivation(state.pending, pending)) {
    throw new Error('activation transaction does not match durable pending tuple');
  }
  if (state.committed.generation !== pending.expectedGeneration) {
    throw new Error('activation pending expected generation is stale');
  }
}
