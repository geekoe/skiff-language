import {
  type DispatchMode,
  type HttpRequestFrameMetadata,
  isRecord,
  RUNTIME_FRAME_SCHEMA_VERSION,
  type RuntimeClientSessionFrameMetadata,
  type TraceContext,
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

export interface RuntimeAssemblyRequestRoutingFrameHeader {
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

export interface RuntimeAssemblyRequestStartFrameHeader {
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
  httpRequest: HttpRequestFrameMetadata;
  testEffectsEnabled: boolean;
}

const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v1:sha256:[0-9a-f]{64}$/;

const canonicalHeaderFields = new Set([
  "schemaVersion",
  "type",
  "requestId",
  "mode",
  "caller",
  "routing",
  "clientSession",
  "deadline",
  "trace",
  "httpRequest",
  "testEffectsEnabled",
]);
const requiredHeaderFields = new Set([
  "schemaVersion",
  "type",
  "requestId",
  "mode",
  "caller",
  "routing",
  "trace",
  "httpRequest",
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
  const unsupportedHeader = firstUnsupportedField(envelope, canonicalHeaderFields);
  if (unsupportedHeader !== undefined) {
    return `invalid request.start runtimeAssembly envelope: ${unsupportedHeader} is not supported`;
  }
  const missingHeader = firstMissingField(envelope, requiredHeaderFields);
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
  if (envelope.mode !== "unary" && envelope.mode !== "serverStream") {
    return "invalid request.start runtimeAssembly envelope: mode must be unary or serverStream";
  }
  const callerError = validateCaller(envelope.caller);
  if (callerError !== null) return callerError;
  return (
    validateRuntimeAssemblyRequestRouting(envelope) ??
    validateRuntimeAssemblyRequestMetadata(envelope)
  );
}

export function normalizeRuntimeAssemblyRequestStartHeader(
  envelope: Record<string, unknown>,
): RuntimeAssemblyRequestStartFrameHeader {
  return normalizeRuntimeAssemblyRequestMetadata(envelope);
}

export function validateRuntimeAssemblyRequestRouting(
  envelope: Record<string, unknown>,
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
    return "invalid request.start envelope: routing.gatewayEntryIdentity must be skiff-gateway-entry-v1:sha256:<64 lowercase hex>";
  }
  return validateIngress(routing.ingress);
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

function validateIngress(input: unknown): string | null {
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
  if (input.protocol !== "http") {
    return "invalid request.start envelope: routing.ingress.protocol must be http";
  }
  if (typeof input.host !== "string" || input.host.length === 0) {
    return "invalid request.start envelope: routing.ingress.host must be a non-empty string";
  }
  if (typeof input.method !== "string" || input.method.length === 0) {
    return "invalid request.start envelope: routing.ingress.method must be a non-empty string";
  }
  if (typeof input.path !== "string" || !input.path.startsWith("/")) {
    return "invalid request.start envelope: routing.ingress.path must be an absolute path";
  }
  return null;
}
