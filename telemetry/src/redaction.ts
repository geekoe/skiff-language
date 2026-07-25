import type { TelemetryEvent } from './protocol.js';
import { isRecord } from './protocol.js';

const REDACTED = '[REDACTED]';
const TRUNCATED = '[TRUNCATED]';
const DEFAULT_MAX_DEPTH = 12;
const DEFAULT_MAX_STRING_LENGTH = 4096;
const DEFAULT_MAX_ARRAY_ITEMS = 50;
const DEFAULT_MAX_OBJECT_KEYS = 100;

const sensitiveKeyPattern =
  /(?:^|[_\-.])(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|authorization|cookie|set[_-]?cookie|private[_-]?key|mongo[_-]?url)(?:$|[_\-.])/i;

const sensitiveValuePatterns = [
  /(?:^|[^a-z0-9])secret(?:$|[^a-z0-9])/i,
  /\b(?:bearer|basic)\s+\S+/i,
  /\b(?:password|passwd|pwd|token|api[_-]?key|access[_-]?key|authorization|cookie|private[_-]?key)\s*[:=]\s*\S+/i,
  /^[a-z][a-z0-9+.-]*:\/\/[^/\s:@]+:[^@\s/]+@/i
] as const;

export interface RedactionOptions {
  maxDepth?: number;
  maxStringLength?: number;
  maxArrayItems?: number;
  maxObjectKeys?: number;
}

export function redactTelemetryEvent(
  event: TelemetryEvent,
  options: RedactionOptions = {}
): TelemetryEvent {
  return {
    ...event,
    ...(event.message !== undefined
      ? { message: redactTelemetryString(event.message, options) }
      : {}),
    ...(event.attrs !== undefined
      ? { attrs: redactJsonObject(event.attrs, options) }
      : {}),
    ...(event.error !== undefined
      ? { error: redactJsonObject(event.error, options) }
      : {}),
    ...(event.dropped !== undefined
      ? { dropped: redactJsonObject(event.dropped, options) }
      : {})
  };
}

export function redactJsonObject(
  value: Record<string, unknown>,
  options: RedactionOptions = {}
): Record<string, unknown> {
  const redacted = redactJsonValue(value, options, 0, '');
  return isRecord(redacted) ? redacted : {};
}

export function redactJsonValue(
  value: unknown,
  options: RedactionOptions = {},
  depth = 0,
  key = ''
): unknown {
  if (isSensitiveKey(key)) {
    return REDACTED;
  }

  const maxDepth = options.maxDepth ?? DEFAULT_MAX_DEPTH;
  if (depth > maxDepth) {
    return TRUNCATED;
  }

  if (typeof value === 'string') {
    if (isSensitiveValue(value)) {
      return REDACTED;
    }
    const maxLength = options.maxStringLength ?? DEFAULT_MAX_STRING_LENGTH;
    return value.length > maxLength ? `${value.slice(0, maxLength)}${TRUNCATED}` : value;
  }

  if (
    value === null ||
    typeof value === 'number' ||
    typeof value === 'boolean'
  ) {
    return value;
  }

  if (Array.isArray(value)) {
    const maxItems = options.maxArrayItems ?? DEFAULT_MAX_ARRAY_ITEMS;
    return value
      .slice(0, maxItems)
      .map((item) => redactJsonValue(item, options, depth + 1));
  }

  if (isRecord(value)) {
    const output: Record<string, unknown> = {};
    const maxKeys = options.maxObjectKeys ?? DEFAULT_MAX_OBJECT_KEYS;
    for (const [entryKey, entryValue] of Object.entries(value).slice(0, maxKeys)) {
      output[entryKey] = redactJsonValue(entryValue, options, depth + 1, entryKey);
    }
    return output;
  }

  return String(value);
}

function redactTelemetryString(value: string, options: RedactionOptions): string {
  const redacted = redactJsonValue(value, options);
  return typeof redacted === 'string' ? redacted : TRUNCATED;
}

function isSensitiveKey(key: string): boolean {
  if (key.length === 0) {
    return false;
  }
  const normalized = key.replace(/([a-z0-9])([A-Z])/g, '$1_$2');
  return sensitiveKeyPattern.test(normalized);
}

function isSensitiveValue(value: string): boolean {
  return sensitiveValuePatterns.some((pattern) => pattern.test(value));
}
