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
  validateCanonicalWebSocketPayload(result.envelope, frame.payloadBytes);
  return {
    header: result.envelope as RuntimeAssemblyRequestStartFrameHeader &
      Record<string, unknown>,
    payloadBytes: frame.payloadBytes,
  };
}

function validateCanonicalWebSocketPayload(
  header: RuntimeAssemblyRequestStartFrameHeader,
  payload: Uint8Array,
): void {
  if (header.routing.ingress.protocol !== "webSocket") return;
  const adapter = header.websocketAdapter!;
  if (adapter.kind === "connect") {
    if (payload.byteLength !== 0) {
      throw new BinaryFrameDecodeError(
        "canonical WebSocket connect payload must be empty",
      );
    }
    return;
  }
  const receive = adapter.receiveEvent!;
  const expectedKinds = receive.contextCodec === undefined
    ? ["websocket.message"]
    : ["websocket.context", "websocket.message"];
  if (receive.payloadSegments.length !== expectedKinds.length) {
    throw new BinaryFrameDecodeError(
      "canonical WebSocket receive payload segments do not match Context presence",
    );
  }
  let offset = 0;
  for (const [index, segment] of receive.payloadSegments.entries()) {
    if (segment.kind !== expectedKinds[index] || segment.offset !== offset) {
      throw new BinaryFrameDecodeError(
        "canonical WebSocket receive payload segments must be ordered and contiguous",
      );
    }
    offset += segment.length;
  }
  if (offset !== payload.byteLength) {
    throw new BinaryFrameDecodeError(
      "canonical WebSocket receive payload segments must cover the complete payload",
    );
  }
}
