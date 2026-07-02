#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';

const DEFAULT_TELEMETRY_URL = 'http://127.0.0.1:4002';

interface CliOptions {
  command: 'logs' | 'traces';
  json: boolean;
  telemetryUrl: string;
  since?: string;
  service?: string;
  trace?: string;
  request?: string;
  target?: string;
  level?: string;
}

async function main(): Promise<void> {
  const options = readCliOptions();
  const payload = await fetchTelemetry(options);
  if (options.json) {
    console.log(JSON.stringify(payload, null, 2));
    return;
  }
  if (options.command === 'logs') {
    printLogs(payload);
    return;
  }
  if (options.trace !== undefined) {
    printTraceTree(payload);
    return;
  }
  printTraceSummaries(payload);
}

function readCliOptions(): CliOptions {
  const args = parseArgs({
    allowPositionals: true,
    options: {
      json: { type: 'boolean' },
      since: { type: 'string' },
      service: { type: 'string' },
      trace: { type: 'string' },
      request: { type: 'string' },
      target: { type: 'string' },
      level: { type: 'string' },
      url: { type: 'string' },
      help: { type: 'boolean', short: 'h' }
    }
  });
  if (args.values.help) {
    printHelp();
    process.exit(0);
  }
  const command = args.positionals[0];
  if (command !== 'logs' && command !== 'traces') {
    printHelp();
    throw new Error('expected command: logs or traces');
  }
  return {
    command,
    json: args.values.json ?? false,
    telemetryUrl:
      args.values.url ??
      process.env.SKIFF_TELEMETRY_URL ??
      DEFAULT_TELEMETRY_URL,
    ...(args.values.since !== undefined ? { since: args.values.since } : {}),
    ...(args.values.service !== undefined ? { service: args.values.service } : {}),
    ...(args.values.trace !== undefined ? { trace: args.values.trace } : {}),
    ...(args.values.request !== undefined ? { request: args.values.request } : {}),
    ...(args.values.target !== undefined ? { target: args.values.target } : {}),
    ...(args.values.level !== undefined ? { level: args.values.level } : {})
  };
}

async function fetchTelemetry(options: CliOptions): Promise<unknown> {
  const url = buildTelemetryUrl(options);
  const response = await fetch(url);
  const text = await response.text();
  const payload = text.length > 0 ? JSON.parse(text) : {};
  if (!response.ok) {
    const message =
      typeof payload === 'object' &&
      payload !== null &&
      'error' in payload &&
      typeof (payload as { error?: unknown }).error === 'string'
        ? (payload as { error: string }).error
        : `telemetry request failed with HTTP ${response.status}`;
    throw new Error(message);
  }
  return payload;
}

function buildTelemetryUrl(options: CliOptions): URL {
  const base = options.telemetryUrl.endsWith('/')
    ? options.telemetryUrl.slice(0, -1)
    : options.telemetryUrl;
  if (options.command === 'traces' && options.trace !== undefined) {
    return new URL(`/traces/${encodeURIComponent(options.trace)}`, base);
  }
  const url = new URL(options.command === 'logs' ? '/logs' : '/traces', base);
  append(url, 'since', options.since);
  append(url, 'serviceId', options.service);
  append(url, 'traceId', options.trace);
  append(url, 'requestId', options.request);
  append(url, 'target', options.target);
  append(url, 'level', options.level);
  return url;
}

function printLogs(payload: unknown): void {
  const events = readEvents(payload);
  for (const event of events) {
    const fields = [
      readString(event.ts),
      readString(event.level).toUpperCase().padEnd(5),
      readString(event.serviceId),
      readString(event.target),
      readString(event.message)
    ].filter((value) => value.length > 0);
    console.log(fields.join(' '));
  }
}

function printTraceSummaries(payload: unknown): void {
  const events = readEvents(payload);
  const summaries = summarizeTraces(events);
  for (const summary of summaries) {
    const fields = [
      summary.lastTs,
      summary.traceId,
      summary.serviceId,
      summary.target,
      `${summary.eventCount} events`
    ].filter((value) => value.length > 0);
    console.log(fields.join(' '));
  }
}

function printTraceTree(payload: unknown): void {
  const traceId = readPayloadString(payload, 'traceId');
  if (traceId.length > 0) {
    console.log(`trace ${traceId}`);
  }
  const spans = readSpans(payload);
  if (spans.length > 0) {
    for (const span of spans) {
      printSpan(span, 0);
    }
  } else {
    for (const event of readEvents(payload)) {
      console.log(`- ${formatEventSummary(event)}`);
    }
  }

  const unspannedEvents = readRecordArray(payload, 'unspannedEvents');
  for (const event of unspannedEvents) {
    console.log(`- ${formatEventSummary(event)}`);
  }
}

function printSpan(span: Record<string, unknown>, depth: number): void {
  const indent = '  '.repeat(depth);
  const events = readRecordArray(span, 'events');
  const event = events.find((item) => readString(item.name).length > 0) ?? events[0];
  const fields = [
    readString(span.spanId),
    readString(span.parentSpanId).length > 0 ? `parent=${readString(span.parentSpanId)}` : '',
    event ? readString(event.name) || readString(event.target) : '',
    event && typeof event.durationMs === 'number' ? `${String(event.durationMs)}ms` : '',
    `${events.length} events`
  ].filter((value) => value.length > 0);
  console.log(`${indent}- ${fields.join(' ')}`);
  for (const child of readRecordArray(span, 'children')) {
    printSpan(child, depth + 1);
  }
}

interface TraceSummary {
  traceId: string;
  firstTs: string;
  lastTs: string;
  eventCount: number;
  serviceId: string;
  target: string;
}

function summarizeTraces(events: Record<string, unknown>[]): TraceSummary[] {
  const byTrace = new Map<string, TraceSummary>();
  for (const event of events) {
    const traceId = readString(event.traceId);
    if (traceId.length === 0) {
      continue;
    }
    const ts = readString(event.ts);
    const current = byTrace.get(traceId);
    if (!current) {
      byTrace.set(traceId, {
        traceId,
        firstTs: ts,
        lastTs: ts,
        eventCount: 1,
        serviceId: readString(event.serviceId),
        target: readString(event.target)
      });
      continue;
    }
    current.eventCount += 1;
    if (ts.length > 0 && (current.firstTs.length === 0 || ts < current.firstTs)) {
      current.firstTs = ts;
    }
    if (ts.length > 0 && (current.lastTs.length === 0 || ts > current.lastTs)) {
      current.lastTs = ts;
    }
    if (current.serviceId.length === 0) {
      current.serviceId = readString(event.serviceId);
    }
    if (current.target.length === 0) {
      current.target = readString(event.target);
    }
  }
  return [...byTrace.values()].sort((a, b) => b.lastTs.localeCompare(a.lastTs));
}

function formatEventSummary(event: Record<string, unknown>): string {
  const fields = [
    readString(event.ts),
    readString(event.topic),
    readString(event.spanId),
    readString(event.name) || readString(event.target) || readString(event.message),
    typeof event.durationMs === 'number' ? `${String(event.durationMs)}ms` : ''
  ].filter((value) => value.length > 0);
  return fields.join(' ');
}

function readEvents(payload: unknown): Record<string, unknown>[] {
  if (
    typeof payload === 'object' &&
    payload !== null &&
    'events' in payload &&
    Array.isArray((payload as { events?: unknown }).events)
  ) {
    return (payload as { events: Record<string, unknown>[] }).events;
  }
  return [];
}

function readSpans(payload: unknown): Record<string, unknown>[] {
  return readRecordArray(payload, 'spans');
}

function readRecordArray(value: unknown, field: string): Record<string, unknown>[] {
  if (typeof value !== 'object' || value === null || !(field in value)) {
    return [];
  }
  const array = (value as Record<string, unknown>)[field];
  return Array.isArray(array) ? array.filter(isRecord) : [];
}

function readPayloadString(payload: unknown, field: string): string {
  if (typeof payload !== 'object' || payload === null || !(field in payload)) {
    return '';
  }
  return readString((payload as Record<string, unknown>)[field]);
}

function readString(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function append(url: URL, key: string, value: string | undefined): void {
  if (value !== undefined) {
    url.searchParams.set(key, value);
  }
}

function printHelp(): void {
  console.log(`Usage:
  skiff logs [--json] [--since 1h] [--service id] [--trace id] [--request id] [--target target] [--level level]
  skiff traces [--json] [--trace id] [--since 15m] [--service id] [--target target] [--level level]

Options:
  --url URL     Telemetry base URL. Defaults to SKIFF_TELEMETRY_URL or ${DEFAULT_TELEMETRY_URL}
`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  });
}
