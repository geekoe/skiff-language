import type { JsonSchema, OperationParameterManifest } from '../../src/manifest/types.js';
import { isRecord } from '../../src/protocol/envelope.js';
import {
  decodeRuntimePayload,
  encodeRuntimePayload,
  RuntimePayloadCodecError
} from '../../../scripts/lib/runtime-payload-codec.mjs';

export {
  decodeRuntimePayload,
  encodeRuntimePayload,
  RuntimePayloadCodecError
} from '../../../scripts/lib/runtime-payload-codec.mjs';

export function operationArgsSchema(
  parameters: readonly OperationParameterManifest[]
): JsonSchema {
  const properties: Record<string, JsonSchema> = {};
  for (const parameter of parameters) {
    properties[parameter.name] = parameter.schema;
  }
  return {
    type: 'object',
    properties,
    required: parameters.map((parameter) => parameter.name),
    additionalProperties: false
  };
}

export function encodeOperationPayload(
  args: Record<string, unknown>,
  parameters: readonly OperationParameterManifest[]
): Buffer {
  return encodeRuntimePayload(args, operationArgsSchema(parameters));
}

export function decodeOperationPayload(
  payloadBytes: Uint8Array,
  parameters: readonly OperationParameterManifest[]
): Record<string, unknown> {
  const decoded = decodeRuntimePayload(payloadBytes, operationArgsSchema(parameters));
  if (!isRecord(decoded)) {
    throw new RuntimePayloadCodecError('operation payload must decode to an args object');
  }
  return decoded;
}
