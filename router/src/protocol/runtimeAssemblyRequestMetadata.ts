import { isRecord } from "./envelope.js";

const ACTIVATION_IDENTITY_PATTERN =
  /^skiff-runtime-activation-v1:opaque:[A-Za-z0-9._:-]+$/;
const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-v1:sha256:[0-9a-f]{64}$/;
const HTTP_SOURCE_KINDS = new Set([
  "http.request",
  "http.body",
  "http.context",
]);
const WEBSOCKET_SOURCE_KINDS = new Set([
  "websocket.connectRequest",
  "websocket.receiveEvent",
  "websocket.connection",
  "websocket.connectionContext",
  "websocket.message",
  "websocket.messageBody",
  "websocket.connectionId",
  "websocket.businessIdentity",
]);

class RuntimeAssemblyRequestMetadataError extends Error {}

export function validateRuntimeAssemblyRequestMetadata(
  envelope: Record<string, unknown>,
): string | null {
  try {
    optionalPattern(
      envelope,
      "activationIdentity",
      ACTIVATION_IDENTITY_PATTERN,
      "skiff-runtime-activation-v1:opaque:<opaque id>",
    );
    optionalPattern(
      envelope,
      "gatewayEntryIdentity",
      GATEWAY_ENTRY_IDENTITY_PATTERN,
      "skiff-gateway-v1:sha256:<64 lowercase hex>",
    );
    optionalString(envelope, "businessIdentity");
    optionalString(envelope, "websocketEntryId");
    validateClientSession(envelope);
    validateDeadline(envelope);
    validateTrace(envelope);
    validateHttpRequest(envelope);
    validateHttpAdapter(envelope);
    validateWebSocketAdapter(envelope);
    optionalBoolean(envelope, "testEffectsEnabled");
    validateTestEffectDoubles(envelope);
    return null;
  } catch (error) {
    if (error instanceof RuntimeAssemblyRequestMetadataError) {
      return error.message;
    }
    throw error;
  }
}

function validateClientSession(envelope: Record<string, unknown>): void {
  if (!has(envelope, "clientSession")) return;
  const session = exactObject(
    envelope.clientSession,
    "clientSession",
    ["id"],
  );
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
  if (!has(envelope, "httpRequest")) return;
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

function validateHttpAdapter(envelope: Record<string, unknown>): void {
  if (!has(envelope, "httpAdapter")) return;
  const adapter = exactObject(
    envelope.httpAdapter,
    "httpAdapter",
    ["kind", "handler"],
    ["guard", "pre", "adapterArgs"],
  );
  requireEnum(adapter.kind, "httpAdapter.kind", ["typedJson", "rawHttp"]);
  validateCallable(adapter.handler, "httpAdapter.handler");
  if (has(adapter, "guard")) validateCallable(adapter.guard, "httpAdapter.guard");
  if (has(adapter, "pre")) validateCallable(adapter.pre, "httpAdapter.pre");
  if (has(adapter, "adapterArgs")) {
    validateAdapterArgs(adapter.adapterArgs, "httpAdapter.adapterArgs", HTTP_SOURCE_KINDS);
  }
}

function validateCallable(value: unknown, label: string): void {
  const callable = exactObject(
    value,
    label,
    ["kind"],
    ["modulePath", "symbol", "packageId", "symbolPath"],
  );
  if (callable.kind === "serviceFunction") {
    exactFields(callable, label, ["kind", "modulePath", "symbol"]);
    requireString(callable, "modulePath", `${label}.modulePath`);
    requireString(callable, "symbol", `${label}.symbol`);
    return;
  }
  if (callable.kind === "packageFunction") {
    exactFields(callable, label, ["kind", "packageId", "symbolPath"]);
    requireString(callable, "packageId", `${label}.packageId`);
    requireString(callable, "symbolPath", `${label}.symbolPath`);
    return;
  }
  fail(`${label}.kind must be serviceFunction or packageFunction`);
}

function validateWebSocketAdapter(envelope: Record<string, unknown>): void {
  if (!has(envelope, "websocketAdapter")) return;
  const adapter = exactObject(
    envelope.websocketAdapter,
    "websocketAdapter",
    ["kind"],
    ["adapterArgs", "contextExpectation", "connectRequest", "receiveEvent"],
  );
  requireEnum(adapter.kind, "websocketAdapter.kind", ["connect", "receive"]);
  if (has(adapter, "adapterArgs")) {
    validateAdapterArgs(
      adapter.adapterArgs,
      "websocketAdapter.adapterArgs",
      WEBSOCKET_SOURCE_KINDS,
    );
  }
  if (has(adapter, "contextExpectation")) {
    validateContextExpectation(adapter.contextExpectation);
  }
  requireString(envelope, "websocketEntryId", "websocketEntryId");
  requireString(envelope, "gatewayEntryIdentity", "gatewayEntryIdentity");
  if (adapter.kind === "connect") {
    if (has(adapter, "receiveEvent")) {
      fail("websocketAdapter.receiveEvent is not supported for connect");
    }
    validateConnectRequest(adapter.connectRequest);
    return;
  }
  if (has(adapter, "connectRequest")) {
    fail("websocketAdapter.connectRequest is not supported for receive");
  }
  validateReceiveEvent(adapter.receiveEvent);
}

function validateContextExpectation(value: unknown): void {
  const label = "websocketAdapter.contextExpectation";
  const expectation = exactObject(
    value,
    label,
    ["kind"],
    ["connectOperationAbiId", "contextTypeIdentity"],
  );
  if (expectation.kind === "null") {
    exactFields(expectation, label, ["kind"]);
    return;
  }
  if (expectation.kind !== "typed") {
    fail(`${label}.kind must be null or typed`);
  }
  exactFields(expectation, label, [
    "kind",
    "connectOperationAbiId",
    "contextTypeIdentity",
  ]);
  requireString(
    expectation,
    "connectOperationAbiId",
    `${label}.connectOperationAbiId`,
  );
  requireString(
    expectation,
    "contextTypeIdentity",
    `${label}.contextTypeIdentity`,
  );
}

function validateConnectRequest(value: unknown): void {
  const label = "websocketAdapter.connectRequest";
  const request = exactObject(
    value,
    label,
    ["connectionId", "url", "query", "headers", "cookies"],
    ["version"],
  );
  requireString(request, "connectionId", `${label}.connectionId`);
  requireString(request, "url", `${label}.url`);
  validateNameValueArray(request.query, `${label}.query`);
  validateNameValueArray(request.headers, `${label}.headers`);
  validateNameValueArray(request.cookies, `${label}.cookies`);
  optionalString(request, "version", `${label}.version`);
}

function validateReceiveEvent(value: unknown): void {
  const label = "websocketAdapter.receiveEvent";
  const event = exactObject(
    value,
    label,
    ["connectionId", "message", "payloadSegments"],
    ["businessIdentity", "contextCodec"],
  );
  requireString(event, "connectionId", `${label}.connectionId`);
  optionalString(event, "businessIdentity", `${label}.businessIdentity`);
  const message = exactObject(event.message, `${label}.message`, ["tag", "encoding"]);
  requireEnum(message.tag, `${label}.message.tag`, ["text", "binary"]);
  requireEnum(message.encoding, `${label}.message.encoding`, ["utf8", "binary"]);
  if (!Array.isArray(event.payloadSegments)) {
    fail(`${label}.payloadSegments must be an array`);
  }
  for (const [index, value] of event.payloadSegments.entries()) {
    const segmentLabel = `${label}.payloadSegments[${index}]`;
    const segment = exactObject(value, segmentLabel, ["kind", "offset", "length"]);
    requireEnum(segment.kind, `${segmentLabel}.kind`, [
      "websocket.context",
      "websocket.message",
    ]);
    requireSafeUnsignedInteger(segment.offset, `${segmentLabel}.offset`);
    requireSafeUnsignedInteger(segment.length, `${segmentLabel}.length`);
  }
  if (has(event, "contextCodec")) {
    const codec = exactObject(event.contextCodec, `${label}.contextCodec`, [
      "operationAbiId",
      "contextTypeIdentity",
    ]);
    requireString(codec, "operationAbiId", `${label}.contextCodec.operationAbiId`);
    requireString(
      codec,
      "contextTypeIdentity",
      `${label}.contextCodec.contextTypeIdentity`,
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

function validateAdapterArgs(
  value: unknown,
  label: string,
  sourceKinds: ReadonlySet<string>,
): void {
  if (!Array.isArray(value)) fail(`${label} must be an array`);
  const params = new Set<string>();
  for (const [index, entry] of value.entries()) {
    const itemLabel = `${label}[${index}]`;
    const item = exactObject(entry, itemLabel, ["param", "source"]);
    requireString(item, "param", `${itemLabel}.param`);
    if ((item.param as string).trim().length === 0) {
      fail(`${itemLabel}.param must be non-blank`);
    }
    if (params.has(item.param as string)) {
      fail(`${label} has duplicate param ${item.param as string}`);
    }
    params.add(item.param as string);
    const source = exactObject(item.source, `${itemLabel}.source`, ["kind"]);
    if (typeof source.kind !== "string" || !sourceKinds.has(source.kind)) {
      fail(`${itemLabel}.source.kind is not supported`);
    }
  }
}

function validateTestEffectDoubles(envelope: Record<string, unknown>): void {
  if (!has(envelope, "testEffectDoubles")) return;
  const doubles = envelope.testEffectDoubles;
  if (!isRecord(doubles)) fail("testEffectDoubles must be an object");
  for (const [target, sequence] of Object.entries(doubles)) {
    if (!Array.isArray(sequence) || sequence.length === 0) {
      fail(`testEffectDoubles.${target} must be a non-empty array`);
    }
    for (const [index, value] of sequence.entries()) {
      const label = `testEffectDoubles.${target}[${index}]`;
      const step = exactObject(value, label, ["response"], ["expectRequest"]);
      if (has(step, "expectRequest")) validateJsonValue(step.expectRequest, `${label}.expectRequest`);
      validateJsonValue(step.response, `${label}.response`);
    }
  }
}

function validateJsonValue(value: unknown, label: string): void {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean" ||
    (typeof value === "number" && Number.isFinite(value))
  ) {
    return;
  }
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      if (!has(value, String(index))) fail(`${label} must not contain sparse array entries`);
      validateJsonValue(value[index], `${label}[${index}]`);
    }
    return;
  }
  if (isRecord(value)) {
    for (const [key, child] of Object.entries(value)) {
      validateJsonValue(child, `${label}.${key}`);
    }
    return;
  }
  fail(`${label} must be a JSON value`);
}

function exactObject(
  value: unknown,
  label: string,
  required: readonly string[],
  optional: readonly string[] = [],
): Record<string, unknown> {
  if (!isRecord(value)) fail(`${label} must be an object`);
  exactFields(value, label, required, optional);
  return value;
}

function exactFields(
  value: Record<string, unknown>,
  label: string,
  required: readonly string[],
  optional: readonly string[] = [],
): void {
  const allowed = new Set([...required, ...optional]);
  const unknown = Object.keys(value).find((field) => !allowed.has(field));
  if (unknown !== undefined) fail(`${label}.${unknown} is not supported`);
  const missing = required.find((field) => !has(value, field));
  if (missing !== undefined) fail(`${label}.${missing} is required`);
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
  label = field,
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

function optionalPattern(
  owner: Record<string, unknown>,
  field: string,
  pattern: RegExp,
  expectation: string,
): void {
  if (!has(owner, field)) return;
  const value = owner[field];
  if (typeof value !== "string" || !pattern.test(value)) {
    fail(`${field} must be ${expectation}`);
  }
}

function requireEnum(value: unknown, label: string, allowed: readonly string[]): void {
  if (typeof value !== "string" || !allowed.includes(value)) {
    fail(`${label} must be one of ${allowed.join(", ")}`);
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
