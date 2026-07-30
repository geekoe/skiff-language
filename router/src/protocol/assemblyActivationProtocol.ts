import {
  activationEnvironment,
  activationGeneration,
  activationToken,
  expectedActivationGeneration,
  runtimeAssemblyIdentity,
} from "./assemblyActivationLexical.js";

export const ASSEMBLY_ACTIVATION_CONTROL_ENDPOINT =
  "POST /__skiff/activate-assembly" as const;
export const ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION =
  "skiff-assembly-activation-request-v2" as const;
export const ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION =
  "skiff-environment-activation-state-v2" as const;

export type RuntimeAssemblyRef = Readonly<{
  assemblyIdentity: string;
}>;

export type RuntimeConfigSnapshotRef = Readonly<{
  snapshotId: string;
}>;

export type AssemblyActivationServiceDb = Readonly<{
  mongoUrl: string;
}>;

export type AssemblyActivationRequest = Readonly<{
  schemaVersion: typeof ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION;
  environment: string;
  activationId: string;
  expectedGeneration: number;
  assembly: RuntimeAssemblyRef;
  configSnapshot: RuntimeConfigSnapshotRef;
}>;

export type CommittedActivation = Readonly<{
  generation: number;
  assembly: RuntimeAssemblyRef;
  configSnapshot: RuntimeConfigSnapshotRef;
}>;

export type PendingActivation = Readonly<{
  activationId: string;
  expectedGeneration: number;
  candidateGeneration: number;
  assembly: RuntimeAssemblyRef;
  configSnapshot: RuntimeConfigSnapshotRef;
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

type ProvisioningTransitionType = "prepare" | "commit";
type ResponseTransitionType = "prepared" | "abort";

export type AssemblyActivationControl =
  | Readonly<{
      type: ProvisioningTransitionType;
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      configSnapshot: RuntimeConfigSnapshotRef;
      replicaId: string;
      serviceDb?: AssemblyActivationServiceDb;
    }>
  | Readonly<{
      type: ResponseTransitionType;
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      configSnapshot: RuntimeConfigSnapshotRef;
      replicaId: string;
    }>
  | Readonly<{
      type: "reject";
      environment: string;
      activationId: string;
      expectedGeneration: number;
      candidateGeneration: number;
      assembly: RuntimeAssemblyRef;
      configSnapshot: RuntimeConfigSnapshotRef;
      replicaId: string;
      reason: AssemblyActivationRejectReason;
    }>
  | Readonly<{
      type: "register";
      environment: string;
      generation: number;
      assembly: RuntimeAssemblyRef;
      configSnapshot: RuntimeConfigSnapshotRef;
      replicaId: string;
    }>;

const transitionFields = [
  "type",
  "environment",
  "activationId",
  "expectedGeneration",
  "candidateGeneration",
  "assembly",
  "configSnapshot",
  "replicaId",
] as const;
const provisioningTransitionFields = [...transitionFields, "serviceDb"] as const;
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
      "configSnapshot",
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
    expectedGeneration: requiredExpectedGeneration(
      value,
      "expectedGeneration",
    ),
    assembly: decodeAssemblyRef(value.assembly),
    configSnapshot: decodeConfigSnapshotRef(value.configSnapshot),
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
      ["type", "environment", "generation", "assembly", "configSnapshot", "replicaId"],
      "assembly activation control",
    );
    return {
      type,
      environment: requiredEnvironment(value, "environment"),
      generation: requiredGeneration(value, "generation"),
      assembly: decodeAssemblyRef(value.assembly),
      configSnapshot: decodeConfigSnapshotRef(value.configSnapshot),
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
  const provisioning = type === "prepare" || type === "commit";
  exactOptionalFields(
    value,
    provisioning ? provisioningTransitionFields : transitionFields,
    provisioning ? ["serviceDb"] : [],
    "assembly activation control",
  );
  return {
    ...decodeTransition(value, type),
    ...(provisioning && value.serviceDb !== undefined
      ? { serviceDb: decodeServiceDb(value.serviceDb) }
      : {}),
  };
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
  exactFields(
    value,
    ["generation", "assembly", "configSnapshot"],
    "committed activation",
  );
  return {
    generation: requiredGeneration(value, "generation"),
    assembly: decodeAssemblyRef(value.assembly),
    configSnapshot: decodeConfigSnapshotRef(value.configSnapshot),
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
      "configSnapshot",
      "participantReplicaIds",
    ],
    "pending activation",
  );
  const expectedGeneration = requiredExpectedGeneration(
    value,
    "expectedGeneration",
  );
  const candidateGeneration = requiredGeneration(value, "candidateGeneration");
  requireCandidateGeneration(expectedGeneration, candidateGeneration);
  return {
    activationId: requiredToken(value, "activationId"),
    expectedGeneration,
    candidateGeneration,
    assembly: decodeAssemblyRef(value.assembly),
    configSnapshot: decodeConfigSnapshotRef(value.configSnapshot),
    participantReplicaIds: decodeParticipantReplicaIds(
      value.participantReplicaIds,
    ),
  };
}

function decodeTransition<
  T extends ProvisioningTransitionType | ResponseTransitionType | "reject",
>(
  value: Record<string, unknown>,
  type: T,
) {
  const expectedGeneration = requiredExpectedGeneration(
    value,
    "expectedGeneration",
  );
  const candidateGeneration = requiredGeneration(value, "candidateGeneration");
  requireCandidateGeneration(expectedGeneration, candidateGeneration);
  return {
    type,
    environment: requiredEnvironment(value, "environment"),
    activationId: requiredToken(value, "activationId"),
    expectedGeneration,
    candidateGeneration,
    assembly: decodeAssemblyRef(value.assembly),
    configSnapshot: decodeConfigSnapshotRef(value.configSnapshot),
    replicaId: requiredToken(value, "replicaId"),
  };
}

function decodeServiceDb(input: unknown): AssemblyActivationServiceDb {
  const value = exactObject(input, "serviceDb");
  exactFields(value, ["mongoUrl"], "serviceDb");
  const mongoUrl = requiredString(value, "mongoUrl");
  if (mongoUrl.trim().length === 0) {
    throw new Error("serviceDb.mongoUrl must be a non-empty string");
  }
  return { mongoUrl };
}

function decodeAssemblyRef(input: unknown): RuntimeAssemblyRef {
  const value = exactObject(input, "runtime assembly ref");
  exactFields(value, ["assemblyIdentity"], "runtime assembly ref");
  const assemblyIdentity = runtimeAssemblyIdentity(value.assemblyIdentity);
  return { assemblyIdentity };
}

function decodeConfigSnapshotRef(input: unknown): RuntimeConfigSnapshotRef {
  const value = exactObject(input, "runtime config snapshot ref");
  exactFields(value, ["snapshotId"], "runtime config snapshot ref");
  const snapshotId = requiredString(value, "snapshotId");
  if (
    !/^skiff-runtime-config-snapshot-v1:[0-9a-f]{32}$/.test(snapshotId)
  ) {
    throw new Error(
      "snapshotId must use skiff-runtime-config-snapshot-v1:<32 lowercase hex>",
    );
  }
  return { snapshotId };
}

function decodeParticipantReplicaIds(input: unknown): readonly string[] {
  if (!Array.isArray(input) || input.length === 0) {
    throw new Error("participantReplicaIds must be a non-empty array");
  }
  for (let index = 0; index < input.length; index += 1) {
    if (!Object.hasOwn(input, index)) {
      throw new Error("participantReplicaIds must be a dense array");
    }
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

function exactOptionalFields(
  value: Record<string, unknown>,
  allowed: readonly string[],
  optional: readonly string[],
  label: string,
) {
  const required = allowed.filter((field) => !optional.includes(field));
  const actual = Object.keys(value);
  if (
    required.some((field) => !Object.hasOwn(value, field)) ||
    actual.some((field) => !allowed.includes(field))
  ) {
    throw new Error(
      `${label} fields must be exactly ${required.join(",")} with optional ${optional.join(",")}; got ${actual.sort().join(",")}`,
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
  return activationToken(value, label);
}

function requiredEnvironment(
  value: Record<string, unknown>,
  field: string,
): string {
  return activationEnvironment(value[field], field);
}

function requiredGeneration(
  value: Record<string, unknown>,
  field: string,
): number {
  return activationGeneration(value[field], field);
}

function requiredExpectedGeneration(
  value: Record<string, unknown>,
  field: string,
): number {
  return expectedActivationGeneration(value[field], field);
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
