import {
  type DispatchMode,
  type HttpHeaderFrameMetadata,
  type HttpQueryParamFrameMetadata,
  type HttpRequestFrameMetadata,
  isRecord,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RuntimeClientSessionFrameMetadata,
  type TraceContext,
  type WebSocketCookieFrameMetadata,
} from "./envelope.js";
import {
  activationGeneration,
  runtimeAssemblyIdentity,
} from "./assemblyActivationLexical.js";
import {
  normalizeRuntimeAssemblyRequestMetadata,
  validateRuntimeAssemblyRequestMetadata,
} from "./runtimeAssemblyRequestMetadata.js";
import {
  firstMissingField,
  firstUnsupportedField,
  rejectUnknownObjectFields,
} from "./runtimeAssemblyRequestStrict.js";

export type RuntimeAssemblyRequestWireKind =
  | "http"
  | "websocketConnect"
  | "websocketJsonRpc";

export interface RuntimeAssemblyHttpRequestRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  gatewayEntryIdentity: string;
  ingress: {
    protocol: "http";
    host: string;
    method: string;
    path: string;
  };
}

export interface RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  gatewayEntryIdentity: string;
  ingress: {
    protocol: "webSocket";
    host: string;
    method: null;
    path: string;
  };
}

export interface RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  gatewayEntryIdentity: string;
  ingress: {
    protocol: "webSocket";
    host: string;
    method: string;
    path: string;
  };
}

// The HTTP producer view remains a named type while the wire reader exposes
// RuntimeAssemblyRequestStartFrameWireHeader as the exact closed union.
export type RuntimeAssemblyRequestRoutingFrameHeader =
  RuntimeAssemblyHttpRequestRoutingFrameHeader;

interface RuntimeAssemblyRequestStartFrameHeaderBase {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: "request.start";
  requestId: string;
  mode: DispatchMode;
  caller: {
    kind: "gateway";
  };
  routing: RuntimeAssemblyRequestRoutingFrameHeader;
  clientSession?: RuntimeClientSessionFrameMetadata;
  deadline?: {
    timeoutMs: number;
    expiresAt: string;
  };
  trace: TraceContext;
  testEffectsEnabled: boolean;
}

export interface RuntimeAssemblyRequestStartFrameHeader
  extends RuntimeAssemblyRequestStartFrameHeaderBase {
  routing: RuntimeAssemblyHttpRequestRoutingFrameHeader;
  httpRequest: HttpRequestFrameMetadata;
}

export interface RuntimeAssemblyWebSocketConnectRequestFrameMetadata {
  connectionId: string;
  url: string;
  query: HttpQueryParamFrameMetadata[];
  headers: HttpHeaderFrameMetadata[];
  cookies: WebSocketCookieFrameMetadata[];
  version?: string;
  websocketEntryId: string;
  gatewayEntryIdentity: string;
}

export interface RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
  extends Omit<RuntimeAssemblyRequestStartFrameHeaderBase, "mode" | "routing"> {
  mode: "unary";
  routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader;
  websocketConnect: RuntimeAssemblyWebSocketConnectRequestFrameMetadata;
}

export interface RuntimeAssemblyWebSocketJsonRpcRequestFrameMetadata {
  profile: "jsonrpc-2.0-text";
  connectionId: string;
  websocketEntryId: string;
  gatewayEntryIdentity: string;
  businessIdentity?: string;
}

export interface RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
  extends Omit<RuntimeAssemblyRequestStartFrameHeaderBase, "mode" | "routing"> {
  mode: "unary";
  routing: RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader;
  websocketJsonRpc: RuntimeAssemblyWebSocketJsonRpcRequestFrameMetadata;
}

export type RuntimeAssemblyRequestStartFrameWireHeader =
  | RuntimeAssemblyRequestStartFrameHeader
  | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
  | RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader;

export type RuntimeAssemblyRequestStartFrameTransportWireHeader =
  RuntimeAssemblyRequestStartFrameWireHeader;

const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$/;

const commonHeaderFields = [
  "schemaVersion",
  "type",
  "requestId",
  "mode",
  "caller",
  "routing",
  "clientSession",
  "deadline",
  "trace",
  "testEffectsEnabled",
] as const;
const commonRequiredHeaderFields = [
  "schemaVersion",
  "type",
  "requestId",
  "mode",
  "caller",
  "routing",
  "trace",
] as const;
const httpHeaderFields = new Set([...commonHeaderFields, "httpRequest"]);
const websocketConnectHeaderFields = new Set([
  ...commonHeaderFields,
  "websocketConnect",
]);
const websocketJsonRpcHeaderFields = new Set([
  ...commonHeaderFields,
  "websocketJsonRpc",
]);
const httpRequiredHeaderFields = new Set([
  ...commonRequiredHeaderFields,
  "httpRequest",
]);
const websocketConnectRequiredHeaderFields = new Set([
  ...commonRequiredHeaderFields,
  "websocketConnect",
]);
const websocketJsonRpcRequiredHeaderFields = new Set([
  ...commonRequiredHeaderFields,
  "websocketJsonRpc",
]);
const routingFields = new Set([
  "kind",
  "assemblyIdentity",
  "assemblyGeneration",
  "gatewayEntryIdentity",
  "ingress",
]);
const ingressFields = new Set(["protocol", "host", "method", "path"]);
const callerFields = new Set(["kind"]);

export function hasRuntimeAssemblyRouting(
  envelope: Record<string, unknown>,
): boolean {
  return Object.prototype.hasOwnProperty.call(envelope, "routing");
}

export function validateRuntimeAssemblyRequestStartHeader(
  envelope: Record<string, unknown>,
): string | null {
  const wireKindResult = runtimeAssemblyRequestWireKind(envelope);
  if ("error" in wireKindResult) return wireKindResult.error;
  const { wireKind } = wireKindResult;
  const allowedFields = {
    http: httpHeaderFields,
    websocketConnect: websocketConnectHeaderFields,
    websocketJsonRpc: websocketJsonRpcHeaderFields,
  }[wireKind];
  const requiredFields = {
    http: httpRequiredHeaderFields,
    websocketConnect: websocketConnectRequiredHeaderFields,
    websocketJsonRpc: websocketJsonRpcRequiredHeaderFields,
  }[wireKind];
  const unsupportedHeader = firstUnsupportedField(envelope, allowedFields);
  if (unsupportedHeader !== undefined) {
    return `invalid request.start runtimeAssembly envelope: ${unsupportedHeader} is not supported`;
  }
  const missingHeader = firstMissingField(envelope, requiredFields);
  if (missingHeader !== undefined) {
    return `invalid request.start runtimeAssembly envelope: ${missingHeader} is required`;
  }
  if (envelope.schemaVersion !== RUNTIME_FRAME_SCHEMA_VERSION) {
    return `invalid request.start runtimeAssembly envelope: schemaVersion must be ${RUNTIME_FRAME_SCHEMA_VERSION}`;
  }
  if (envelope.type !== "request.start") {
    return "invalid request.start runtimeAssembly envelope: type must be request.start";
  }
  if (typeof envelope.requestId !== "string") {
    return "invalid request.start runtimeAssembly envelope: requestId must be a string";
  }
  if (
    wireKind === "websocketJsonRpc" &&
    !isBoundedCanonicalString(envelope.requestId, 1024)
  ) {
    return "invalid request.start runtimeAssembly envelope: websocketJsonRpc requestId must be a bounded non-empty canonical string";
  }
  if (envelope.mode !== "unary" && envelope.mode !== "serverStream") {
    return "invalid request.start runtimeAssembly envelope: mode must be unary or serverStream";
  }
  if (wireKind !== "http" && envelope.mode !== "unary") {
    return `invalid request.start runtimeAssembly envelope: ${wireKind} mode must be unary`;
  }
  const callerError = validateCaller(envelope.caller);
  if (callerError !== null) return callerError;
  return (
    validateRuntimeAssemblyRequestRouting(envelope, wireKind) ??
    validateRuntimeAssemblyRequestMetadata(envelope, wireKind)
  );
}

export function normalizeRuntimeAssemblyRequestStartHeader(
  envelope: Record<string, unknown>,
): RuntimeAssemblyRequestStartFrameTransportWireHeader {
  return normalizeRuntimeAssemblyRequestMetadata(envelope);
}

export function validateRuntimeAssemblyRequestRouting(
  envelope: Record<string, unknown>,
  wireKind: RuntimeAssemblyRequestWireKind,
): string | null {
  if (!isRecord(envelope.routing)) {
    return "invalid request.start envelope: routing must be an object";
  }
  const routing = envelope.routing;
  const unsupportedRouting = firstUnsupportedField(routing, routingFields);
  if (unsupportedRouting !== undefined) {
    return `invalid request.start envelope: routing.${unsupportedRouting} is not supported`;
  }
  const missingRouting = firstMissingField(routing, routingFields);
  if (missingRouting !== undefined) {
    return `invalid request.start envelope: routing.${missingRouting} is required`;
  }
  if (routing.kind !== "runtimeAssembly") {
    return "invalid request.start envelope: routing.kind must be runtimeAssembly";
  }
  try {
    runtimeAssemblyIdentity(routing.assemblyIdentity);
  } catch {
    return "invalid request.start envelope: routing.assemblyIdentity must be skiff-runtime-assembly-v2:sha256:<64 lowercase hex>";
  }
  try {
    activationGeneration(
      routing.assemblyGeneration,
      "request.start routing.assemblyGeneration",
    );
  } catch {
    return "invalid request.start envelope: routing.assemblyGeneration must be a non-negative safe integer";
  }
  if (
    typeof routing.gatewayEntryIdentity !== "string" ||
    !GATEWAY_ENTRY_IDENTITY_PATTERN.test(routing.gatewayEntryIdentity)
  ) {
    return "invalid request.start envelope: routing.gatewayEntryIdentity must be skiff-gateway-entry-v2:sha256:<64 lowercase hex>";
  }
  return validateIngress(routing.ingress, wireKind);
}

function runtimeAssemblyRequestWireKind(
  envelope: Record<string, unknown>,
):
  | { wireKind: RuntimeAssemblyRequestWireKind }
  | {
      error: string;
    } {
  if (!isRecord(envelope.routing)) {
    return {
      error:
        "invalid request.start runtimeAssembly envelope: routing must be an object",
    };
  }
  if (!isRecord(envelope.routing.ingress)) {
    return {
      error:
        "invalid request.start runtimeAssembly envelope: routing.ingress must be an object",
    };
  }
  if (envelope.routing.ingress.protocol === "http") {
    return { wireKind: "http" };
  }
  if (envelope.routing.ingress.protocol === "webSocket") {
    if (envelope.routing.ingress.method === null) {
      return { wireKind: "websocketConnect" };
    }
    if (typeof envelope.routing.ingress.method === "string") {
      return { wireKind: "websocketJsonRpc" };
    }
    return {
      error:
        "invalid request.start runtimeAssembly envelope: routing.ingress.method must be null or a string for webSocket",
    };
  }
  return {
    error:
      "invalid request.start runtimeAssembly envelope: routing.ingress.protocol must be http or webSocket",
  };
}

function validateCaller(input: unknown): string | null {
  if (!isRecord(input)) {
    return "invalid request.start runtimeAssembly envelope: caller must be an object";
  }
  const unsupported = rejectUnknownObjectFields(input, callerFields, "caller");
  if (unsupported !== null) return unsupported;
  return input.kind === "gateway"
    ? null
    : "invalid request.start runtimeAssembly envelope: caller.kind must be gateway";
}

function validateIngress(
  input: unknown,
  wireKind: RuntimeAssemblyRequestWireKind,
): string | null {
  if (!isRecord(input)) {
    return "invalid request.start envelope: routing.ingress must be an object";
  }
  const unsupported = firstUnsupportedField(input, ingressFields);
  if (unsupported !== undefined) {
    return `invalid request.start envelope: routing.ingress.${unsupported} is not supported`;
  }
  const missing = firstMissingField(input, ingressFields);
  if (missing !== undefined) {
    return `invalid request.start envelope: routing.ingress.${missing} is required`;
  }
  if (input.protocol !== "http" && input.protocol !== "webSocket") {
    return "invalid request.start envelope: routing.ingress.protocol must be http or webSocket";
  }
  if (typeof input.host !== "string" || input.host.length === 0) {
    return "invalid request.start envelope: routing.ingress.host must be a non-empty string";
  }
  if (
    (wireKind === "http" &&
      (typeof input.method !== "string" || input.method.length === 0)) ||
    (wireKind === "websocketConnect" && input.method !== null) ||
    (wireKind === "websocketJsonRpc" &&
      (typeof input.method !== "string" ||
        input.method.length === 0 ||
        Buffer.byteLength(input.method, "utf8") > 256))
  ) {
    return "invalid request.start envelope: routing.ingress.method does not match protocol";
  }
  if (typeof input.path !== "string" || !input.path.startsWith("/")) {
    return "invalid request.start envelope: routing.ingress.path must be an absolute path";
  }
  return null;
}

function isBoundedCanonicalString(value: string, maxBytes: number): boolean {
  return (
    value.length > 0 &&
    value.trim() === value &&
    !/\p{Cc}/u.test(value) &&
    Buffer.byteLength(value, "utf8") <= maxBytes
  );
}
