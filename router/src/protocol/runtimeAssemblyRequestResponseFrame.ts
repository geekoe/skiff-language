import {
  BinaryFrameDecodeError,
  decodeBinaryFrameParts,
  encodeBinaryFrame,
  isRecord,
  type BinaryFrame,
  type RuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
  type RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
} from "./envelope.js";
import { decodeRuntimeAssemblyWireJson } from "./runtimeAssemblyRequestJson.js";
import {
  validateRuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
  validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
} from "./runtimeProtocol.js";

const RUNTIME_ASSEMBLY_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES = 1024 * 1024;

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

export function encodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
  header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
  payloadBytes: Uint8Array = new Uint8Array(),
): Buffer {
  const envelope = validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
    header,
    payloadBytes,
  );
  return encodeBinaryFrame(
    envelope as RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader &
      Record<string, unknown>,
    payloadBytes,
  );
}

export function decodeRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): BinaryFrame<
  RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader &
    Record<string, unknown>
> {
  const frame = decodeBinaryFrameParts(input);
  const header = decodeRuntimeAssemblyWireJson(
    frame.headerBytes,
    "runtimeAssembly websocketJsonRpc response.end",
  );
  if (!isRecord(header)) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketJsonRpc response.end frame: header must be an object",
    );
  }
  const envelope = validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
    header,
    frame.payloadBytes,
  );
  return {
    header:
      envelope as RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader &
        Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}

export function validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrame(
  header: unknown,
  payloadBytes: Uint8Array,
): RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader {
  const result =
    validateRuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader(header);
  if (!result.ok) throw new BinaryFrameDecodeError(result.error);
  validateRuntimeAssemblyWebSocketJsonRpcResponsePayload(
    result.envelope,
    payloadBytes,
  );
  return result.envelope;
}

function validateRuntimeAssemblyWebSocketJsonRpcResponsePayload(
  header: RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader,
  payloadBytes: Uint8Array,
): void {
  if (
    payloadBytes.byteLength >
    RUNTIME_ASSEMBLY_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES
  ) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketJsonRpc response.end frame: payload exceeds the payload limit",
    );
  }
  const expectedPayloadPresent =
    header.websocketJsonRpc.outcome === "success";
  if (
    header.payloadPresent !== expectedPayloadPresent ||
    (payloadBytes.byteLength > 0) !== expectedPayloadPresent
  ) {
    throw new BinaryFrameDecodeError(
      "invalid runtimeAssembly websocketJsonRpc response.end frame: payload presence must match outcome",
    );
  }
}
