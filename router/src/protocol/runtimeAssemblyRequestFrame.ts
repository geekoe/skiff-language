import {
  BinaryFrameDecodeError,
  decodeBinaryFrameParts,
  isRecord,
  type BinaryFrame,
} from "./envelope.js";
import { decodeRuntimeAssemblyRequestJson } from "./runtimeAssemblyRequestJson.js";
import { validateRuntimeAssemblyRequestStartFrameHeader } from "./runtimeProtocol.js";
import type { RuntimeAssemblyRequestStartFrameHeader } from "./runtimeAssemblyRequest.js";

export function decodeRuntimeAssemblyRequestStartFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): BinaryFrame<RuntimeAssemblyRequestStartFrameHeader & Record<string, unknown>> {
  const frame = decodeBinaryFrameParts(input);
  const header = decodeRuntimeAssemblyRequestJson(frame.headerBytes);
  if (!isRecord(header)) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly request.start frame: header must be an object",
    );
  }
  const result = validateRuntimeAssemblyRequestStartFrameHeader(header);
  if (!result.ok) throw new BinaryFrameDecodeError(result.error);
  return {
    header: result.envelope as RuntimeAssemblyRequestStartFrameHeader &
      Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}
