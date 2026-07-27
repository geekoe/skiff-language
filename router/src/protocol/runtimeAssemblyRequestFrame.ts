import {
  BinaryFrameDecodeError,
  decodeBinaryFrameParts,
  encodeBinaryFrame,
  isRecord,
  type BinaryFrame,
} from "./envelope.js";
import { decodeRuntimeAssemblyRequestJson } from "./runtimeAssemblyRequestJson.js";
import { validateRuntimeAssemblyRequestStartFrameWireHeader } from "./runtimeProtocol.js";
import type { RuntimeAssemblyRequestStartFrameTransportWireHeader } from "./runtimeAssemblyRequest.js";

const RUNTIME_ASSEMBLY_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES = 1024 * 1024;

export function encodeRuntimeAssemblyRequestStartFrame(
  header: RuntimeAssemblyRequestStartFrameTransportWireHeader,
  payloadBytes: Uint8Array = new Uint8Array(),
): Buffer {
  const result = validateRuntimeAssemblyRequestStartFrameWireHeader(header);
  if (!result.ok) throw new BinaryFrameDecodeError(result.error);
  validateRuntimeAssemblyRequestPayload(result.envelope, payloadBytes);
  return encodeBinaryFrame(
    result.envelope as RuntimeAssemblyRequestStartFrameTransportWireHeader &
      Record<string, unknown>,
    payloadBytes,
  );
}

export function decodeRuntimeAssemblyRequestStartFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): BinaryFrame<
  RuntimeAssemblyRequestStartFrameTransportWireHeader & Record<string, unknown>
> {
  const frame = decodeBinaryFrameParts(input);
  const header = decodeRuntimeAssemblyRequestJson(frame.headerBytes);
  if (!isRecord(header)) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly request.start frame: header must be an object",
    );
  }
  const result = validateRuntimeAssemblyRequestStartFrameWireHeader(header);
  if (!result.ok) throw new BinaryFrameDecodeError(result.error);
  validateRuntimeAssemblyRequestPayload(result.envelope, frame.payloadBytes);
  return {
    header:
      result.envelope as RuntimeAssemblyRequestStartFrameTransportWireHeader &
      Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}

function validateRuntimeAssemblyRequestPayload(
  header: RuntimeAssemblyRequestStartFrameTransportWireHeader,
  payloadBytes: Uint8Array,
): void {
  if (header.routing.ingress.protocol !== "webSocket") return;
  if (header.routing.ingress.method === null) {
    if (payloadBytes.byteLength !== 0) {
      throw new BinaryFrameDecodeError(
        "invalid runtimeAssembly websocketConnect request.start frame: payload must be empty",
      );
    }
    return;
  }
  if (
    payloadBytes.byteLength === 0 ||
    payloadBytes.byteLength >
      RUNTIME_ASSEMBLY_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES
  ) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketJsonRpc request.start frame: payload must be present and within the payload limit",
    );
  }
}
