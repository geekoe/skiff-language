import {
  MongoClient,
  type Collection,
  type CreateIndexesOptions,
  type Db,
  type Filter,
  type IndexSpecification
} from 'mongodb';

import type {
  TelemetryBatchEnvelope,
  TelemetryEvent,
  TelemetryLevel
} from './protocol.js';
import { redactTelemetryEvent } from './redaction.js';

export interface LogEventDocument extends TelemetryEvent {
  receivedAt: Date;
  producerId: string;
  seq: number;
  eventIndex: number;
}

export interface LogQuery {
  serviceId?: string;
  since?: string;
  traceId?: string;
  requestId?: string;
  target?: string;
  level?: TelemetryLevel;
  limit?: number;
}

export interface TraceQuery {
  serviceId?: string;
  since?: string;
  traceId?: string;
  requestId?: string;
  target?: string;
  level?: TelemetryLevel;
  limit?: number;
}

export interface InsertBatchResult {
  inserted: number;
  duplicate: boolean;
}

export interface TelemetryStoreHealth {
  store: 'mongo' | 'memory';
  insertedEvents: number;
  duplicateBatches: number;
}

export interface MongoTelemetryIndexSpec {
  keys: IndexSpecification;
  options: CreateIndexesOptions & {
    name: string;
  };
}

export interface TelemetryStore {
  init(): Promise<void>;
  close(): Promise<void>;
  insertBatch(batch: TelemetryBatchEnvelope): Promise<InsertBatchResult>;
  queryLogs(query: LogQuery): Promise<LogEventDocument[]>;
  queryTrace(traceId: string): Promise<LogEventDocument[]>;
  queryTraces(query: TraceQuery): Promise<LogEventDocument[]>;
  health(): Promise<TelemetryStoreHealth>;
}

export interface MongoTelemetryStoreOptions {
  mongoUrl: string;
  databaseName?: string;
  ttlDays?: number;
}

const DEFAULT_DATABASE_NAME = 'skiff_telemetry';
const DEFAULT_TTL_DAYS = 7;
const DEFAULT_QUERY_LIMIT = 100;
const MAX_QUERY_LIMIT = 1000;

export class MongoTelemetryStore implements TelemetryStore {
  private readonly client: MongoClient;
  private db: Db | undefined;
  private collection: Collection<LogEventDocument> | undefined;
  private insertedEvents = 0;
  private duplicateBatches = 0;

  constructor(private readonly options: MongoTelemetryStoreOptions) {
    this.client = new MongoClient(options.mongoUrl, {
      connectTimeoutMS: 5000,
      serverSelectionTimeoutMS: 5000,
    });
  }

  async init(): Promise<void> {
    await this.client.connect();
    this.db = this.client.db(this.options.databaseName ?? DEFAULT_DATABASE_NAME);
    this.collection = this.db.collection<LogEventDocument>('log_event');
    await this.createIndexes();
  }

  async close(): Promise<void> {
    await this.client.close();
  }

  async insertBatch(batch: TelemetryBatchEnvelope): Promise<InsertBatchResult> {
    const collection = this.requireCollection();
    const existing = await collection.findOne(
      { producerId: batch.producerId, seq: batch.seq },
      { projection: { _id: 1 } }
    );
    if (existing) {
      this.duplicateBatches += 1;
      return { inserted: 0, duplicate: true };
    }

    const receivedAt = new Date();
    const documents = batch.events.map((event, index) => ({
      ...redactTelemetryEvent(event),
      receivedAt,
      producerId: batch.producerId,
      seq: batch.seq,
      eventIndex: index
    }));
    if (documents.length === 0) {
      return { inserted: 0, duplicate: false };
    }
    const result = await collection.insertMany(documents, { ordered: true });
    this.insertedEvents += result.insertedCount;
    return { inserted: result.insertedCount, duplicate: false };
  }

  async queryLogs(query: LogQuery): Promise<LogEventDocument[]> {
    const collection = this.requireCollection();
    return collection
      .find(buildLogFilter(query))
      .sort({ ts: -1, receivedAt: -1, producerId: 1, seq: -1, eventIndex: -1 })
      .limit(readLimit(query.limit))
      .toArray();
  }

  async queryTrace(traceId: string): Promise<LogEventDocument[]> {
    const collection = this.requireCollection();
    return collection
      .find({ traceId })
      .sort({ ts: 1, receivedAt: 1, producerId: 1, seq: 1, eventIndex: 1 })
      .limit(MAX_QUERY_LIMIT)
      .toArray();
  }

  async queryTraces(query: TraceQuery): Promise<LogEventDocument[]> {
    const collection = this.requireCollection();
    return collection
      .find(buildTraceFilter(query))
      .sort({ ts: 1, receivedAt: 1, producerId: 1, seq: 1, eventIndex: 1 })
      .limit(readLimit(query.limit))
      .toArray();
  }

  async health(): Promise<TelemetryStoreHealth> {
    return {
      store: 'mongo',
      insertedEvents: this.insertedEvents,
      duplicateBatches: this.duplicateBatches
    };
  }

  private async createIndexes(): Promise<void> {
    const collection = this.requireCollection();
    for (const index of mongoTelemetryIndexSpecs(this.options.ttlDays)) {
      await collection.createIndex(index.keys, index.options);
    }
  }

  private requireCollection(): Collection<LogEventDocument> {
    if (!this.collection) {
      throw new Error('telemetry store has not been initialized');
    }
    return this.collection;
  }
}

export class InMemoryTelemetryStore implements TelemetryStore {
  private readonly events: LogEventDocument[] = [];
  private readonly seenBatches = new Set<string>();
  private insertedEvents = 0;
  private duplicateBatches = 0;

  async init(): Promise<void> {}

  async close(): Promise<void> {}

  async insertBatch(batch: TelemetryBatchEnvelope): Promise<InsertBatchResult> {
    const key = `${batch.producerId}\u0000${batch.seq}`;
    if (this.seenBatches.has(key)) {
      this.duplicateBatches += 1;
      return { inserted: 0, duplicate: true };
    }
    this.seenBatches.add(key);
    const receivedAt = new Date();
    const documents = batch.events.map((event, index) => ({
      ...redactTelemetryEvent(event),
      receivedAt,
      producerId: batch.producerId,
      seq: batch.seq,
      eventIndex: index
    }));
    this.events.push(...documents);
    this.insertedEvents += documents.length;
    return { inserted: documents.length, duplicate: false };
  }

  async queryLogs(query: LogQuery): Promise<LogEventDocument[]> {
    return this.events
      .filter((event) => matchesFilter(event, buildLogFilter(query)))
      .sort(sortDesc)
      .slice(0, readLimit(query.limit));
  }

  async queryTrace(traceId: string): Promise<LogEventDocument[]> {
    return this.events
      .filter((event) => event.traceId === traceId)
      .sort(sortAsc)
      .slice(0, MAX_QUERY_LIMIT);
  }

  async queryTraces(query: TraceQuery): Promise<LogEventDocument[]> {
    return this.events
      .filter((event) => matchesFilter(event, buildTraceFilter(query)))
      .sort(sortAsc)
      .slice(0, readLimit(query.limit));
  }

  async health(): Promise<TelemetryStoreHealth> {
    return {
      store: 'memory',
      insertedEvents: this.insertedEvents,
      duplicateBatches: this.duplicateBatches
    };
  }
}

export function telemetryStoreFromEnv(env: NodeJS.ProcessEnv = process.env): TelemetryStore {
  if (
    env.SKIFF_TELEMETRY_IN_MEMORY === '1' ||
    env.SKIFF_TELEMETRY_IN_MEMORY === 'true'
  ) {
    return new InMemoryTelemetryStore();
  }
  const mongoUrl = env.SKIFF_TELEMETRY_MONGO_URL ?? env.MONGO_URL;
  if (!mongoUrl) {
    throw new Error(
      'SKIFF_TELEMETRY_MONGO_URL or MONGO_URL is required unless SKIFF_TELEMETRY_IN_MEMORY=true'
    );
  }
  const ttlDays = readTtlDays(env.SKIFF_TELEMETRY_TTL_DAYS);
  return new MongoTelemetryStore({
    mongoUrl,
    databaseName: env.SKIFF_TELEMETRY_DB ?? DEFAULT_DATABASE_NAME,
    ...(ttlDays !== undefined ? { ttlDays } : {})
  });
}

function buildLogFilter(query: LogQuery): Filter<LogEventDocument> {
  const filter: Filter<LogEventDocument> = { topic: 'log' };
  applyCommonFilter(filter, query);
  if (query.level !== undefined) {
    filter.level = query.level;
  }
  return filter;
}

function buildTraceFilter(query: TraceQuery): Filter<LogEventDocument> {
  const filter: Filter<LogEventDocument> = { traceId: { $exists: true } };
  applyCommonFilter(filter, query);
  if (query.level !== undefined) {
    filter.level = query.level;
  }
  return filter;
}

function applyCommonFilter(
  filter: Filter<LogEventDocument>,
  query: {
    serviceId?: string;
    since?: string;
    traceId?: string;
    requestId?: string;
    target?: string;
  }
): void {
  if (query.serviceId !== undefined) {
    filter.serviceId = query.serviceId;
  }
  if (query.traceId !== undefined) {
    filter.traceId = query.traceId;
  }
  if (query.requestId !== undefined) {
    filter.requestId = query.requestId;
  }
  if (query.target !== undefined) {
    filter.target = query.target;
  }
  if (query.since !== undefined) {
    filter.ts = { $gte: query.since };
  }
}

function matchesFilter(event: LogEventDocument, filter: Filter<LogEventDocument>): boolean {
  for (const [key, expected] of Object.entries(filter)) {
    const actual = event[key as keyof LogEventDocument];
    if (isGteFilter(expected)) {
      if (typeof actual !== 'string' || actual < expected.$gte) {
        return false;
      }
      continue;
    }
    if (isExistsFilter(expected)) {
      if (expected.$exists !== (actual !== undefined)) {
        return false;
      }
      continue;
    }
    if (actual !== expected) {
      return false;
    }
  }
  return true;
}

function isGteFilter(value: unknown): value is { $gte: string } {
  return (
    typeof value === 'object' &&
    value !== null &&
    '$gte' in value &&
    typeof (value as { $gte?: unknown }).$gte === 'string'
  );
}

function isExistsFilter(value: unknown): value is { $exists: boolean } {
  return (
    typeof value === 'object' &&
    value !== null &&
    '$exists' in value &&
    typeof (value as { $exists?: unknown }).$exists === 'boolean'
  );
}

function sortDesc(a: LogEventDocument, b: LogEventDocument): number {
  return (
    b.ts.localeCompare(a.ts) ||
    b.receivedAt.getTime() - a.receivedAt.getTime() ||
    a.producerId.localeCompare(b.producerId) ||
    b.seq - a.seq ||
    b.eventIndex - a.eventIndex
  );
}

function sortAsc(a: LogEventDocument, b: LogEventDocument): number {
  return (
    a.ts.localeCompare(b.ts) ||
    a.receivedAt.getTime() - b.receivedAt.getTime() ||
    a.producerId.localeCompare(b.producerId) ||
    a.seq - b.seq ||
    a.eventIndex - b.eventIndex
  );
}

function readLimit(value: number | undefined): number {
  if (value === undefined || !Number.isSafeInteger(value) || value <= 0) {
    return DEFAULT_QUERY_LIMIT;
  }
  return Math.min(value, MAX_QUERY_LIMIT);
}

function readTtlDays(value: string | undefined): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error('SKIFF_TELEMETRY_TTL_DAYS must be a positive number');
  }
  return parsed;
}

export function mongoTelemetryIndexSpecs(ttlDays = DEFAULT_TTL_DAYS): readonly MongoTelemetryIndexSpec[] {
  const ttlSeconds = Math.max(1, Math.floor(ttlDays * 24 * 60 * 60));
  return [
    { keys: { producerId: 1, seq: 1 }, options: { name: 'batch_dedupe' } },
    { keys: { ts: -1 }, options: { name: 'ts_desc' } },
    { keys: { serviceId: 1, ts: -1 }, options: { name: 'service_ts_desc' } },
    { keys: { traceId: 1, ts: 1 }, options: { name: 'trace_ts_asc' } },
    { keys: { requestId: 1, ts: 1 }, options: { name: 'request_ts_asc' } },
    { keys: { target: 1, ts: -1 }, options: { name: 'target_ts_desc' } },
    { keys: { level: 1, ts: -1 }, options: { name: 'level_ts_desc' } },
    {
      keys: { providerCapability: 1, ts: -1 },
      options: { name: 'provider_capability_ts_desc' }
    },
    {
      keys: { receivedAt: 1 },
      options: { name: 'ttl_receivedAt', expireAfterSeconds: ttlSeconds }
    }
  ];
}
