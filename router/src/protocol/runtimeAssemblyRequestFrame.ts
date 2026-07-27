import {
  BinaryFrameDecodeError,
  decodeBinaryFrameParts,
  isRecord,
  type BinaryFrame,
} from "./envelope.js";
import { decodeRuntimeAssemblyRequestJson } from "./runtimeAssemblyRequestJson.js";
import { validateRuntimeAssemblyRequestStartFrameWireHeader } from "./runtimeProtocol.js";
import type { RuntimeAssemblyRequestStartFrameWireHeader } from "./runtimeAssemblyRequest.js";

export function decodeRuntimeAssemblyRequestStartFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): BinaryFrame<RuntimeAssemblyRequestStartFrameWireHeader & Record<string, unknown>> {
  const frame = decodeBinaryFrameParts(input);
  const header = decodeRuntimeAssemblyRequestJson(frame.headerBytes);
  if (!isRecord(header)) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly request.start frame: header must be an object",
    );
  }
  const result = validateRuntimeAssemblyRequestStartFrameWireHeader(header);
  if (!result.ok) throw new BinaryFrameDecodeError(result.error);
  if (
    result.envelope.routing.ingress.protocol === "webSocket" &&
    frame.payloadBytes.byteLength !== 0
  ) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketConnect request.start frame: payload must be empty",
    );
  }
  return {
    header: result.envelope as RuntimeAssemblyRequestStartFrameWireHeader &
      Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}
