import { parseStrictJson } from "./strictJson.js";

export function decodeRuntimeAssemblyRequestJson(input: Uint8Array): unknown {
  return decodeRuntimeAssemblyWireJson(
    input,
    "runtimeAssembly request.start",
  );
}

export function decodeRuntimeAssemblyWireJson(
  input: Uint8Array,
  label: string,
): unknown {
  try {
    return parseStrictJson(input);
  } catch (error) {
    throw new Error(`invalid ${label} JSON`, {
      cause: error,
    });
  }
}
