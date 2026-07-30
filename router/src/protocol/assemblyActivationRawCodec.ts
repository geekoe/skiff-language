import {
  type AssemblyActivationControl,
  type AssemblyActivationRequest,
  decodeAssemblyActivationControl,
  decodeAssemblyActivationControls,
  decodeAssemblyActivationRequest,
  decodeEnvironmentActivationState,
  type EnvironmentActivationState,
} from "./assemblyActivationProtocol.js";
import {
  type ActivationJsonInput,
  parseStrictActivationJson,
} from "./strictActivationJson.js";

export function decodeRawAssemblyActivationRequest(
  input: ActivationJsonInput,
): AssemblyActivationRequest {
  return decodeAssemblyActivationRequest(parseStrictActivationJson(input));
}

export function decodeRawEnvironmentActivationState(
  input: ActivationJsonInput,
): EnvironmentActivationState {
  return decodeEnvironmentActivationState(parseStrictActivationJson(input));
}

export function decodeRawAssemblyActivationControl(
  input: ActivationJsonInput,
): AssemblyActivationControl {
  return decodeAssemblyActivationControl(parseStrictActivationJson(input));
}

export function decodeRawAssemblyActivationControls(
  input: ActivationJsonInput,
): AssemblyActivationControl[] {
  return decodeAssemblyActivationControls(parseStrictActivationJson(input));
}
