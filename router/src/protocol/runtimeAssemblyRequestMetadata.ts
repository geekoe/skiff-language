import { isRecord } from "./envelope.js";
import type {
  RuntimeAssemblyRequestStartFrameTransportWireHeader,
  RuntimeAssemblyRequestWireKind,
} from "./runtimeAssemblyRequest.js";

class RuntimeAssemblyRequestMetadataError extends Error {}

const CONNECTION_ID_PATTERN = /^(?=.{1,255}$)[A-Za-z0-9._:~-]+$/;
const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$/;
const WEBSOCKET_ENTRY_ID_PATTERN =
  /^skiff-websocket-entry-v1:sha256:[0-9a-f]{64}$/;

export function isCanonicalRuntimeAssemblyWebSocketConnectionId(
  value: unknown,
): value is string {
  return typeof value === "string" && CONNECTION_ID_PATTERN.test(value);
}

export function isCanonicalRuntimeAssemblyWebSocketEntryId(
  value: unknown,
): value is string {
  return typeof value === "string" && WEBSOCKET_ENTRY_ID_PATTERN.test(value);
}

export function isCanonicalRuntimeAssemblyWebSocketBusinessIdentity(
  value: unknown,
): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.trim() === value &&
    !/\p{Cc}/u.test(value) &&
    Buffer.byteLength(value, "utf8") <= 1024
  );
}

export function validateRuntimeAssemblyRequestMetadata(
  envelope: Record<string, unknown>,
  wireKind: RuntimeAssemblyRequestWireKind,
): string | null {
  try {
    validateClientSession(envelope);
    validateDeadline(envelope);
    validateTrace(envelope);
    if (wireKind === "http") {
      validateHttpRequest(envelope);
    } else if (wireKind === "websocketConnect") {
      validateWebSocketConnect(envelope);
    } else {
      validateWebSocketJsonRpc(envelope);
    }
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
): RuntimeAssemblyRequestStartFrameTransportWireHeader {
  return {
    ...envelope,
    testEffectsEnabled: envelope.testEffectsEnabled ?? false,
  } as unknown as RuntimeAssemblyRequestStartFrameTransportWireHeader;
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

function validateWebSocketConnect(envelope: Record<string, unknown>): void {
  const connect = exactObject(
    envelope.websocketConnect,
    "websocketConnect",
    [
      "connectionId",
      "url",
      "query",
      "headers",
      "cookies",
      "websocketEntryId",
      "gatewayEntryIdentity",
    ],
    ["version"],
  );
  requireCanonicalConnectionId(
    connect,
    "connectionId",
    "websocketConnect.connectionId",
  );
  requireString(connect, "url", "websocketConnect.url");
  validateNameValueArray(connect.query, "websocketConnect.query");
  validateNameValueArray(connect.headers, "websocketConnect.headers");
  validateNameValueArray(connect.cookies, "websocketConnect.cookies");
  optionalString(connect, "version", "websocketConnect.version");
  requireCanonicalWebSocketEntryId(
    connect,
    "websocketEntryId",
    "websocketConnect.websocketEntryId",
  );
  requirePattern(
    connect,
    "gatewayEntryIdentity",
    "websocketConnect.gatewayEntryIdentity",
    GATEWAY_ENTRY_IDENTITY_PATTERN,
  );
  const routing = exactObject(envelope.routing, "routing", [
    "kind",
    "assemblyIdentity",
    "assemblyGeneration",
    "gatewayEntryIdentity",
    "ingress",
  ]);
  if (connect.gatewayEntryIdentity !== routing.gatewayEntryIdentity) {
    fail(
      "websocketConnect.gatewayEntryIdentity must match routing.gatewayEntryIdentity",
    );
  }
}

function validateWebSocketJsonRpc(
  envelope: Record<string, unknown>,
): void {
  const request = exactObject(
    envelope.websocketJsonRpc,
    "websocketJsonRpc",
    [
      "profile",
      "connectionId",
      "websocketEntryId",
      "gatewayEntryIdentity",
    ],
    ["businessIdentity"],
  );
  if (request.profile !== "jsonrpc-2.0-text") {
    fail("websocketJsonRpc.profile must be jsonrpc-2.0-text");
  }
  requireCanonicalConnectionId(
    request,
    "connectionId",
    "websocketJsonRpc.connectionId",
  );
  requireCanonicalWebSocketEntryId(
    request,
    "websocketEntryId",
    "websocketJsonRpc.websocketEntryId",
  );
  requirePattern(
    request,
    "gatewayEntryIdentity",
    "websocketJsonRpc.gatewayEntryIdentity",
    GATEWAY_ENTRY_IDENTITY_PATTERN,
  );
  optionalBusinessIdentity(
    request,
    "businessIdentity",
    "websocketJsonRpc.businessIdentity",
  );
  const routing = exactObject(envelope.routing, "routing", [
    "kind",
    "assemblyIdentity",
    "assemblyGeneration",
    "gatewayEntryIdentity",
    "ingress",
  ]);
  if (request.gatewayEntryIdentity !== routing.gatewayEntryIdentity) {
    fail(
      "websocketJsonRpc.gatewayEntryIdentity must match routing.gatewayEntryIdentity",
    );
  }
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

function optionalBusinessIdentity(
  owner: Record<string, unknown>,
  field: string,
  label: string,
): void {
  if (!has(owner, field)) return;
  if (!isCanonicalRuntimeAssemblyWebSocketBusinessIdentity(owner[field])) {
    fail(`${label} must be a bounded non-empty canonical string`);
  }
}

function requireCanonicalConnectionId(
  owner: Record<string, unknown>,
  field: string,
  label: string,
): void {
  if (!isCanonicalRuntimeAssemblyWebSocketConnectionId(owner[field])) {
    fail(`${label} is not canonical`);
  }
}

function requireCanonicalWebSocketEntryId(
  owner: Record<string, unknown>,
  field: string,
  label: string,
): void {
  if (!isCanonicalRuntimeAssemblyWebSocketEntryId(owner[field])) {
    fail(`${label} is not canonical`);
  }
}

function requirePattern(
  owner: Record<string, unknown>,
  field: string,
  label: string,
  pattern: RegExp,
): void {
  if (
    !has(owner, field) ||
    typeof owner[field] !== "string" ||
    !pattern.test(owner[field])
  ) {
    fail(`${label} is not canonical`);
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
