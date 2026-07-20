export const ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT =
  "POST /__skiff/activate-assembly" as const;
export const ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION =
  "skiff-assembly-activation-request-v1" as const;
export const ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION =
  "skiff-environment-activation-state-v1" as const;

const runtimeAssemblyIdentityPattern =
  /^skiff-runtime-assembly-v1:sha256:[0-9a-f]{64}$/u;

export type RuntimeAssemblyRef = Readonly<{
  assemblyIdentity: string;
}>;

export type AssemblyActivationRequest = Readonly<{
  schemaVersion: typeof ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION;
  environment: string;
  activationId: string;
  expectedGeneration: number;
  assembly: RuntimeAssemblyRef;
}>;

export type CommittedActivation = Readonly<{
  generation: number;
  assembly: RuntimeAssemblyRef;
}>;

export type PendingActivation = Readonly<{
  activationId: string;
  expectedGeneration: number;
  candidateGeneration: number;
  assembly: RuntimeAssemblyRef;
  participantReplicaIds: readonly string[];
}>;

export type EnvironmentActivationState = Readonly<{
  schemaVersion: typeof ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION;
  environment: string;
  committed: CommittedActivation;
  pending: PendingActivation | null;
}>;

export type AssemblyActivationRejectReason =
  | "resolve"
  | "load"
  | "link"
  | "admission"
  | "participantDisconnected";

type TransitionType = "prepare" | "prepared" | "commit" | "abort";

export type AssemblyActivationControl =
  | Readonly<{
      type: TransitionType;
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      replicaId: string;
    }>
  | Readonly<{
      type: "reject";
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      replicaId: string;
      reason: AssemblyActivationRejectReason;
    }>
  | Readonly<{
      type: "register";
      environment: string;
      generation: number;
      assembly: RuntimeAssemblyRef;
      replicaId: string;
    }>;

const transitionFields = [
  "type",
  "environment",
  "activationId",
  "expectedGeneration",
  "candidateGeneration",
  "assembly",
  "replicaId",
] as const;
const rejectReasons = new Set<AssemblyActivationRejectReason>([
  "resolve",
  "load",
  "link",
  "admission",
  "participantDisconnected",
]);

export function decodeAssemblyActivationRequest(
  input: unknown,
): AssemblyActivationRequest {
  const value = exactObject(input, "assembly activation request");
  exactFields(
    value,
    [
      "schemaVersion",
      "environment",
      "activationId",
      "expectedGeneration",
      "assembly",
    ],
    "assembly activation request",
  );
  const schemaVersion = requiredString(value, "schemaVersion");
  if (schemaVersion !== ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION) {
    throw new Error(
      `schemaVersion must be ${ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION}`,
    );
  }
  return {
    schemaVersion,
    environment: requiredEnvironment(value, "environment"),
    activationId: requiredToken(value, "activationId"),
    expectedGeneration: requiredGeneration(value, "expectedGeneration"),
    assembly: decodeAssemblyRef(value.assembly),
  };
}

export function decodeEnvironmentActivationState(
  input: unknown,
): EnvironmentActivationState {
  const value = exactObject(input, "environment activation state");
  exactFields(
    value,
    ["schemaVersion", "environment", "committed", "pending"],
    "environment activation state",
  );
  const schemaVersion = requiredString(value, "schemaVersion");
  if (schemaVersion !== ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION) {
    throw new Error(
      `schemaVersion must be ${ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION}`,
    );
  }
  const committed = decodeCommittedActivation(value.committed);
  const pending =
    value.pending === null ? null : decodePendingActivation(value.pending);
  if (
    pending !== null &&
    pending.expectedGeneration !== committed.generation
  ) {
    throw new Error(
      "pending expectedGeneration must equal committed generation",
    );
  }
  return {
    schemaVersion,
    environment: requiredEnvironment(value, "environment"),
    committed,
    pending,
  };
}

export function decodeAssemblyActivationControl(
  input: unknown,
): AssemblyActivationControl {
  const value = exactObject(input, "assembly activation control");
  const type = requiredString(value, "type");
  if (type === "register") {
    exactFields(
      value,
      ["type", "environment", "generation", "assembly", "replicaId"],
      "assembly activation control",
    );
    return {
      type,
      environment: requiredEnvironment(value, "environment"),
      generation: requiredGeneration(value, "generation"),
      assembly: decodeAssemblyRef(value.assembly),
      replicaId: requiredToken(value, "replicaId"),
    };
  }
  if (type === "reject") {
    exactFields(
      value,
      [...transitionFields, "reason"],
      "assembly activation control",
    );
    const reason = requiredString(value, "reason");
    if (!rejectReasons.has(reason as AssemblyActivationRejectReason)) {
      throw new Error(`unknown assembly activation reject reason ${reason}`);
    }
    return {
      ...decodeTransition(value, type),
      type,
      reason: reason as AssemblyActivationRejectReason,
    };
  }
  if (
    type !== "prepare" &&
    type !== "prepared" &&
    type !== "commit" &&
    type !== "abort"
  ) {
    throw new Error(`unknown assembly activation control type ${type}`);
  }
  exactFields(value, transitionFields, "assembly activation control");
  return decodeTransition(value, type);
}

export function decodeAssemblyActivationControls(
  input: unknown,
): AssemblyActivationControl[] {
  if (!Array.isArray(input)) {
    throw new Error("assembly activation controls must be an array");
  }
  return input.map(decodeAssemblyActivationControl);
}

function decodeCommittedActivation(input: unknown): CommittedActivation {
  const value = exactObject(input, "committed activation");
  exactFields(value, ["generation", "assembly"], "committed activation");
  return {
    generation: requiredGeneration(value, "generation"),
    assembly: decodeAssemblyRef(value.assembly),
  };
}

function decodePendingActivation(input: unknown): PendingActivation {
  const value = exactObject(input, "pending activation");
  exactFields(
    value,
    [
      "activationId",
      "expectedGeneration",
      "candidateGeneration",
      "assembly",
      "participantReplicaIds",
    ],
    "pending activation",
  );
  const expectedGeneration = requiredGeneration(value, "expectedGeneration");
  const candidateGeneration = requiredGeneration(value, "candidateGeneration");
  requireCandidateGeneration(expectedGeneration, candidateGeneration);
  return {
    activationId: requiredToken(value, "activationId"),
    expectedGeneration,
    candidateGeneration,
    assembly: decodeAssemblyRef(value.assembly),
    participantReplicaIds: decodeParticipantReplicaIds(
      value.participantReplicaIds,
    ),
  };
}

function decodeTransition<T extends TransitionType | "reject">(
  value: Record<string, unknown>,
  type: T,
) {
  const expectedGeneration = requiredGeneration(value, "expectedGeneration");
  const candidateGeneration = requiredGeneration(value, "candidateGeneration");
  requireCandidateGeneration(expectedGeneration, candidateGeneration);
  return {
    type,
    environment: requiredEnvironment(value, "environment"),
    activationId: requiredToken(value, "activationId"),
    expectedGeneration,
    candidateGeneration,
    assembly: decodeAssemblyRef(value.assembly),
    replicaId: requiredToken(value, "replicaId"),
  };
}

function decodeAssemblyRef(input: unknown): RuntimeAssemblyRef {
  const value = exactObject(input, "runtime assembly ref");
  exactFields(value, ["assemblyIdentity"], "runtime assembly ref");
  const assemblyIdentity = requiredString(value, "assemblyIdentity");
  if (!runtimeAssemblyIdentityPattern.test(assemblyIdentity)) {
    throw new Error(
      "assemblyIdentity must be skiff-runtime-assembly-v1:sha256:<64 lowercase hex>",
    );
  }
  return { assemblyIdentity };
}

function decodeParticipantReplicaIds(input: unknown): readonly string[] {
  if (!Array.isArray(input) || input.length === 0) {
    throw new Error("participantReplicaIds must be a non-empty array");
  }
  const replicaIds = input.map((value, index) =>
    canonicalToken(value, `participantReplicaIds[${index}]`),
  );
  const unique = new Set(replicaIds);
  const sorted = [...replicaIds].sort((left, right) =>
    Buffer.compare(Buffer.from(left), Buffer.from(right)),
  );
  if (
    unique.size !== replicaIds.length ||
    sorted.some((value, index) => value !== replicaIds[index])
  ) {
    throw new Error(
      "participantReplicaIds must be non-empty, unique, and sorted",
    );
  }
  return replicaIds;
}

function exactObject(
  input: unknown,
  label: string,
): Record<string, unknown> {
  if (input === null || typeof input !== "object" || Array.isArray(input)) {
    throw new Error(`${label} must be an object`);
  }
  return input as Record<string, unknown>;
}

function exactFields(
  value: Record<string, unknown>,
  expected: readonly string[],
  label: string,
) {
  const actual = Object.keys(value).sort();
  const canonical = [...expected].sort();
  if (
    actual.length !== canonical.length ||
    actual.some((field, index) => field !== canonical[index])
  ) {
    throw new Error(
      `${label} fields must be exactly ${canonical.join(",")}; got ${actual.join(",")}`,
    );
  }
}

function requiredString(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== "string") {
    throw new Error(`${field} must be a string`);
  }
  return fieldValue;
}

function requiredToken(value: Record<string, unknown>, field: string): string {
  return canonicalToken(value[field], field);
}

function canonicalToken(value: unknown, label: string): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value !== value.trim() ||
    Buffer.byteLength(value, "utf8") > 200 ||
    /\p{Cc}/u.test(value)
  ) {
    throw new Error(
      `${label} must be non-empty, have no surrounding whitespace or control characters, and be at most 200 bytes`,
    );
  }
  return value;
}

function requiredEnvironment(
  value: Record<string, unknown>,
  field: string,
): string {
  const environment = requiredToken(value, field);
  if (
    environment === "." ||
    environment === ".." ||
    !/^[A-Za-z0-9._-]+$/u.test(environment)
  ) {
    throw new Error(
      `${field} must use only letters, digits, dot, dash, or underscore`,
    );
  }
  return environment;
}

function requiredGeneration(
  value: Record<string, unknown>,
  field: string,
): number {
  const fieldValue = value[field];
  if (
    typeof fieldValue !== "number" ||
    !Number.isSafeInteger(fieldValue) ||
    fieldValue < 0
  ) {
    throw new Error(`${field} must be a non-negative safe integer`);
  }
  return fieldValue;
}

function requireCandidateGeneration(
  expectedGeneration: number,
  candidateGeneration: number,
) {
  if (candidateGeneration !== expectedGeneration + 1) {
    throw new Error(
      "candidateGeneration must equal expectedGeneration + 1",
    );
  }
}
