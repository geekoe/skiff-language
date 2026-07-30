import { describe, expect, it } from 'vitest';
import type { Db } from 'mongodb';

import {
  MongoAssemblyActivationStateStore,
  initialActivationState
} from '../src/index.js';

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;

describe('MongoAssemblyActivationStateStore', () => {
  it('persists CAS state and one derived audit event for idempotent retries', async () => {
    const mongo = fakeMongo();
    const store = new MongoAssemblyActivationStateStore(mongo.database);
    await store.initialize(initialActivationState({
      environment: 'test',
      generation: 0,
      assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'0'.repeat(64)}`
    }));
    const request = {
      schemaVersion: 'skiff-assembly-activation-request-v1' as const,
      environment: 'test',
      activationId: 'activation-a',
      expectedGeneration: 0,
      assembly: { assemblyIdentity: ASSEMBLY }
    };

    const prepared = await store.prepare(request, ['replica-b', 'replica-a']);
    await store.prepare(request, ['replica-a', 'replica-b']);
    const committed = await store.commit(
      'test',
      prepared.pending!,
      ['replica-b', 'replica-a'],
      ['replica-b', 'replica-a']
    );
    await store.commit(
      'test',
      prepared.pending!,
      ['replica-a', 'replica-b'],
      ['replica-a', 'replica-b']
    );

    expect(committed.committed).toEqual({
      generation: 1,
      assembly: { assemblyIdentity: ASSEMBLY }
    });
    expect(mongo.documents('router_assembly_activation_audit')).toHaveLength(2);
    expect(mongo.documents('router_assembly_activation_audit')).toMatchObject([
      {
        transition: 'prepare',
        participantReplicaIds: ['replica-a', 'replica-b']
      },
      {
        transition: 'commit',
        participantReplicaIds: ['replica-a', 'replica-b'],
        connectedReplicaIds: ['replica-a', 'replica-b'],
        preparedReplicaIds: ['replica-a', 'replica-b']
      }
    ]);
  });

  it('retries a transient transaction from its original snapshot without duplicate audit', async () => {
    const mongo = fakeMongo({ retryFirstTransaction: true });
    const store = new MongoAssemblyActivationStateStore(mongo.database);
    await store.initialize(initialActivationState({
      environment: 'test',
      generation: 0,
      assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'0'.repeat(64)}`
    }));

    await store.prepare({
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment: 'test',
      activationId: 'activation-retry',
      expectedGeneration: 0,
      assembly: { assemblyIdentity: ASSEMBLY }
    }, ['replica-a']);

    expect(mongo.transactionAttempts()).toBe(2);
    expect(mongo.documents('router_assembly_activation_audit')).toHaveLength(1);
    await expect(store.read('test')).resolves.toMatchObject({
      pending: { activationId: 'activation-retry' }
    });
  });

  it('rolls state back when the audit append fails', async () => {
    const mongo = fakeMongo({ failAuditInsert: true });
    const store = new MongoAssemblyActivationStateStore(mongo.database);
    const initial = await store.initialize(initialActivationState({
      environment: 'test',
      generation: 0,
      assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'0'.repeat(64)}`
    }));

    await expect(store.prepare({
      schemaVersion: 'skiff-assembly-activation-request-v1',
      environment: 'test',
      activationId: 'activation-failure',
      expectedGeneration: 0,
      assembly: { assemblyIdentity: ASSEMBLY }
    }, ['replica-a'])).rejects.toThrow('injected audit failure');
    await expect(store.read('test')).resolves.toEqual(initial);
  });
});

function fakeMongo(options: {
  retryFirstTransaction?: boolean;
  failAuditInsert?: boolean;
} = {}) {
  const collections = new Map<string, Map<string, Record<string, unknown>>>();
  let attempts = 0;
  const collection = (name: string) => {
    const records = collections.get(name) ?? new Map();
    collections.set(name, records);
    return {
      createIndex: async () => undefined,
      findOne: async (filter: { _id: string }) =>
        structuredClone(records.get(filter._id) ?? null),
      findOneAndUpdate: async (
        filter: { _id: string },
        update: { $setOnInsert: Record<string, unknown> }
      ) => {
        if (!records.has(filter._id)) {
          records.set(filter._id, structuredClone(update.$setOnInsert));
        }
        return structuredClone(records.get(filter._id)!);
      },
      updateOne: async (
        filter: { _id: string; revision: number },
        update: { $set: Record<string, unknown> }
      ) => {
        const current = records.get(filter._id);
        if (current?.revision !== filter.revision) return { matchedCount: 0 };
        records.set(filter._id, { ...current, ...structuredClone(update.$set) });
        return { matchedCount: 1 };
      },
      insertOne: async (document: Record<string, unknown>) => {
        if (options.failAuditInsert && name.endsWith('_audit')) {
          throw new Error('injected audit failure');
        }
        records.set(document._id as string, structuredClone(document));
      }
    };
  };
  const client = {
    startSession: () => ({
      withTransaction: async (callback: () => Promise<void>) => {
        const snapshot = structuredClone(collectionSnapshot(collections));
        attempts += 1;
        try {
          await callback();
        } catch (error) {
          restoreCollections(collections, snapshot);
          throw error;
        }
        if (options.retryFirstTransaction && attempts === 1) {
          restoreCollections(collections, snapshot);
          await callback();
          attempts += 1;
        }
      },
      endSession: async () => undefined
    })
  };
  const database = {
    client,
    collection
  } as unknown as Db;
  return {
    database,
    documents: (name: string) =>
      [...(collections.get(name)?.values() ?? [])],
    transactionAttempts: () => attempts
  };
}

function collectionSnapshot(
  collections: Map<string, Map<string, Record<string, unknown>>>
): [string, [string, Record<string, unknown>][]][] {
  return [...collections].map(([name, records]) => [name, [...records]]);
}

function restoreCollections(
  collections: Map<string, Map<string, Record<string, unknown>>>,
  snapshot: [string, [string, Record<string, unknown>][]][]
): void {
  for (const records of collections.values()) {
    records.clear();
  }
  for (const [name, records] of snapshot) {
    const target = collections.get(name) ?? new Map();
    for (const [id, document] of records) {
      target.set(id, document);
    }
    collections.set(name, target);
  }
}
