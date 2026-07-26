import { isRecord } from "./envelope.js";
import type { RuntimeAssemblyRequestStartFrameHeader } from "./runtimeAssemblyRequest.js";

class RuntimeAssemblyRequestMetadataError extends Error {}

export function validateRuntimeAssemblyRequestMetadata(
  envelope: Record<string, unknown>,
): string | null {
  try {
    validateClientSession(envelope);
    validateDeadline(envelope);
    validateTrace(envelope);
    validateHttpRequest(envelope);
    optionalBoolean(envelope, "testEffectsEnabled");
    return null;
  } catch (error) {
    if (error instanceof RuntimeAssemblyRequestMetadataError) {
      return error.message;
    }
    throw error;
  }
}

export function normalizeRuntimeAssemblyRequestMetadata(
  envelope: Record<string, unknown>,
): RuntimeAssemblyRequestStartFrameHeader {
  return {
    ...envelope,
    testEffectsEnabled: envelope.testEffectsEnabled ?? false,
  } as unknown as RuntimeAssemblyRequestStartFrameHeader;
}

function validateClientSession(envelope: Record<string, unknown>): void {
  if (!has(envelope, "clientSession")) return;
  const session = exactObject(envelope.clientSession, "clientSession", ["id"]);
  requireString(session, "id", "clientSession.id");
}

function validateDeadline(envelope: Record<string, unknown>): void {
  if (!has(envelope, "deadline")) return;
  const deadline = exactObject(envelope.deadline, "deadline", [
    "timeoutMs",
    "expiresAt",
  ]);
  requireSafeUnsignedInteger(deadline.timeoutMs, "deadline.timeoutMs");
  requireString(deadline, "expiresAt", "deadline.expiresAt");
}

function validateTrace(envelope: Record<string, unknown>): void {
  const trace = exactObject(
    envelope.trace,
    "trace",
    ["traceId", "spanId"],
    ["parentSpanId", "sampled"],
  );
  requireString(trace, "traceId", "trace.traceId");
  requireString(trace, "spanId", "trace.spanId");
  optionalString(trace, "parentSpanId", "trace.parentSpanId");
  optionalBoolean(trace, "sampled", "trace.sampled");
}

function validateHttpRequest(envelope: Record<string, unknown>): void {
  const request = exactObject(
    envelope.httpRequest,
    "httpRequest",
    ["method", "url", "path", "query", "headers"],
  );
  requireString(request, "method", "httpRequest.method");
  requireString(request, "url", "httpRequest.url");
  requireString(request, "path", "httpRequest.path");
  validateNameValueArray(request.query, "httpRequest.query");
  validateNameValueArray(request.headers, "httpRequest.headers");
}

function validateNameValueArray(value: unknown, label: string): void {
  if (!Array.isArray(value)) fail(`${label} must be an array`);
  for (const [index, entry] of value.entries()) {
    const itemLabel = `${label}[${index}]`;
    const item = exactObject(entry, itemLabel, ["name", "value"]);
    requireString(item, "name", `${itemLabel}.name`);
    requireString(item, "value", `${itemLabel}.value`);
  }
}

function exactObject(
  value: unknown,
  label: string,
  required: readonly string[],
  optional: readonly string[] = [],
): Record<string, unknown> {
  if (!isRecord(value)) fail(`${label} must be an object`);
  const allowed = new Set([...required, ...optional]);
  const unknown = Object.keys(value).find((field) => !allowed.has(field));
  if (unknown !== undefined) fail(`${label}.${unknown} is not supported`);
  const missing = required.find((field) => !has(value, field));
  if (missing !== undefined) fail(`${label}.${missing} is required`);
  return value;
}

function requireString(
  owner: Record<string, unknown>,
  field: string,
  label: string,
): void {
  if (!has(owner, field) || typeof owner[field] !== "string") {
    fail(`${label} must be a string`);
  }
}

function optionalString(
  owner: Record<string, unknown>,
  field: string,
  label: string,
): void {
  if (has(owner, field) && typeof owner[field] !== "string") {
    fail(`${label} must be a string when present`);
  }
}

function optionalBoolean(
  owner: Record<string, unknown>,
  field: string,
  label = field,
): void {
  if (has(owner, field) && typeof owner[field] !== "boolean") {
    fail(`${label} must be a boolean when present`);
  }
}

function requireSafeUnsignedInteger(value: unknown, label: string): void {
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < 0 ||
    Object.is(value, -0)
  ) {
    fail(`${label} must be a non-negative safe integer other than -0`);
  }
}

function has(owner: object, field: string): boolean {
  return Object.prototype.hasOwnProperty.call(owner, field);
}

function fail(message: string): never {
  throw new RuntimeAssemblyRequestMetadataError(
    `invalid request.start runtimeAssembly envelope: ${message}`,
  );
}
