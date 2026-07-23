import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  decodeBinaryFrameParts,
  encodeBinaryFrame,
  isRecord,
} from "./envelope.js";
import {
  type ActivationJsonInput,
  parseStrictActivationJson,
} from "./strictActivationJson.js";
import { isPublicationId } from "../publicationId.js";

export const WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE =
  "websocket.generation.lifecycle" as const;

export const WEBSOCKET_GENERATION_LIFECYCLE_REJECTION_CODES = [
  "generation-unavailable",
  "not-acquired",
  "request-conflict",
  "sender-mismatch",
  "tuple-mismatch",
] as const;

export type WebSocketGenerationLifecycleRejectionCode =
  (typeof WEBSOCKET_GENERATION_LIFECYCLE_REJECTION_CODES)[number];

export type WebSocketGenerationLifecycleDirection =
  | "routerToRuntime"
  | "runtimeToRouter";

export interface WebSocketGenerationLifecycleTuple {
  routerSessionId: string;
  serviceId: string;
  assemblyIdentity: string;
  assemblyGeneration: number;
  websocketEntryId: string;
  connectionId: string;
}

export type WebSocketGenerationLifecycleOperation = "acquire" | "release";
export type WebSocketGenerationLifecycleSender = "router" | "runtime";

interface WebSocketGenerationLifecycleControlBase {
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: typeof WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE;
  requestId: string;
  sender: WebSocketGenerationLifecycleSender;
  tuple: WebSocketGenerationLifecycleTuple;
}

export interface WebSocketGenerationAcquireControl
  extends WebSocketGenerationLifecycleControlBase {
  action: "acquire";
  sender: "runtime";
}

export interface WebSocketGenerationReleaseControl
  extends WebSocketGenerationLifecycleControlBase {
  action: "release";
  sender: "router";
}

export interface WebSocketGenerationLifecycleAckControl
  extends WebSocketGenerationLifecycleControlBase {
  action: "ack";
  operation: WebSocketGenerationLifecycleOperation;
}

export interface WebSocketGenerationLifecycleRejectControl
  extends WebSocketGenerationLifecycleControlBase {
  action: "reject";
  operation: WebSocketGenerationLifecycleOperation;
  code: WebSocketGenerationLifecycleRejectionCode;
  reason: string;
}

export type WebSocketGenerationLifecycleRequest =
  | WebSocketGenerationAcquireControl
  | WebSocketGenerationReleaseControl;

export type WebSocketGenerationLifecycleResponse =
  | WebSocketGenerationLifecycleAckControl
  | WebSocketGenerationLifecycleRejectControl;

export type WebSocketGenerationLifecycleControl =
  | WebSocketGenerationLifecycleRequest
  | WebSocketGenerationLifecycleResponse;

export function decodeWebSocketGenerationLifecycleControl(
  value: unknown,
  direction: WebSocketGenerationLifecycleDirection,
): WebSocketGenerationLifecycleControl {
  return decodeControl(value, direction);
}

export function decodeRawWebSocketGenerationLifecycleControl(
  input: ActivationJsonInput,
  direction: WebSocketGenerationLifecycleDirection,
): WebSocketGenerationLifecycleControl {
  return decodeControl(parseStrictActivationJson(input), direction);
}

export function encodeWebSocketGenerationLifecycleFrame(
  control: WebSocketGenerationLifecycleControl,
  direction: WebSocketGenerationLifecycleDirection,
): Buffer {
  const validated = decodeControl(control, direction);
  return encodeBinaryFrame(
    validated as unknown as Record<string, unknown>,
  );
}

export function decodeWebSocketGenerationLifecycleFrame(
  frame: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
  direction: WebSocketGenerationLifecycleDirection,
): WebSocketGenerationLifecycleControl {
  const { headerBytes, payloadBytes } = decodeBinaryFrameParts(frame);
  if (payloadBytes.byteLength !== 0) {
    throw new Error("websocket generation lifecycle frame payload must be empty");
  }
  return decodeRawWebSocketGenerationLifecycleControl(headerBytes, direction);
}

export function assertWebSocketGenerationLifecycleResponseMatches(
  request: WebSocketGenerationLifecycleRequest,
  response: WebSocketGenerationLifecycleResponse,
): void {
  if (response.requestId !== request.requestId) {
    throw new Error("websocket generation lifecycle response requestId mismatch");
  }
  if (response.operation !== request.action) {
    throw new Error("websocket generation lifecycle response operation mismatch");
  }
  if (!tuplesEqual(response.tuple, request.tuple)) {
    throw new Error("websocket generation lifecycle response tuple mismatch");
  }
}

function decodeControl(
  value: unknown,
  direction: WebSocketGenerationLifecycleDirection,
): WebSocketGenerationLifecycleControl {
  if (!isRecord(value)) {
    throw new Error("websocket generation lifecycle control must be an object");
  }
  const action = requireEnum(value, "action", ["acquire", "release", "ack", "reject"] as const);
  const expectedFields =
    action === "acquire" || action === "release"
      ? ["schemaVersion", "type", "action", "requestId", "sender", "tuple"]
      : action === "ack"
        ? ["schemaVersion", "type", "action", "operation", "requestId", "sender", "tuple"]
        : [
            "schemaVersion",
            "type",
            "action",
            "operation",
            "requestId",
            "sender",
            "tuple",
            "code",
            "reason",
          ];
  requireExactFields(value, expectedFields);
  if (value.schemaVersion !== RUNTIME_FRAME_SCHEMA_VERSION) {
    throw new Error(
      `websocket generation lifecycle schemaVersion must be ${RUNTIME_FRAME_SCHEMA_VERSION}`,
    );
  }
  if (value.type !== WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE) {
    throw new Error(
      `websocket generation lifecycle type must be ${WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE}`,
    );
  }
  const requestId = requirePattern(
    value,
    "requestId",
    /^skiff-websocket-lifecycle-request-v1:opaque:[A-Za-z0-9._:-]+$/,
  );
  const sender = requireEnum(value, "sender", ["router", "runtime"] as const);
  const tuple = decodeTuple(value.tuple);

  if (action === "acquire") {
    requireDirection(direction, "runtimeToRouter", action);
    requireSender(sender, "runtime", action);
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action,
      requestId,
      sender,
      tuple,
    };
  }
  if (action === "release") {
    requireDirection(direction, "routerToRuntime", action);
    requireSender(sender, "router", action);
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action,
      requestId,
      sender,
      tuple,
    };
  }

  const operation = requireEnum(value, "operation", ["acquire", "release"] as const);
  const expectedDirection =
    operation === "acquire" ? "routerToRuntime" : "runtimeToRouter";
  const expectedSender = operation === "acquire" ? "router" : "runtime";
  requireDirection(direction, expectedDirection, `${operation} ${action}`);
  requireSender(sender, expectedSender, `${operation} ${action}`);
  if (action === "ack") {
    return {
      schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
      type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
      action,
      operation,
      requestId,
      sender,
      tuple,
    };
  }
  const code = requireEnum(
    value,
    "code",
    WEBSOCKET_GENERATION_LIFECYCLE_REJECTION_CODES,
  );
  const reason = requireNonEmptyString(value, "reason");
  return {
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    action,
    operation,
    requestId,
    sender,
    tuple,
    code,
    reason,
  };
}

function decodeTuple(value: unknown): WebSocketGenerationLifecycleTuple {
  if (!isRecord(value)) {
    throw new Error("websocket generation lifecycle tuple must be an object");
  }
  requireExactFields(value, [
    "routerSessionId",
    "serviceId",
    "assemblyIdentity",
    "assemblyGeneration",
    "websocketEntryId",
    "connectionId",
  ]);
  const assemblyGeneration = value.assemblyGeneration;
  if (
    !Number.isSafeInteger(assemblyGeneration) ||
    (assemblyGeneration as number) < 0
  ) {
    throw new Error(
      "websocket generation lifecycle tuple assemblyGeneration must be a non-negative safe integer",
    );
  }
  return {
    routerSessionId: requirePattern(
      value,
      "routerSessionId",
      /^skiff-router-session-v1:opaque:[A-Za-z0-9._:-]+$/,
    ),
    serviceId: requirePublicationId(value, "serviceId"),
    assemblyIdentity: requirePattern(
      value,
      "assemblyIdentity",
      /^skiff-runtime-assembly-v1:sha256:[0-9a-f]{64}$/,
    ),
    assemblyGeneration: assemblyGeneration as number,
    websocketEntryId: requirePattern(
      value,
      "websocketEntryId",
      /^skiff-websocket-entry-v1:sha256:[0-9a-f]{64}$/,
    ),
    connectionId: requirePattern(
      value,
      "connectionId",
      /^(?=.{1,255}$)[A-Za-z0-9._:~-]+$/,
    ),
  };
}

function requireExactFields(
  value: Record<string, unknown>,
  expected: readonly string[],
): void {
  const actual = Object.keys(value).sort();
  const required = [...expected].sort();
  if (
    actual.length !== required.length ||
    actual.some((field, index) => field !== required[index])
  ) {
    throw new Error(
      `websocket generation lifecycle fields must be exactly ${required.join(", ")}`,
    );
  }
}

function requireNonEmptyString(
  value: Record<string, unknown>,
  field: string,
): string {
  const result = value[field];
  if (typeof result !== "string" || result.length === 0) {
    throw new Error(`websocket generation lifecycle ${field} must be a non-empty string`);
  }
  return result;
}

function requirePattern(
  value: Record<string, unknown>,
  field: string,
  pattern: RegExp,
): string {
  const result = requireNonEmptyString(value, field);
  if (!pattern.test(result)) {
    throw new Error(`websocket generation lifecycle ${field} is invalid`);
  }
  return result;
}

function requirePublicationId(
  value: Record<string, unknown>,
  field: string,
): string {
  const result = requireNonEmptyString(value, field);
  if (!isPublicationId(result)) {
    throw new Error(
      `websocket generation lifecycle ${field} must be a publication id`,
    );
  }
  return result;
}

function requireEnum<const T extends readonly string[]>(
  value: Record<string, unknown>,
  field: string,
  allowed: T,
): T[number] {
  const result = value[field];
  if (
    typeof result !== "string" ||
    !(allowed as readonly string[]).includes(result)
  ) {
    throw new Error(
      `websocket generation lifecycle ${field} must be one of ${allowed.join(", ")}`,
    );
  }
  return result as T[number];
}

function requireDirection(
  actual: WebSocketGenerationLifecycleDirection,
  expected: WebSocketGenerationLifecycleDirection,
  action: string,
): void {
  if (actual !== expected) {
    throw new Error(
      `websocket generation lifecycle ${action} is invalid for ${actual} direction`,
    );
  }
}

function requireSender<const TExpected extends WebSocketGenerationLifecycleSender>(
  actual: WebSocketGenerationLifecycleSender,
  expected: TExpected,
  action: string,
): asserts actual is TExpected {
  if (actual !== expected) {
    throw new Error(
      `websocket generation lifecycle ${action} sender must be ${expected}`,
    );
  }
}

function tuplesEqual(
  left: WebSocketGenerationLifecycleTuple,
  right: WebSocketGenerationLifecycleTuple,
): boolean {
  return (
    left.routerSessionId === right.routerSessionId &&
    left.serviceId === right.serviceId &&
    left.assemblyIdentity === right.assemblyIdentity &&
    left.assemblyGeneration === right.assemblyGeneration &&
    left.websocketEntryId === right.websocketEntryId &&
    left.connectionId === right.connectionId
  );
}
