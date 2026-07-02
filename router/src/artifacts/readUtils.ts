import type { JsonSchema } from "../manifest/types.js";

export function readRequiredArray(value: unknown, name: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new Error(`${name} must be an array`);
  }
  return value;
}

export function readRequiredString(value: unknown, name: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${name} must be a non-empty string`);
  }
  return value;
}

export function readOptionalString(value: unknown): string | undefined {
  if (typeof value === "string" && value.length > 0) {
    return value;
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return String(value);
  }
  return undefined;
}

export function readOptionalRecord(
  value: unknown,
): Record<string, unknown> | undefined {
  return isRecord(value) ? value : undefined;
}

export function assertRecord(
  value: unknown,
  name: string,
): asserts value is Record<string, unknown> {
  if (!isRecord(value)) {
    throw new Error(`${name} must be an object`);
  }
}

export function readRequiredJsonSchema(
  value: unknown,
  label: string,
): JsonSchema {
  if (isJsonSchema(value)) {
    return value;
  }
  throw new Error(`${label} is required and must be a JSON schema`);
}

export function isJsonSchema(value: unknown): value is JsonSchema {
  return (
    isRecord(value) &&
    value.kind === undefined &&
    (typeof value.type === "string" ||
      Array.isArray(value.oneOf) ||
      typeof value.$ref === "string" ||
      typeof value.const === "string" ||
      typeof value.enum !== "undefined")
  );
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
