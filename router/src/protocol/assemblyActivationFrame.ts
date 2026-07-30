import {
  RUNTIME_FRAME_SCHEMA_VERSION,
  decodeBinaryFrame,
  encodeBinaryFrame,
  isRecord,
} from "./envelope.js";
import {
  type AssemblyActivationControl,
  decodeAssemblyActivationControl,
} from "./assemblyActivationProtocol.js";

export const ASSEMBLY_ACTIVATION_FRAME_TYPE = "assembly.activation" as const;

export type AssemblyActivationFrameDirection =
  | "routerToRuntime"
  | "runtimeToRouter";

export type AssemblyActivationFrameHeader = Readonly<{
  schemaVersion: typeof RUNTIME_FRAME_SCHEMA_VERSION;
  type: typeof ASSEMBLY_ACTIVATION_FRAME_TYPE;
  control: AssemblyActivationControl;
}>;

export function encodeAssemblyActivationFrame(
  direction: AssemblyActivationFrameDirection,
  controlInput: unknown,
): Buffer {
  const control = decodeAssemblyActivationControl(controlInput);
  validateDirection(direction, control);
  return encodeBinaryFrame({
    schemaVersion: RUNTIME_FRAME_SCHEMA_VERSION,
    type: ASSEMBLY_ACTIVATION_FRAME_TYPE,
    control,
  });
}

export function decodeAssemblyActivationFrame(
  direction: AssemblyActivationFrameDirection,
  input: Buffer | ArrayBuffer | Buffer[] | Uint8Array | string,
): AssemblyActivationControl {
  const frame = decodeBinaryFrame(input);
  if (frame.payloadBytes.byteLength !== 0) {
    throw new Error("assembly activation frame payload must be empty");
  }
  const header = exactHeader(frame.header);
  if (header.schemaVersion !== RUNTIME_FRAME_SCHEMA_VERSION) {
    throw new Error(
      `assembly activation frame schemaVersion must be ${RUNTIME_FRAME_SCHEMA_VERSION}`,
    );
  }
  if (header.type !== ASSEMBLY_ACTIVATION_FRAME_TYPE) {
    throw new Error(
      `assembly activation frame type must be ${ASSEMBLY_ACTIVATION_FRAME_TYPE}`,
    );
  }
  const control = decodeAssemblyActivationControl(header.control);
  validateDirection(direction, control);
  return control;
}

function exactHeader(input: unknown): Record<string, unknown> {
  if (!isRecord(input)) {
    throw new Error("assembly activation frame header must be an object");
  }
  const allowed = new Set(["schemaVersion", "type", "control"]);
  const unknown = Object.keys(input).find((field) => !allowed.has(field));
  if (unknown !== undefined) {
    throw new Error(`assembly activation frame header field ${unknown} is not supported`);
  }
  for (const field of allowed) {
    if (!Object.prototype.hasOwnProperty.call(input, field)) {
      throw new Error(`assembly activation frame header field ${field} is required`);
    }
  }
  return input;
}

function validateDirection(
  direction: AssemblyActivationFrameDirection,
  control: AssemblyActivationControl,
): void {
  if (direction !== "routerToRuntime" && direction !== "runtimeToRouter") {
    throw new Error(`unknown assembly activation frame direction ${direction}`);
  }
  const routerToRuntime =
    control.type === "prepare" ||
    control.type === "commit" ||
    control.type === "abort";
  const allowed =
    direction === "routerToRuntime" ? routerToRuntime : !routerToRuntime;
  if (!allowed) {
    throw new Error(
      `assembly activation control ${control.type} is invalid for ${direction} direction`,
    );
  }
}
