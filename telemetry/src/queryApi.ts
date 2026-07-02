import type { IncomingMessage, ServerResponse } from 'node:http';

import type {
  LogEventDocument,
  LogQuery,
  TelemetryStore,
  TraceQuery
} from './mongoStore.js';
import { isTelemetryLevel } from './protocol.js';

export interface TelemetryRuntimeStats {
  connectionCount: number;
  acceptedBatches: number;
  rejectedMessages: number;
}

export interface TraceSpanNode {
  spanId: string;
  events: LogEventDocument[];
  children: TraceSpanNode[];
  firstTs: string;
  lastTs: string;
  parentSpanId?: string;
  serviceId?: string;
  target?: string;
  name?: string;
}

export interface TraceView {
  traceId: string;
  events: LogEventDocument[];
  spans: TraceSpanNode[];
  unspannedEvents: LogEventDocument[];
}

export interface QueryApiOptions {
  store: TelemetryStore;
  stats?: TelemetryRuntimeStats;
}

export async function handleQueryRequest(
  request: IncomingMessage,
  response: ServerResponse,
  options: QueryApiOptions
): Promise<boolean> {
  const url = new URL(request.url ?? '/', `http://${request.headers.host ?? '127.0.0.1'}`);
  if (request.method !== 'GET') {
    writeJson(response, 405, { error: 'method not allowed' });
    return true;
  }

  if (url.pathname === '/health') {
    const storeHealth = await options.store.health();
    const connectionCount = options.stats?.connectionCount ?? 0;
    const acceptedBatches = options.stats?.acceptedBatches ?? 0;
    const rejectedMessages = options.stats?.rejectedMessages ?? 0;
    writeJson(response, 200, {
      ok: true,
      ...storeHealth,
      connectionCount,
      connections: connectionCount,
      acceptedBatches,
      rejectedMessages,
      rejectedCount: rejectedMessages,
      writeCount: storeHealth.insertedEvents,
      storeType: storeHealth.store
    });
    return true;
  }

  if (url.pathname === '/logs') {
    const query = readLogQuery(url.searchParams);
    if (!query.ok) {
      writeJson(response, 400, { error: query.error });
      return true;
    }
    writeJson(response, 200, { events: await options.store.queryLogs(query.value) });
    return true;
  }

  if (url.pathname === '/traces') {
    const query = readTraceQuery(url.searchParams);
    if (!query.ok) {
      writeJson(response, 400, { error: query.error });
      return true;
    }
    writeJson(response, 200, { events: await options.store.queryTraces(query.value) });
    return true;
  }

  const traceMatch = /^\/traces\/([^/]+)$/.exec(url.pathname);
  if (traceMatch) {
    const traceId = decodeURIComponent(traceMatch[1]!);
    if (traceId.length === 0) {
      writeJson(response, 400, { error: 'traceId must be non-empty' });
      return true;
    }
    writeJson(response, 200, buildTraceView(traceId, await options.store.queryTrace(traceId)));
    return true;
  }

  return false;
}

function readLogQuery(
  searchParams: URLSearchParams
):
  | {
      ok: true;
      value: LogQuery;
    }
  | {
      ok: false;
      error: string;
    } {
  const common = readCommonQuery(searchParams);
  if (!common.ok) {
    return common;
  }
  const level = searchParams.get('level') ?? undefined;
  if (level !== undefined && !isTelemetryLevel(level)) {
    return { ok: false, error: 'level must be debug, info, warn, or error' };
  }
  return {
    ok: true,
    value: {
      ...common.value,
      ...(level !== undefined ? { level } : {})
    }
  };
}

function readTraceQuery(
  searchParams: URLSearchParams
):
  | {
      ok: true;
      value: TraceQuery;
    }
  | {
      ok: false;
      error: string;
    } {
  const common = readCommonQuery(searchParams);
  if (!common.ok) {
    return common;
  }
  const level = searchParams.get('level') ?? undefined;
  if (level !== undefined && !isTelemetryLevel(level)) {
    return { ok: false, error: 'level must be debug, info, warn, or error' };
  }
  return {
    ok: true,
    value: {
      ...common.value,
      ...(level !== undefined ? { level } : {})
    }
  };
}

function readCommonQuery(
  searchParams: URLSearchParams
):
  | {
      ok: true;
      value: Omit<LogQuery, 'level'>;
    }
  | {
      ok: false;
      error: string;
    } {
  const query: Omit<LogQuery, 'level'> = {};
  const serviceId = searchParams.get('serviceId') ?? searchParams.get('service') ?? undefined;
  if (serviceId !== undefined) {
    query.serviceId = serviceId;
  }
  const traceId = searchParams.get('traceId') ?? searchParams.get('trace') ?? undefined;
  if (traceId !== undefined) {
    query.traceId = traceId;
  }
  const requestId = searchParams.get('requestId') ?? searchParams.get('request') ?? undefined;
  if (requestId !== undefined) {
    query.requestId = requestId;
  }
  const target = searchParams.get('target') ?? undefined;
  if (target !== undefined) {
    query.target = target;
  }
  const since = searchParams.get('since') ?? undefined;
  if (since !== undefined) {
    const parsed = parseSince(since);
    if (!parsed) {
      return { ok: false, error: 'since must be RFC3339, milliseconds epoch, or a duration like 15m' };
    }
    query.since = parsed;
  }
  const limit = searchParams.get('limit') ?? undefined;
  if (limit !== undefined) {
    const parsed = Number(limit);
    if (!Number.isSafeInteger(parsed) || parsed <= 0) {
      return { ok: false, error: 'limit must be a positive integer' };
    }
    query.limit = parsed;
  }
  return { ok: true, value: query };
}

export function parseSince(value: string, now = Date.now()): string | null {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return null;
  }
  const duration = /^(\d+)(ms|s|m|h|d)$/.exec(trimmed);
  if (duration) {
    const amount = Number(duration[1]);
    const unit = duration[2];
    const multiplier =
      unit === 'ms'
        ? 1
        : unit === 's'
          ? 1000
          : unit === 'm'
            ? 60 * 1000
            : unit === 'h'
              ? 60 * 60 * 1000
              : 24 * 60 * 60 * 1000;
    return timestampToIso(now - amount * multiplier);
  }
  const asNumber = Number(trimmed);
  if (Number.isFinite(asNumber)) {
    return timestampToIso(asNumber);
  }
  const parsed = Date.parse(trimmed);
  return Number.isFinite(parsed) ? timestampToIso(parsed) : null;
}

export function buildTraceView(traceId: string, events: LogEventDocument[]): TraceView {
  const spanMap = new Map<string, TraceSpanNode>();
  const unspannedEvents: LogEventDocument[] = [];

  for (const event of events) {
    if (event.spanId === undefined || event.spanId.length === 0) {
      unspannedEvents.push(event);
      continue;
    }

    let span = spanMap.get(event.spanId);
    if (!span) {
      span = {
        spanId: event.spanId,
        events: [],
        children: [],
        firstTs: event.ts,
        lastTs: event.ts
      };
      spanMap.set(event.spanId, span);
    }

    span.events.push(event);
    if (event.ts < span.firstTs) {
      span.firstTs = event.ts;
    }
    if (event.ts > span.lastTs) {
      span.lastTs = event.ts;
    }
    if (span.parentSpanId === undefined && event.parentSpanId !== undefined) {
      span.parentSpanId = event.parentSpanId;
    }
    if (span.serviceId === undefined && event.serviceId !== undefined) {
      span.serviceId = event.serviceId;
    }
    if (span.target === undefined && event.target !== undefined) {
      span.target = event.target;
    }
    if (span.name === undefined && event.name !== undefined) {
      span.name = event.name;
    }
  }

  const spans = [...spanMap.values()].sort(compareSpanNodes);
  const roots: TraceSpanNode[] = [];
  for (const span of spans) {
    if (
      span.parentSpanId !== undefined &&
      span.parentSpanId !== span.spanId &&
      spanMap.has(span.parentSpanId)
    ) {
      spanMap.get(span.parentSpanId)!.children.push(span);
      continue;
    }
    roots.push(span);
  }
  sortSpanTree(roots);

  return {
    traceId,
    events,
    spans: roots,
    unspannedEvents
  };
}

function sortSpanTree(spans: TraceSpanNode[]): void {
  spans.sort(compareSpanNodes);
  for (const span of spans) {
    sortSpanTree(span.children);
  }
}

function compareSpanNodes(a: TraceSpanNode, b: TraceSpanNode): number {
  return a.firstTs.localeCompare(b.firstTs) || a.spanId.localeCompare(b.spanId);
}

function timestampToIso(timestamp: number): string | null {
  const date = new Date(timestamp);
  return Number.isFinite(date.getTime()) ? date.toISOString() : null;
}

function writeJson(response: ServerResponse, statusCode: number, payload: unknown): void {
  response.statusCode = statusCode;
  response.setHeader('content-type', 'application/json; charset=utf-8');
  response.end(`${JSON.stringify(payload)}\n`);
}
