import {
  isRecord,
  type RequestStartFrameHeader,
} from "./envelope.js";
import {
  activationGeneration,
  runtimeAssemblyIdentity,
} from "./assemblyActivationLexical.js";

type LegacyRoutingField =
  | "target"
  | "operationAbiId"
  | "selector"
  | "serviceId"
  | "version"
  | "buildId"
  | "serviceProtocolIdentity"
  | "assemblyIdentity"
  | "assemblyGeneration"
  | "contractOperationId"
  | "ingress";

export interface RuntimeAssemblyRequestRoutingFrameHeader {
  kind: "runtimeAssembly";
  assemblyIdentity: string;
  assemblyGeneration: number;
  contractOperationId: string;
  ingress: {
    protocol: "http" | "webSocket";
    host: string;
    method: string | null;
    path: string;
  };
}

export type RuntimeAssemblyRequestStartFrameHeader = Omit<
  RequestStartFrameHeader,
  LegacyRoutingField
> & {
  caller: {
    kind: "gateway";
    target: string;
  };
  routing: RuntimeAssemblyRequestRoutingFrameHeader;
};

const CONTRACT_OPERATION_IDENTITY_PATTERN =
  /^skiff-contract-operation-v1:sha256:[0-9a-f]{64}$/;

const canonicalHeaderFields = new Set([
  "schemaVersion",
  "type",
  "requestId",
  "mode",
  "caller",
  "routing",
  "activationIdentity",
  "gatewayEntryIdentity",
  "businessIdentity",
  "websocketEntryId",
  "clientSession",
  "deadline",
  "trace",
  "httpRequest",
  "httpAdapter",
  "websocketAdapter",
  "testEffectsEnabled",
  "testEffectDoubles",
]);

const routingFields = new Set([
  "kind",
  "assemblyIdentity",
  "assemblyGeneration",
  "contractOperationId",
  "ingress",
]);
const ingressFields = new Set(["protocol", "host", "method", "path"]);
const callerFields = new Set(["kind", "target"]);
const traceFields = new Set(["traceId", "spanId", "parentSpanId", "sampled"]);

export function hasRuntimeAssemblyRouting(
  envelope: Record<string, unknown>,
): boolean {
  return Object.prototype.hasOwnProperty.call(envelope, "routing");
}

export function validateRuntimeAssemblyRequestRouting(
  envelope: Record<string, unknown>,
): string | null {
  const unsupportedHeader = firstUnsupported(envelope, canonicalHeaderFields);
  if (unsupportedHeader !== undefined) {
    return `invalid request.start runtimeAssembly envelope: ${unsupportedHeader} is not supported`;
  }
  const callerError = rejectUnknownNestedFields(
    envelope.caller,
    callerFields,
    "caller",
  );
  if (callerError !== null) return callerError;
  const traceError = rejectUnknownNestedFields(
    envelope.trace,
    traceFields,
    "trace",
  );
  if (traceError !== null) return traceError;
  if (!isRecord(envelope.routing)) {
    return "invalid request.start envelope: routing must be an object";
  }
  const routing = envelope.routing;
  const unsupportedRouting = firstUnsupported(routing, routingFields);
  if (unsupportedRouting !== undefined) {
    return `invalid request.start envelope: routing.${unsupportedRouting} is not supported`;
  }
  const missingRouting = firstMissing(routing, routingFields);
  if (missingRouting !== undefined) {
    return `invalid request.start envelope: routing.${missingRouting} is required`;
  }
  if (routing.kind !== "runtimeAssembly") {
    return "invalid request.start envelope: routing.kind must be runtimeAssembly";
  }
  try {
    runtimeAssemblyIdentity(routing.assemblyIdentity);
  } catch {
    return "invalid request.start envelope: routing.assemblyIdentity must be skiff-runtime-assembly-v1:sha256:<64 lowercase hex>";
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
    typeof routing.contractOperationId !== "string" ||
    !CONTRACT_OPERATION_IDENTITY_PATTERN.test(routing.contractOperationId)
  ) {
    return "invalid request.start envelope: routing.contractOperationId must be skiff-contract-operation-v1:sha256:<64 lowercase hex>";
  }
  return validateIngress(routing.ingress);
}

function rejectUnknownNestedFields(
  input: unknown,
  allowed: ReadonlySet<string>,
  label: string,
): string | null {
  if (!isRecord(input)) return null;
  const unsupported = firstUnsupported(input, allowed);
  return unsupported === undefined
    ? null
    : `invalid request.start envelope: ${label}.${unsupported} is not supported`;
}

function validateIngress(input: unknown): string | null {
  if (!isRecord(input)) {
    return "invalid request.start envelope: routing.ingress must be an object";
  }
  const unsupported = firstUnsupported(input, ingressFields);
  if (unsupported !== undefined) {
    return `invalid request.start envelope: routing.ingress.${unsupported} is not supported`;
  }
  const missing = firstMissing(input, ingressFields);
  if (missing !== undefined) {
    return `invalid request.start envelope: routing.ingress.${missing} is required`;
  }
  const { protocol, host, method, path } = input;
  if (protocol !== "http" && protocol !== "webSocket") {
    return "invalid request.start envelope: routing.ingress.protocol must be http or webSocket";
  }
  if (
    typeof host !== "string" ||
    host.length === 0 ||
    typeof path !== "string" ||
    !path.startsWith("/")
  ) {
    return "invalid request.start envelope: routing.ingress must carry a non-empty host and absolute path";
  }
  if (
    (protocol === "http" && (typeof method !== "string" || method.length === 0)) ||
    (protocol === "webSocket" && method !== null)
  ) {
    return "invalid request.start envelope: routing.ingress.method does not match protocol";
  }
  return null;
}

function firstUnsupported(
  value: Record<string, unknown>,
  allowed: ReadonlySet<string>,
): string | undefined {
  return Object.keys(value).find((field) => !allowed.has(field));
}

function firstMissing(
  value: Record<string, unknown>,
  required: ReadonlySet<string>,
): string | undefined {
  return [...required].find(
    (field) => !Object.prototype.hasOwnProperty.call(value, field),
  );
}
