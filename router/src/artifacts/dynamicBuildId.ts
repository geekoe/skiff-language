import { readJsonAtArtifactPath } from "./artifactPath.js";
import {
  assertRecord,
  readOptionalRecord,
  readOptionalString,
} from "./readUtils.js";
import {
  computeRuntimeProgramBuildIdWithIdentityCli,
  type IdentityCliResolutionOptions,
} from "./identityCli.js";
import type { ArtifactPointer } from "./types.js";

const SERVICE_UNIT_SCHEMA_VERSION = "skiff-service-unit-v1";

export interface RuntimeProgramServiceUnitArtifact {
  path: string;
  value: Record<string, unknown>;
}

export async function computeRuntimeProgramBuildId(input: {
  root: string;
  pointer: ArtifactPointer;
  serviceAssembly: Record<string, unknown>;
  serviceUnit?: RuntimeProgramServiceUnitArtifact;
} & IdentityCliResolutionOptions): Promise<string> {
  const serviceUnit =
    input.serviceUnit ??
    (await readRuntimeProgramServiceUnit({
      root: input.root,
      pointer: input.pointer,
      serviceAssembly: input.serviceAssembly,
    }));
  return computeRuntimeProgramBuildIdWithIdentityCli({
    artifactRoot: input.root,
    serviceUnit: serviceUnit.value,
    ...(input.identityCliPath !== undefined
      ? { identityCliPath: input.identityCliPath }
      : {}),
    ...(input.releaseMode !== undefined ? { releaseMode: input.releaseMode } : {}),
  });
}

export async function readRuntimeProgramServiceUnit(input: {
  root: string;
  pointer: ArtifactPointer;
  serviceAssembly: Record<string, unknown>;
}): Promise<RuntimeProgramServiceUnitArtifact> {
  const serviceUnitPath =
    input.pointer.serviceUnit ??
    serviceUnitPathFromRecord(
      input.serviceAssembly,
      `${input.pointer.indexPath} serviceAssembly`,
    );
  if (serviceUnitPath === undefined) {
    throw new Error(
      `${input.pointer.indexPath} does not declare serviceUnit/serviceUnitPath; router artifact loading requires typed ServiceUnit`,
    );
  }

  const serviceUnit = await readJsonAtArtifactPath(
    input.root,
    serviceUnitPath,
    input.pointer.indexPath,
  );
  assertRecord(serviceUnit, `${serviceUnitPath} service unit`);
  if (serviceUnit.schemaVersion !== SERVICE_UNIT_SCHEMA_VERSION) {
    throw new Error(
      `${serviceUnitPath} service unit schemaVersion must be ${SERVICE_UNIT_SCHEMA_VERSION}`,
    );
  }

  return {
    path: serviceUnitPath,
    value: serviceUnit,
  };
}

function serviceUnitPathFromRecord(
  value: Record<string, unknown>,
  label: string,
): string | undefined {
  const directPath = readOptionalString(value.serviceUnitPath);
  if (directPath !== undefined) {
    return directPath;
  }
  const serviceUnit = value.serviceUnit;
  const stringPath = readOptionalString(serviceUnit);
  if (stringPath !== undefined) {
    return stringPath;
  }
  const object = readOptionalRecord(serviceUnit);
  if (!object) {
    return undefined;
  }
  const path =
    readOptionalString(object.unitPath) ??
    readOptionalString(object.artifactPath) ??
    readOptionalString(object.path) ??
    readOptionalString(object.serviceUnitPath);
  if (path === undefined) {
    throw new Error(`${label}.serviceUnit requires unitPath/artifactPath/path`);
  }
  return path;
}
