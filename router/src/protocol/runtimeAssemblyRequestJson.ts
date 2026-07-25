import { parseStrictJson } from "./strictJson.js";

export function decodeRuntimeAssemblyRequestJson(input: Uint8Array): unknown {
  try {
    return parseStrictJson(input);
  } catch (error) {
    throw new Error("invalid runtimeAssembly request.start JSON", {
      cause: error,
    });
  }
}
