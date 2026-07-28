export const MAX_SAFE_ACTIVATION_GENERATION = Number.MAX_SAFE_INTEGER;
export const MAX_EXPECTED_ACTIVATION_GENERATION =
  MAX_SAFE_ACTIVATION_GENERATION - 1;

const runtimeAssemblyIdentityPattern =
  /^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$/;
const environmentPattern = /^[A-Za-z0-9._-]{1,200}$/;
const visibleAsciiTokenPattern = /^[\x21-\x7e]{1,200}$/;

export function activationToken(value: unknown, label: string): string {
  if (typeof value !== "string" || !visibleAsciiTokenPattern.test(value)) {
    throw new Error(
      `${label} must be an ASCII visible token between 1 and 200 bytes`,
    );
  }
  return value;
}

export function activationEnvironment(
  value: unknown,
  label: string,
): string {
  if (
    typeof value !== "string" ||
    value === "." ||
    value === ".." ||
    !environmentPattern.test(value)
  ) {
    throw new Error(
      `${label} must be 1-200 ASCII letters, digits, dot, dash, or underscore and must not be . or ..`,
    );
  }
  return value;
}

export function activationGeneration(
  value: unknown,
  label: string,
): number {
  if (
    typeof value !== "number" ||
    Object.is(value, -0) ||
    !Number.isSafeInteger(value) ||
    value < 0
  ) {
    throw new Error(`${label} must be a canonical non-negative safe integer`);
  }
  return value;
}

export function expectedActivationGeneration(
  value: unknown,
  label: string,
): number {
  const generation = activationGeneration(value, label);
  if (generation > MAX_EXPECTED_ACTIVATION_GENERATION) {
    throw new Error(
      `${label} must not exceed ${MAX_EXPECTED_ACTIVATION_GENERATION}`,
    );
  }
  return generation;
}

export function runtimeAssemblyIdentity(value: unknown): string {
  if (
    typeof value !== "string" ||
    !runtimeAssemblyIdentityPattern.test(value)
  ) {
    throw new Error(
      "assemblyIdentity must be skiff-runtime-assembly-v3:sha256:<64 lowercase hex>",
    );
  }
  return value;
}
