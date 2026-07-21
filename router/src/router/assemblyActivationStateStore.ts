import {
  lstat,
  open,
  readFile,
  realpath,
  rename,
  rm
} from 'node:fs/promises';
import { dirname, join, relative, resolve, sep } from 'node:path';

import {
  ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
  decodeEnvironmentActivationState,
  type AssemblyActivationRequest,
  type EnvironmentActivationState,
  type PendingActivation
} from '../protocol/assemblyActivationProtocol.js';
import { decodeRawEnvironmentActivationState } from '../protocol/assemblyActivationRawCodec.js';

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

export class FileAssemblyActivationStateStore implements AssemblyActivationStateStore {
  private mutation: Promise<void> = Promise.resolve();
  private root: string | undefined;

  constructor(private readonly artifactRoot: string) {}

  async read(environment: string): Promise<EnvironmentActivationState> {
    const path = await this.statePath(environment);
    const bytes = await readFile(path);
    const state = decodeRawEnvironmentActivationState(bytes);
    if (state.environment !== environment) {
      throw new Error('activation state environment does not match its canonical path');
    }
    if (!bytes.equals(Buffer.from(canonicalActivationJson(state)))) {
      throw new Error('activation state is not canonical JSON');
    }
    return state;
  }

  prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    const participants = canonicalReplicaIds(participantReplicaIds);
    return this.mutate(request.environment, (current) => {
      if (current.committed.generation !== request.expectedGeneration) {
        throw new Error('activation prepare expected generation is stale');
      }
      const pending: PendingActivation = {
        activationId: request.activationId,
        expectedGeneration: request.expectedGeneration,
        candidateGeneration: request.expectedGeneration + 1,
        assembly: request.assembly,
        participantReplicaIds: participants
      };
      if (current.pending !== null) {
        if (samePendingActivation(current.pending, pending)) {
          return current;
        }
        throw new Error('a different assembly activation is already pending');
      }
      return decodeEnvironmentActivationState({
        ...current,
        pending
      });
    });
  }

  abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    return this.mutateForPending(environment, pending, (current) => ({
      ...current,
      pending: null
    }));
  }

  commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    const connected = new Set(canonicalReplicaIds(connectedReplicaIds));
    const prepared = canonicalReplicaIds(preparedReplicaIds);
    if (
      prepared.length !== pending.participantReplicaIds.length ||
      prepared.some((replicaId, index) => replicaId !== pending.participantReplicaIds[index]) ||
      pending.participantReplicaIds.some((replicaId) => !connected.has(replicaId))
    ) {
      throw new Error('activation commit requires every frozen participant connected and prepared');
    }
    return this.mutate(environment, (current) => {
      if (
        current.pending === null &&
        current.committed.generation === pending.candidateGeneration &&
        current.committed.assembly.assemblyIdentity === pending.assembly.assemblyIdentity
      ) {
        return current;
      }
      assertPendingTuple(current, pending);
      return decodeEnvironmentActivationState({
        ...current,
        committed: {
          generation: pending.candidateGeneration,
          assembly: pending.assembly
        },
        pending: null
      });
    });
  }

  private mutateForPending(
    environment: string,
    pending: PendingActivation,
    update: (current: EnvironmentActivationState) => EnvironmentActivationState
  ): Promise<EnvironmentActivationState> {
    return this.mutate(environment, (current) => {
      if (current.pending === null) {
        if (current.committed.generation !== pending.expectedGeneration) {
          throw new Error('activation abort expected generation is stale');
        }
        return current;
      }
      assertPendingTuple(current, pending);
      return decodeEnvironmentActivationState(update(current));
    });
  }

  private mutate(
    environment: string,
    update: (current: EnvironmentActivationState) => EnvironmentActivationState
  ): Promise<EnvironmentActivationState> {
    let resolveMutation!: () => void;
    const previous = this.mutation;
    this.mutation = new Promise<void>((resolveMutationPromise) => {
      resolveMutation = resolveMutationPromise;
    });
    return previous
      .then(async () => {
        const current = await this.read(environment);
        const next = update(current);
        if (next !== current) {
          await this.replace(environment, next);
        }
        return next;
      })
      .finally(resolveMutation);
  }

  private async statePath(environment: string): Promise<string> {
    if (!/^[A-Za-z0-9._-]{1,200}$/.test(environment) || environment === '.' || environment === '..') {
      throw new Error('invalid activation environment');
    }
    const root = this.root ?? (this.root = await realpath(resolve(this.artifactRoot)));
    const path = resolve(root, 'environments', environment, 'activation.json');
    const pathRelative = relative(root, path);
    if (pathRelative.startsWith(`..${sep}`) || pathRelative === '..') {
      throw new Error('activation state path escapes artifact root');
    }
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink()) {
      throw new Error('activation state path must not be a symlink');
    }
    if (!metadata.isFile()) {
      throw new Error('activation state path must be a file');
    }
    const canonicalParent = await realpath(dirname(path));
    if (canonicalParent !== dirname(path)) {
      throw new Error('activation state parent must not contain symlinks');
    }
    return path;
  }

  private async replace(
    environment: string,
    state: EnvironmentActivationState
  ): Promise<void> {
    const destination = await this.statePath(environment);
    const temporary = join(
      dirname(destination),
      `.activation.${process.pid}.${Date.now()}.${Math.random().toString(16).slice(2)}.tmp`
    );
    let handle: Awaited<ReturnType<typeof open>> | undefined;
    try {
      handle = await open(temporary, 'wx', 0o600);
      await handle.writeFile(canonicalActivationJson(state));
      await handle.sync();
      await handle.close();
      handle = undefined;
      await rename(temporary, destination);
      const parent = await open(dirname(destination), 'r');
      try {
        await parent.sync();
      } finally {
        await parent.close();
      }
    } finally {
      await handle?.close();
      await rm(temporary, { force: true });
    }
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
    const current = await this.read(request.environment);
    if (current.committed.generation !== request.expectedGeneration) {
      throw new Error('activation prepare CAS mismatch');
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
    this.state = decodeEnvironmentActivationState({ ...current, pending });
    return this.read(request.environment);
  }

  async abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    if (environment !== this.state.environment) {
      throw new Error(`unknown activation environment ${environment}`);
    }
    const current = this.state;
    if (current.pending === null) {
      if (current.committed.generation !== pending.expectedGeneration) {
        throw new Error('activation abort expected generation is stale');
      }
      return structuredClone(current);
    }
    assertPendingTuple(current, pending);
    this.state = decodeEnvironmentActivationState({ ...current, pending: null });
    return structuredClone(this.state);
  }

  async commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    const current = await this.read(environment);
    if (
      current.pending === null &&
      current.committed.generation === pending.candidateGeneration &&
      current.committed.assembly.assemblyIdentity === pending.assembly.assemblyIdentity
    ) {
      return structuredClone(current);
    }
    assertPendingTuple(current, pending);
    const participants = canonicalReplicaIds(pending.participantReplicaIds);
    const prepared = canonicalReplicaIds(preparedReplicaIds);
    const connected = new Set(canonicalReplicaIds(connectedReplicaIds));
    if (
      prepared.length !== participants.length ||
      prepared.some((replicaId, index) => replicaId !== participants[index]) ||
      participants.some((replicaId) => !connected.has(replicaId))
    ) {
      throw new Error('activation commit requires every frozen participant connected and prepared');
    }
    this.state = decodeEnvironmentActivationState({
      ...current,
      committed: {
        generation: pending.candidateGeneration,
        assembly: pending.assembly
      },
      pending: null
    });
    return structuredClone(this.state);
  }
}

export function canonicalActivationJson(state: EnvironmentActivationState): string {
  return JSON.stringify(sortJsonValue(state));
}

function sortJsonValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJsonValue);
  }
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, sortJsonValue(nested)])
    );
  }
  return value;
}

function canonicalReplicaIds(replicaIds: readonly string[]): readonly string[] {
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
