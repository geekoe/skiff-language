export class RuntimePayloadCodecError extends Error {
  constructor(message: string);
}

export function encodeRuntimePayload(value: unknown, schema: unknown): Buffer;

export function decodeRuntimePayload(payloadBytes: Uint8Array, schema: unknown): unknown;
