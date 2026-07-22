import {
  ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
  decodeEnvironmentActivationState,
  type AssemblyActivationRequest,
  type EnvironmentActivationState,
  type PendingActivation
} from '../protocol/assemblyActivationProtocol.js';
import {
  AssemblyActivationFilePersistence,
  type ActivationPersistenceFailpoint
} from './assemblyActivationFilePersistence.js';
import {
  withActivationFileLock,
  type ActivationFileLockOptions
} from './assemblyActivationFileLock.js';
import {
  abortActivationState,
  commitActivationState,
  prepareActivationState
} from './assemblyActivationStateReducer.js';

export { canonicalActivationJson } from './assemblyActivationFilePersistence.js';

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

export type FileAssemblyActivationStateStoreOptions = Readonly<{
  lock?: ActivationFileLockOptions;
  persistenceFailpoint?: ActivationPersistenceFailpoint;
}>;

export class FileAssemblyActivationStateStore implements AssemblyActivationStateStore {
  private readonly persistence: AssemblyActivationFilePersistence;

  constructor(
    artifactRoot: string,
    private readonly options: FileAssemblyActivationStateStoreOptions = {}
  ) {
    this.persistence = new AssemblyActivationFilePersistence(
      artifactRoot,
      options.persistenceFailpoint
    );
  }

  async read(environment: string): Promise<EnvironmentActivationState> {
    return this.persistence.read(environment);
  }

  prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return this.mutate(request.environment, (current) =>
      prepareActivationState(current, request, participantReplicaIds)
    );
  }

  abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    return this.mutate(environment, (current) =>
      abortActivationState(current, environment, pending)
    );
  }

  commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return this.mutate(environment, (current) =>
      commitActivationState(
        current,
        environment,
        pending,
        connectedReplicaIds,
        preparedReplicaIds
      )
    );
  }

  private async mutate(
    environment: string,
    update: (current: EnvironmentActivationState) => EnvironmentActivationState
  ): Promise<EnvironmentActivationState> {
    const { lock } = await this.persistence.paths(environment);
    return withActivationFileLock(lock, async () => {
      const current = await this.read(environment);
      const next = update(current);
      if (next !== current) {
        await this.persistence.replace(environment, next);
      }
      return next;
    }, this.options.lock);
  }
}

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
}): EnvironmentActivationState {
  return decodeEnvironmentActivationState({
    schemaVersion: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
    environment: input.environment,
    committed: {
      generation: input.generation,
      assembly: { assemblyIdentity: input.assemblyIdentity }
    },
    pending: null
  });
}
