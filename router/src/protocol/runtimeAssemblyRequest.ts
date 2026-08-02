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
  | "websocketJsonRpc"
  | "spawn";

export interface RuntimeAssemblyRequestDeploymentFrameHeader {
  serviceId: string;
  contractVersion: string;
  deploymentRevision: string;
  deploymentArtifactIdentity: string;
}

export interface RuntimeAssemblyHttpRequestRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  deployment: RuntimeAssemblyRequestDeploymentFrameHeader;
  gatewayEntryIdentity: string;
  ingress: {
    protocol: "http";
    method: string;
    path: string;
  };
}

export interface RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  deployment: RuntimeAssemblyRequestDeploymentFrameHeader;
  gatewayEntryIdentity: string;
  ingress: {
    protocol: "webSocket";
    method: null;
    path: string;
  };
}

export interface RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  deployment: RuntimeAssemblyRequestDeploymentFrameHeader;
  gatewayEntryIdentity: string;
  ingress: {
    protocol: "webSocket";
    method: string;
    path: string;
  };
}

export interface RuntimeAssemblySpawnRequestRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  deployment: RuntimeAssemblyRequestDeploymentFrameHeader;
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
  testCaseCapability?: string;
  testCaseParentRequestId?: string;
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

export interface RuntimeAssemblySpawnRequestStartFrameHeader
  extends Omit<
    RuntimeAssemblyRequestStartFrameHeaderBase,
    "caller" | "mode" | "routing"
  > {
  mode: "unary";
  caller: {
    kind: "service";
  };
  routing: RuntimeAssemblySpawnRequestRoutingFrameHeader;
  invocation: {
    kind: "spawn";
    targetKind: "function";
    target: string;
  };
  testCaseCapability?: string;
}

export type RuntimeAssemblyRequestStartFrameWireHeader =
  | RuntimeAssemblyRequestStartFrameHeader
  | RuntimeAssemblyWebSocketConnectRequestStartFrameHeader
  | RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader
  | RuntimeAssemblySpawnRequestStartFrameHeader;

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
const httpHeaderFields = new Set([
  ...commonHeaderFields,
  "httpRequest",
  "testCaseCapability",
  "testCaseParentRequestId",
]);
const websocketConnectHeaderFields = new Set([
  ...commonHeaderFields,
  "websocketConnect",
]);
const websocketJsonRpcHeaderFields = new Set([
  ...commonHeaderFields,
  "websocketJsonRpc",
]);
const spawnHeaderFields = new Set([
  ...commonHeaderFields,
  "invocation",
  "testCaseCapability",
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
const spawnRequiredHeaderFields = new Set([
  ...commonRequiredHeaderFields,
  "invocation",
]);
const routingFields = new Set([
  "kind",
  "assemblyIdentity",
  "assemblyGeneration",
  "deployment",
  "gatewayEntryIdentity",
  "ingress",
]);
const spawnRoutingFields = new Set([
  "kind",
  "assemblyIdentity",
  "assemblyGeneration",
  "deployment",
]);
const deploymentFields = new Set([
  "serviceId",
  "contractVersion",
  "deploymentRevision",
  "deploymentArtifactIdentity",
]);
const ingressFields = new Set(["protocol", "method", "path"]);
const callerFields = new Set(["kind"]);
const invocationFields = new Set(["kind", "targetKind", "target"]);

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
    spawn: spawnHeaderFields,
  }[wireKind];
  const requiredFields = {
    http: httpRequiredHeaderFields,
    websocketConnect: websocketConnectRequiredHeaderFields,
    websocketJsonRpc: websocketJsonRpcRequiredHeaderFields,
    spawn: spawnRequiredHeaderFields,
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
  const callerError = validateCaller(envelope.caller, wireKind);
  if (callerError !== null) return callerError;
  const testCapabilityError = validateTestCapability(envelope, wireKind);
  if (testCapabilityError !== null) return testCapabilityError;
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

function validateTestCapability(
  envelope: Record<string, unknown>,
  wireKind: RuntimeAssemblyRequestWireKind,
): string | null {
  const hasCapability = Object.prototype.hasOwnProperty.call(
    envelope,
    "testCaseCapability",
  );
  const hasParentRequestId = Object.prototype.hasOwnProperty.call(
    envelope,
    "testCaseParentRequestId",
  );
  const testEffectsEnabled = envelope.testEffectsEnabled ?? false;
  if (wireKind === "websocketConnect" || wireKind === "websocketJsonRpc") {
    return testEffectsEnabled === false
      ? null
      : `invalid request.start runtimeAssembly envelope: ${wireKind} testEffectsEnabled must be false`;
  }
  if (testEffectsEnabled !== hasCapability) {
    return testEffectsEnabled === true
      ? "invalid request.start runtimeAssembly envelope: testEffectsEnabled true requires testCaseCapability"
      : "invalid request.start runtimeAssembly envelope: testCaseCapability requires testEffectsEnabled true";
  }
  if (
    hasCapability &&
    !isTestCaseCorrelationToken(envelope.testCaseCapability)
  ) {
    return "invalid request.start runtimeAssembly envelope: testCaseCapability must be a 1..256 byte test correlation token";
  }
  if (hasParentRequestId && wireKind !== "http") {
    return `invalid request.start runtimeAssembly envelope: ${wireKind} testCaseParentRequestId is not supported`;
  }
  if (hasParentRequestId && !hasCapability) {
    return "invalid request.start runtimeAssembly envelope: testCaseParentRequestId requires testCaseCapability";
  }
  if (
    hasParentRequestId &&
    !isTestCaseCorrelationToken(envelope.testCaseParentRequestId)
  ) {
    return "invalid request.start runtimeAssembly envelope: testCaseParentRequestId must be a 1..256 byte test correlation token";
  }
  return null;
}

function isTestCaseCorrelationToken(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[A-Za-z0-9_.:-]{1,256}$/.test(value)
  );
}

export function validateRuntimeAssemblyRequestRouting(
  envelope: Record<string, unknown>,
  wireKind: RuntimeAssemblyRequestWireKind,
): string | null {
  if (!isRecord(envelope.routing)) {
    return "invalid request.start envelope: routing must be an object";
  }
  const routing = envelope.routing;
  const expectedRoutingFields =
    wireKind === "spawn" ? spawnRoutingFields : routingFields;
  const unsupportedRouting = firstUnsupportedField(
    routing,
    expectedRoutingFields,
  );
  if (unsupportedRouting !== undefined) {
    return `invalid request.start envelope: routing.${unsupportedRouting} is not supported`;
  }
  const missingRouting = firstMissingField(routing, expectedRoutingFields);
  if (missingRouting !== undefined) {
    return `invalid request.start envelope: routing.${missingRouting} is required`;
  }
  if (routing.kind !== "runtimeAssembly") {
    return "invalid request.start envelope: routing.kind must be runtimeAssembly";
  }
  try {
    runtimeAssemblyIdentity(routing.assemblyIdentity);
  } catch {
    return "invalid request.start envelope: routing.assemblyIdentity must be skiff-runtime-assembly-v3:sha256:<64 lowercase hex>";
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
    wireKind !== "spawn" &&
    (typeof routing.gatewayEntryIdentity !== "string" ||
      !GATEWAY_ENTRY_IDENTITY_PATTERN.test(routing.gatewayEntryIdentity))
  ) {
    return "invalid request.start envelope: routing.gatewayEntryIdentity must be skiff-gateway-entry-v2:sha256:<64 lowercase hex>";
  }
  const deploymentError = validateDeployment(routing.deployment);
  if (deploymentError !== null) return deploymentError;
  return wireKind === "spawn"
    ? validateSpawnInvocation(envelope.invocation)
    : validateIngress(routing.ingress, wireKind);
}

function validateDeployment(input: unknown): string | null {
  if (!isRecord(input)) {
    return "invalid request.start envelope: routing.deployment must be an object";
  }
  const unsupported = firstUnsupportedField(input, deploymentFields);
  if (unsupported !== undefined) {
    return `invalid request.start envelope: routing.deployment.${unsupported} is not supported`;
  }
  const missing = firstMissingField(input, deploymentFields);
  if (missing !== undefined) {
    return `invalid request.start envelope: routing.deployment.${missing} is required`;
  }
  if (
    typeof input.serviceId !== "string" ||
    input.serviceId.length === 0 ||
    typeof input.contractVersion !== "string" ||
    input.contractVersion.length === 0 ||
    typeof input.deploymentRevision !== "string" ||
    input.deploymentRevision.length === 0
  ) {
    return "invalid request.start envelope: routing.deployment coordinate must contain non-empty strings";
  }
  if (
    typeof input.deploymentArtifactIdentity !== "string" ||
    !/^skiff-deployment-artifact-v4:sha256:[0-9a-f]{64}$/.test(
      input.deploymentArtifactIdentity,
    )
  ) {
    return "invalid request.start envelope: routing.deployment.deploymentArtifactIdentity must be skiff-deployment-artifact-v4:sha256:<64 lowercase hex>";
  }
  return null;
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
  if (Object.prototype.hasOwnProperty.call(envelope, "invocation")) {
    return { wireKind: "spawn" };
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

function validateCaller(
  input: unknown,
  wireKind: RuntimeAssemblyRequestWireKind,
): string | null {
  if (!isRecord(input)) {
    return "invalid request.start runtimeAssembly envelope: caller must be an object";
  }
  const unsupported = rejectUnknownObjectFields(input, callerFields, "caller");
  if (unsupported !== null) return unsupported;
  const expected = wireKind === "spawn" ? "service" : "gateway";
  return input.kind === expected
    ? null
    : `invalid request.start runtimeAssembly envelope: caller.kind must be ${expected}`;
}

function validateSpawnInvocation(input: unknown): string | null {
  if (!isRecord(input)) {
    return "invalid request.start envelope: invocation must be an object";
  }
  const unsupported = firstUnsupportedField(input, invocationFields);
  if (unsupported !== undefined) {
    return `invalid request.start envelope: invocation.${unsupported} is not supported`;
  }
  const missing = firstMissingField(input, invocationFields);
  if (missing !== undefined) {
    return `invalid request.start envelope: invocation.${missing} is required`;
  }
  if (
    input.kind !== "spawn" ||
    input.targetKind !== "function" ||
    typeof input.target !== "string" ||
    input.target.length === 0
  ) {
    return "invalid request.start envelope: invocation must be an exact function spawn";
  }
  return null;
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
