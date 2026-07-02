export const TELEMETRY_PROTOCOL = 'skiff-telemetry-v1' as const;

export const TELEMETRY_TOPICS = ['log', 'trace', 'metric', 'health', 'debug'] as const;

export const TELEMETRY_SOURCES = [
  'gateway',
  'router',
  'runtime',
  'provider',
  'test'
] as const;

export const TELEMETRY_REGISTER_SOURCES = ['router', 'runtime', 'test'] as const;

export const TELEMETRY_LEVELS = ['debug', 'info', 'warn', 'error'] as const;

export type TelemetryTopic = (typeof TELEMETRY_TOPICS)[number];
export type TelemetrySource = (typeof TELEMETRY_SOURCES)[number];
export type TelemetryRegisterSource = (typeof TELEMETRY_REGISTER_SOURCES)[number];
export type TelemetryLevel = (typeof TELEMETRY_LEVELS)[number];

export interface TelemetryRegisterEnvelope {
  type: 'telemetry.register';
  protocol: typeof TELEMETRY_PROTOCOL;
  producerId: string;
  source: TelemetryRegisterSource;
  runtimeId?: string;
  topics: TelemetryTopic[];
}

export interface TelemetryEvent {
  topic: TelemetryTopic;
  ts: string;
  source: TelemetrySource;
  serviceId?: string;
  revisionId?: string;
  buildId?: string;
  activationIdentity?: string;
  runtimeId?: string;
  providerId?: string;
  providerRevision?: string;
  providerCapability?: string;
  providerTarget?: string;
  requestId?: string;
  clientRequestId?: string;
  traceId?: string;
  spanId?: string;
  parentSpanId?: string;
  target?: string;
  level?: TelemetryLevel;
  name?: string;
  message?: string;
  attrs?: Record<string, unknown>;
  error?: Record<string, unknown>;
  durationMs?: number;
  dropped?: Record<string, unknown>;
}

export interface TelemetryBatchEnvelope {
  type: 'telemetry.batch';
  producerId: string;
  seq: number;
  events: TelemetryEvent[];
}

export type TelemetryEnvelope = TelemetryRegisterEnvelope | TelemetryBatchEnvelope;

export type ValidationResult<T> =
  | {
      ok: true;
      value: T;
    }
  | {
      ok: false;
      error: string;
    };

const eventStringFields = [
  'serviceId',
  'revisionId',
  'buildId',
  'activationIdentity',
  'runtimeId',
  'providerId',
  'providerRevision',
  'providerCapability',
  'providerTarget',
  'requestId',
  'clientRequestId',
  'traceId',
  'spanId',
  'parentSpanId',
  'target',
  'name',
  'message'
] as const;

const eventObjectFields = ['attrs', 'error', 'dropped'] as const;

const allowedEventFields = new Set<string>([
  'topic',
  'ts',
  'source',
  ...eventStringFields,
  'level',
  ...eventObjectFields,
  'durationMs'
]);

const rfc3339TimestampPattern =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/;

export function decodeTelemetryEnvelope(data: Buffer | ArrayBuffer | Buffer[] | string): unknown {
  const text = Array.isArray(data)
    ? Buffer.concat(data).toString('utf8')
    : typeof data === 'string'
      ? data
      : data instanceof ArrayBuffer
        ? Buffer.from(new Uint8Array(data)).toString('utf8')
        : Buffer.from(data).toString('utf8');
  return JSON.parse(text);
}

export function validateTelemetryRegister(
  value: unknown
): ValidationResult<TelemetryRegisterEnvelope> {
  if (!isRecord(value)) {
    return invalid('telemetry.register envelope must be an object');
  }
  const unknownField = firstUnknownField(value, [
    'type',
    'protocol',
    'producerId',
    'source',
    'runtimeId',
    'topics'
  ]);
  if (unknownField) {
    return invalid(`telemetry.register.${unknownField} is not supported`);
  }
  if (value.type !== 'telemetry.register') {
    return invalid('telemetry.register type must be telemetry.register');
  }
  if (value.protocol !== TELEMETRY_PROTOCOL) {
    return invalid(`telemetry.register protocol must be ${TELEMETRY_PROTOCOL}`);
  }
  const producerId = readNonEmptyString(value.producerId, 'telemetry.register producerId');
  if (producerId) {
    return invalid(producerId);
  }
  if (!isOneOf(value.source, TELEMETRY_REGISTER_SOURCES)) {
    return invalid('telemetry.register source must be router, runtime or test');
  }
  if (value.runtimeId !== undefined) {
    const runtimeId = readNonEmptyString(value.runtimeId, 'telemetry.register runtimeId');
    if (runtimeId) {
      return invalid(runtimeId);
    }
  }
  const topics = validateTopicList(value.topics, 'telemetry.register topics');
  if (!topics.ok) {
    return topics;
  }
  return { ok: true, value: value as unknown as TelemetryRegisterEnvelope };
}

export function validateTelemetryBatch(
  value: unknown,
  registeredTopics?: readonly TelemetryTopic[]
): ValidationResult<TelemetryBatchEnvelope> {
  if (!isRecord(value)) {
    return invalid('telemetry.batch envelope must be an object');
  }
  const unknownField = firstUnknownField(value, ['type', 'producerId', 'seq', 'events']);
  if (unknownField) {
    return invalid(`telemetry.batch.${unknownField} is not supported`);
  }
  if (value.type !== 'telemetry.batch') {
    return invalid('telemetry.batch type must be telemetry.batch');
  }
  const producerId = readNonEmptyString(value.producerId, 'telemetry.batch producerId');
  if (producerId) {
    return invalid(producerId);
  }
  if (typeof value.seq !== 'number' || !Number.isSafeInteger(value.seq) || value.seq <= 0) {
    return invalid('telemetry.batch seq must be a positive integer');
  }
  if (!Array.isArray(value.events) || value.events.length === 0) {
    return invalid('telemetry.batch events must be a non-empty array');
  }
  const topicSet = registeredTopics ? new Set<TelemetryTopic>(registeredTopics) : undefined;
  for (let index = 0; index < value.events.length; index += 1) {
    const eventResult = validateTelemetryEvent(value.events[index], `telemetry.batch events[${index}]`);
    if (!eventResult.ok) {
      return eventResult;
    }
    if (topicSet && !topicSet.has(eventResult.value.topic)) {
      return invalid(
        `telemetry.batch events[${index}].topic must be included in telemetry.register topics`
      );
    }
  }
  return { ok: true, value: value as unknown as TelemetryBatchEnvelope };
}

export function validateTelemetryEvent(
  value: unknown,
  label = 'telemetry.event'
): ValidationResult<TelemetryEvent> {
  if (!isRecord(value)) {
    return invalid(`${label} must be an object`);
  }
  for (const key of Object.keys(value)) {
    if (!allowedEventFields.has(key)) {
      return invalid(`${label}.${key} is not supported`);
    }
  }
  if (!isOneOf(value.topic, TELEMETRY_TOPICS)) {
    return invalid(`${label}.topic must be one of ${TELEMETRY_TOPICS.join(', ')}`);
  }
  if (typeof value.ts !== 'string' || !isValidTimestamp(value.ts)) {
    return invalid(`${label}.ts must be an RFC3339 timestamp string`);
  }
  if (!isOneOf(value.source, TELEMETRY_SOURCES)) {
    return invalid(`${label}.source must be one of ${TELEMETRY_SOURCES.join(', ')}`);
  }
  for (const field of eventStringFields) {
    if (value[field] !== undefined && typeof value[field] !== 'string') {
      return invalid(`${label}.${field} must be a string`);
    }
  }
  if (value.level !== undefined && !isOneOf(value.level, TELEMETRY_LEVELS)) {
    return invalid(`${label}.level must be one of ${TELEMETRY_LEVELS.join(', ')}`);
  }
  for (const field of eventObjectFields) {
    if (value[field] !== undefined && !isRecord(value[field])) {
      return invalid(`${label}.${field} must be a JSON object`);
    }
  }
  if (
    value.durationMs !== undefined &&
    (typeof value.durationMs !== 'number' || !Number.isFinite(value.durationMs) || value.durationMs < 0)
  ) {
    return invalid(`${label}.durationMs must be a non-negative finite number`);
  }
  if (value.topic === 'log' && value.level === undefined) {
    return invalid(`${label}.level is required when topic is log`);
  }
  if (value.topic === 'trace' && value.name === undefined && value.target === undefined) {
    return invalid(`${label}.name or ${label}.target is required when topic is trace`);
  }
  return { ok: true, value: value as unknown as TelemetryEvent };
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function isTelemetryLevel(value: unknown): value is TelemetryLevel {
  return isOneOf(value, TELEMETRY_LEVELS);
}

export function isValidTimestamp(value: string): boolean {
  if (!rfc3339TimestampPattern.test(value)) {
    return false;
  }
  const time = Date.parse(value);
  return Number.isFinite(time);
}

function validateTopicList(
  value: unknown,
  label: string
): ValidationResult<readonly TelemetryTopic[]> {
  if (!Array.isArray(value) || value.length === 0) {
    return invalid(`${label} must be a non-empty array`);
  }
  const seen = new Set<TelemetryTopic>();
  for (const item of value) {
    if (!isOneOf(item, TELEMETRY_TOPICS)) {
      return invalid(`${label} items must be one of ${TELEMETRY_TOPICS.join(', ')}`);
    }
    if (seen.has(item)) {
      return invalid(`${label} must not contain duplicate topics`);
    }
    seen.add(item);
  }
  return { ok: true, value };
}

function readNonEmptyString(value: unknown, label: string): string | null {
  return typeof value === 'string' && value.trim().length > 0
    ? null
    : `${label} must be a non-empty string`;
}

function firstUnknownField(value: Record<string, unknown>, allowed: readonly string[]): string | null {
  const allowedSet = new Set(allowed);
  return Object.keys(value).find((key) => !allowedSet.has(key)) ?? null;
}

function isOneOf<const TValue extends string>(
  value: unknown,
  options: readonly TValue[]
): value is TValue {
  return typeof value === 'string' && (options as readonly string[]).includes(value);
}

function invalid(error: string): ValidationResult<never> {
  return { ok: false, error };
}
