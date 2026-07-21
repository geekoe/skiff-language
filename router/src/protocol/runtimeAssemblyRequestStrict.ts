import { isRecord } from "./envelope.js";

export function firstUnsupportedField(
  value: Record<string, unknown>,
  allowed: ReadonlySet<string>,
): string | undefined {
  return Object.keys(value).find((field) => !allowed.has(field));
}
export function firstMissingField(
  value: Record<string, unknown>,
  required: ReadonlySet<string>,
): string | undefined {
  return [...required].find(
    (field) => !Object.prototype.hasOwnProperty.call(value, field),
  );
}

export function rejectUnknownObjectFields(
  input: unknown,
  allowed: ReadonlySet<string>,
  label: string,
): string | null {
  if (!isRecord(input)) return null;
  const unsupported = firstUnsupportedField(input, allowed);
  return unsupported === undefined
    ? null
    : `invalid request.start envelope: ${label}.${unsupported} is not supported`;
}
