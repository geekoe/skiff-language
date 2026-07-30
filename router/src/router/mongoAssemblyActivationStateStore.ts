import {
  MongoClient,
  type ClientSession,
  type Collection,
  type Db,
  type MongoClientOptions
} from 'mongodb';

import {
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
import type { AssemblyActivationStateStore } from './assemblyActivationStateStore.js';

const STATE_COLLECTION = 'router_assembly_activation_states';
const AUDIT_COLLECTION = 'router_assembly_activation_audit';

type ActivationStateDocument = Readonly<{
  _id: string;
  revision: number;
  state: EnvironmentActivationState;
}>;

export type ActivationAuditDocument = Readonly<{
  _id: string;
  schemaVersion: 'skiff-router-activation-audit-v2';
  environment: string;
  activationId: string;
  transition: 'prepare' | 'commit' | 'abort';
  beforeGeneration: number;
  afterGeneration: number;
  assemblyIdentity: string;
  configSnapshotId: string;
  participantReplicaIds: readonly string[];
  connectedReplicaIds: readonly string[];
  preparedReplicaIds: readonly string[];
  recordedAt: Date;
}>;

export type MongoAssemblyActivationStateStoreOptions = Readonly<{
  stateCollectionName?: string;
  auditCollectionName?: string;
  now?: () => Date;
}>;

/**
 * Router-owned durable activation state.
 *
 * The caller supplies the Router's already-resolved Mongo database. This module
 * neither reads configuration nor knows about Registry service collections.
 */
export class MongoAssemblyActivationStateStore implements AssemblyActivationStateStore {
  private readonly client: MongoClient;
  private readonly states: Collection<ActivationStateDocument>;
  private readonly audit: Collection<ActivationAuditDocument>;
  private readonly now: () => Date;

  constructor(database: Db, options: MongoAssemblyActivationStateStoreOptions = {}) {
    this.client = database.client;
    this.states = database.collection(
      options.stateCollectionName ?? STATE_COLLECTION
    );
    this.audit = database.collection(
      options.auditCollectionName ?? AUDIT_COLLECTION
    );
    this.now = options.now ?? (() => new Date());
  }

  async ensureIndexes(): Promise<void> {
    await Promise.all([
      this.states.createIndex({ 'state.environment': 1 }, { unique: true }),
      this.audit.createIndex(
        { environment: 1, activationId: 1, transition: 1 },
        { unique: true }
      ),
      this.audit.createIndex({ environment: 1, recordedAt: 1 })
    ]);
  }

  async initialize(state: EnvironmentActivationState): Promise<EnvironmentActivationState> {
    const decoded = decodeEnvironmentActivationState(state);
    const result = await this.states.findOneAndUpdate(
      { _id: decoded.environment },
      {
        $setOnInsert: {
          _id: decoded.environment,
          revision: 0,
          state: decoded
        }
      },
      { upsert: true, returnDocument: 'after' }
    );
    if (result === null) {
      throw new Error(`failed to initialize activation environment ${decoded.environment}`);
    }
    return decodeEnvironmentActivationState(result.state);
  }

  async read(environment: string): Promise<EnvironmentActivationState> {
    const document = await this.states.findOne({ _id: environment });
    if (document === null) {
      throw new Error(`unknown activation environment ${environment}`);
    }
    return decodeEnvironmentActivationState(document.state);
  }

  async prepare(
    request: AssemblyActivationRequest,
    participantReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return this.transition(request.environment, (current) => {
      const next = prepareActivationState(current, request, participantReplicaIds);
      return {
        next,
        audit: {
          activationId: request.activationId,
          transition: 'prepare',
          assemblyIdentity: request.assembly.assemblyIdentity,
          configSnapshotId: request.configSnapshot.snapshotId,
          participantReplicaIds: next.pending?.participantReplicaIds ?? [],
          connectedReplicaIds: [],
          preparedReplicaIds: []
        }
      };
    });
  }

  async abort(
    environment: string,
    pending: PendingActivation
  ): Promise<EnvironmentActivationState> {
    return this.transition(environment, (current) => ({
      next: abortActivationState(current, environment, pending),
      audit: {
        activationId: pending.activationId,
        transition: 'abort',
        assemblyIdentity: pending.assembly.assemblyIdentity,
        configSnapshotId: pending.configSnapshot.snapshotId,
        participantReplicaIds: pending.participantReplicaIds,
        connectedReplicaIds: [],
        preparedReplicaIds: []
      }
    }));
  }

  async commit(
    environment: string,
    pending: PendingActivation,
    connectedReplicaIds: readonly string[],
    preparedReplicaIds: readonly string[]
  ): Promise<EnvironmentActivationState> {
    return this.transition(environment, (current) => ({
      next: commitActivationState(
        current,
        environment,
        pending,
        connectedReplicaIds,
        preparedReplicaIds
      ),
      audit: {
        activationId: pending.activationId,
        transition: 'commit',
        assemblyIdentity: pending.assembly.assemblyIdentity,
        configSnapshotId: pending.configSnapshot.snapshotId,
        participantReplicaIds: pending.participantReplicaIds,
        connectedReplicaIds: canonicalIds(connectedReplicaIds),
        preparedReplicaIds: canonicalIds(preparedReplicaIds)
      }
    }));
  }

  private async transition(
    environment: string,
    reduce: (current: EnvironmentActivationState) => Readonly<{
      next: EnvironmentActivationState;
      audit: Omit<
        ActivationAuditDocument,
        '_id' | 'schemaVersion' | 'environment' | 'beforeGeneration' |
        'afterGeneration' | 'recordedAt'
      >;
    }>
  ): Promise<EnvironmentActivationState> {
    const session = this.client.startSession();
    try {
      let result: EnvironmentActivationState | undefined;
      await session.withTransaction(async () => {
        const document = await this.states.findOne(
          { _id: environment },
          { session }
        );
        if (document === null) {
          throw new Error(`unknown activation environment ${environment}`);
        }
        const current = decodeEnvironmentActivationState(document.state);
        const transition = reduce(current);
        const next = decodeEnvironmentActivationState(transition.next);
        result = next;
        if (sameState(current, next)) {
          return;
        }
        const replacement = await this.states.updateOne(
          { _id: environment, revision: document.revision },
          { $set: { revision: document.revision + 1, state: next } },
          { session }
        );
        if (replacement.matchedCount !== 1) {
          throw new Error('activation state CAS conflict');
        }
        await this.audit.insertOne(
          {
            _id: auditIdentity(
              environment,
              transition.audit.activationId,
              transition.audit.transition
            ),
            schemaVersion: 'skiff-router-activation-audit-v2',
            environment,
            ...transition.audit,
            beforeGeneration: current.committed.generation,
            afterGeneration: next.committed.generation,
            recordedAt: this.now()
          },
          { session }
        );
      }, {
        readConcern: { level: 'snapshot' },
        writeConcern: { w: 'majority' }
      });
      if (result === undefined) {
        throw new Error('activation transaction completed without a state result');
      }
      return result;
    } finally {
      await session.endSession();
    }
  }
}

export async function connectMongoAssemblyActivationStateStore(input: {
  mongoUrl: string;
  clientOptions?: MongoClientOptions;
  storeOptions?: MongoAssemblyActivationStateStoreOptions;
}): Promise<Readonly<{
  client: MongoClient;
  store: MongoAssemblyActivationStateStore;
}>> {
  const client = new MongoClient(input.mongoUrl, input.clientOptions);
  try {
    await client.connect();
    return {
      client,
      store: new MongoAssemblyActivationStateStore(
        client.db(),
        input.storeOptions
      )
    };
  } catch (error) {
    await client.close().catch(() => undefined);
    throw error;
  }
}

function sameState(
  left: EnvironmentActivationState,
  right: EnvironmentActivationState
): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function canonicalIds(values: readonly string[]): readonly string[] {
  return [...values].sort((left, right) =>
    Buffer.compare(Buffer.from(left), Buffer.from(right))
  );
}

function auditIdentity(
  environment: string,
  activationId: string,
  transition: ActivationAuditDocument['transition']
): string {
  return `${environment}\u0000${activationId}\u0000${transition}`;
}
