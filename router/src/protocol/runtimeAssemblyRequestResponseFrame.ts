import {
  BinaryFrameDecodeError,
  decodeBinaryFrameParts,
  isRecord,
  type BinaryFrame,
  type RuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
} from "./envelope.js";
import { decodeRuntimeAssemblyWireJson } from "./runtimeAssemblyRequestJson.js";
import { validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader } from "./runtimeProtocol.js";

export function decodeRuntimeAssemblyWebSocketConnectResponseEndFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): BinaryFrame<
  RuntimeAssemblyWebSocketConnectResponseEndFrameHeader &
    Record<string, unknown>
> {
  const frame = decodeBinaryFrameParts(input);
  const header = decodeRuntimeAssemblyWireJson(
    frame.headerBytes,
    "runtimeAssembly websocketConnect response.end",
  );
  if (!isRecord(header)) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketConnect response.end frame: header must be an object",
    );
  }
  const result =
    validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader(header);
  if (!result.ok) throw new BinaryFrameDecodeError(result.error);
  if (frame.payloadBytes.byteLength !== 0) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketConnect response.end frame: payload must be empty",
    );
  }
  return {
    header: result.envelope as RuntimeAssemblyWebSocketConnectResponseEndFrameHeader &
      Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}
