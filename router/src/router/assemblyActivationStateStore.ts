import {
  ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
  decodeEnvironmentActivationState,
  type AssemblyActivationRequest,
  type EnvironmentActivationState,
  type PendingActivation
} from '../protocol/assemblyActivationProtocol.js';
import {
  abortActivationState,
  commitActivationState,
  prepareActivationState
} from './assemblyActivationStateReducer.js';

export interface AssemblyActivationStateStore {
  read(environment: string): Promise<EnvironmentActivationState>;
  prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState>;
  abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState>;
  commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState>;
}

/** Direct-test fake. Production uses MongoAssemblyActivationStateStore. */
export class MemoryAssemblyActivationStateStore implements AssemblyActivationStateStore {
  private state: EnvironmentActivationState;

  constructor(initial: EnvironmentActivationState) {
    this.state = decodeEnvironmentActivationState(initial);
  }

  async read(environment: string): Promise<EnvironmentActivationState> {
    if (this.state.environment !== environment) {
      throw new Error(`unknown activation environment ${environment}`);
    }
    return structuredClone(this.state);
  }

  async prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    this.state = prepareActivationState(this.state, request, participantReplicaIds);
    return structuredClone(this.state);
  }

  async abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    this.state = abortActivationState(this.state, environment, pending);
    return structuredClone(this.state);
  }

  async commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    this.state = commitActivationState(
      this.state,
      environment,
      pending,
      connectedReplicaIds,
      preparedReplicaIds
    );
    return structuredClone(this.state);
  }
}

export function initialActivationState(input: {
  environment: string;
  generation: number;
  assemblyIdentity: string;
  configSnapshotId: string;
}): EnvironmentActivationState {
  return decodeEnvironmentActivationState({
    schemaVersion: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
    environment: input.environment,
    committed: {
      generation: input.generation,
      assembly: { assemblyIdentity: input.assemblyIdentity },
      configSnapshot: { snapshotId: input.configSnapshotId }
    },
    pending: null
  });
}
