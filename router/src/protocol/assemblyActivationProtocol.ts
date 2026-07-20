export type RuntimeAssemblyRef = Readonly<{
  assemblyIdentity: string;
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

export function decodeAssemblyActivationControl(
  input: unknown,
): AssemblyActivationControl {
  const value = exactObject(input, "assembly activation control");
  const type = requiredString(value, "type");
  if (type === "register") {
    exactFields(value, [
      "type",
      "environment",
      "generation",
      "assembly",
      "replicaId",
    ]);
    return {
      type,
      environment: requiredString(value, "environment"),
      generation: requiredGeneration(value, "generation"),
      assembly: decodeAssemblyRef(value.assembly),
      replicaId: requiredString(value, "replicaId"),
    };
  }
  if (type === "reject") {
    exactFields(value, [...transitionFields, "reason"]);
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
  exactFields(value, transitionFields);
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

function decodeTransition(
  value: Record<string, unknown>,
  type: TransitionType | "reject",
) {
  return {
    type,
    environment: requiredString(value, "environment"),
    activationId: requiredString(value, "activationId"),
    expectedGeneration: requiredGeneration(value, "expectedGeneration"),
    candidateGeneration: requiredGeneration(value, "candidateGeneration"),
    assembly: decodeAssemblyRef(value.assembly),
    replicaId: requiredString(value, "replicaId"),
  };
}

function decodeAssemblyRef(input: unknown): RuntimeAssemblyRef {
  const value = exactObject(input, "runtime assembly ref");
  exactFields(value, ["assemblyIdentity"]);
  return { assemblyIdentity: requiredString(value, "assemblyIdentity") };
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
) {
  const actual = Object.keys(value).sort();
  const canonical = [...expected].sort();
  if (
    actual.length !== canonical.length ||
    actual.some((field, index) => field !== canonical[index])
  ) {
    throw new Error(
      `assembly activation fields must be exactly ${canonical.join(",")}; got ${actual.join(",")}`,
    );
  }
}

function requiredString(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== "string" || fieldValue.trim() === "") {
    throw new Error(`${field} must be a non-empty string`);
  }
  return fieldValue;
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
