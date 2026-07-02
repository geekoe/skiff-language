import {
  identityHash,
  identityHashWithLabel,
  serviceBuildIdHash,
  validateDevReloadContractHash,
} from "./identity.js";
import {
  assertRecord,
  readOptionalRecord,
  readOptionalString,
  readRequiredString,
} from "./readUtils.js";
import type {
  ArtifactPointer,
  ArtifactPointerInput,
  ServiceVersionBuildBinding,
} from "./types.js";

const SERVICE_VERSION_POINTER_SCHEMA_VERSION =
  "skiff-service-version-pointer-v1";
const SERVICE_BUILD_SCHEMA_VERSION = "skiff-service-build-v1";
const SERVICE_BUILD_ID_PREFIX = "skiff-service-build-v1";

export function readDevReloadPointer(
  value: unknown,
  pointerPath: string,
): ArtifactPointer {
  assertRecord(value, `${pointerPath} dev reload pointer`);
  rejectUnsupportedPointerAliases(value, pointerPath);
  const mode = readRequiredString(value.mode, `${pointerPath} mode`);
  if (mode !== "dev") {
    throw new Error(`${pointerPath} mode must be dev`);
  }
  readRequiredString(value.profile, `${pointerPath} profile`);
  const contractHash = readRequiredString(
    value.contractHash,
    `${pointerPath} contractHash`,
  );
  const protocolIdentity = readRequiredString(
    value.protocolIdentity,
    `${pointerPath} protocolIdentity`,
  );
  validateDevReloadContractHash(contractHash, protocolIdentity, pointerPath);
  const serviceAssembly = readServiceAssemblyPointer(
    value.serviceAssembly,
    pointerPath,
  );
  if (serviceAssembly.assemblyIdentity === undefined) {
    throw new Error(
      `${pointerPath} serviceAssembly.assemblyIdentity is required`,
    );
  }
  const buildId = readRequiredString(value.buildId, `${pointerPath} buildId`);
  serviceBuildIdHash(buildId, `${pointerPath} buildId`);
  const expectedBuildId = `${SERVICE_BUILD_ID_PREFIX}:sha256:${identityHashWithLabel(
    serviceAssembly.assemblyIdentity,
    "serviceAssembly",
  )}`;
  if (buildId !== expectedBuildId) {
    throw new Error(
      `${pointerPath} buildId must match serviceAssembly.assemblyIdentity`,
    );
  }
  return definedPointer({
    buildId,
    indexPath: pointerPath,
    contractIdentity: protocolIdentity,
    fingerprint: readOptionalString(value.fingerprint),
    generation:
      readOptionalString(value.generation) ??
      readOptionalString(value.revision) ??
      readOptionalString(value.version),
    serviceAssembly: serviceAssembly.path,
    serviceAssemblyIdentity: serviceAssembly.assemblyIdentity,
    serviceUnit: readServiceUnitPath(value, pointerPath),
    serviceId: readRequiredString(value.serviceId, `${pointerPath} serviceId`),
  });
}

export function readServiceVersionPointer(
  value: unknown,
  pointerPath: string,
): ServiceVersionBuildBinding {
  assertRecord(value, `${pointerPath} service version pointer`);
  const schemaVersion = readOptionalString(value.schemaVersion);
  if (schemaVersion !== SERVICE_VERSION_POINTER_SCHEMA_VERSION) {
    throw new Error(
      `${pointerPath} schemaVersion must be ${SERVICE_VERSION_POINTER_SCHEMA_VERSION}`,
    );
  }
  const serviceId = readRequiredString(
    value.serviceId,
    `${pointerPath} serviceId`,
  );
  const version = readRequiredString(value.version, `${pointerPath} version`);
  const buildId = readRequiredString(value.buildId, `${pointerPath} buildId`);
  serviceBuildIdHash(buildId, `${pointerPath} buildId`);
  return { buildId, serviceId, version };
}

export function readBuildRecordPointer(
  value: unknown,
  buildPath: string,
  serviceVersion: ServiceVersionBuildBinding,
): ArtifactPointer {
  assertRecord(value, `${buildPath} build record`);
  rejectUnsupportedPointerAliases(value, buildPath);
  const schemaVersion = readOptionalString(value.schemaVersion);
  if (schemaVersion !== SERVICE_BUILD_SCHEMA_VERSION) {
    throw new Error(
      `${buildPath} schemaVersion must be ${SERVICE_BUILD_SCHEMA_VERSION}`,
    );
  }
  const serviceId = readRequiredString(
    value.serviceId,
    `${buildPath} serviceId`,
  );
  const buildServiceVersion = readRequiredString(
    value.serviceVersion,
    `${buildPath} serviceVersion`,
  );
  const buildId = readRequiredString(value.buildId, `${buildPath} buildId`);
  serviceBuildIdHash(buildId, `${buildPath} buildId`);
  if (serviceId !== serviceVersion.serviceId) {
    throw new Error(
      `${buildPath} serviceId must match service version pointer serviceId`,
    );
  }
  if (buildServiceVersion !== serviceVersion.version) {
    throw new Error(
      `${buildPath} serviceVersion must match service version pointer version`,
    );
  }
  if (buildId !== serviceVersion.buildId) {
    throw new Error(
      `${buildPath} buildId must match service version pointer buildId`,
    );
  }
  rejectLegacyContractIdentityAliases(value, buildPath);
  const contractIdentity = readOptionalString(value.contractIdentity);
  if (contractIdentity !== undefined) {
    identityHash(contractIdentity);
  }
  const serviceAssembly = readServiceAssemblyPointer(
    value.serviceAssembly,
    buildPath,
  );
  return definedPointer({
    buildId,
    contractIdentity,
    fingerprint: readOptionalString(value.fingerprint) ?? buildId,
    generation:
      readOptionalString(value.generation) ??
      readOptionalString(value.revision) ??
      readOptionalString(value.version),
    indexPath: buildPath,
    serviceVersion: serviceVersion.version,
    serviceAssembly: serviceAssembly.path,
    serviceAssemblyIdentity: serviceAssembly.assemblyIdentity,
    serviceUnit: readServiceUnitPath(value, buildPath),
    serviceId,
  });
}

function rejectLegacyContractIdentityAliases(
  value: Record<string, unknown>,
  buildPath: string,
): void {
  if ("protocolIdentity" in value) {
    throw new Error(
      `${buildPath} protocolIdentity is not supported; use contractIdentity`,
    );
  }
  if ("serviceProtocolIdentity" in value) {
    throw new Error(
      `${buildPath} serviceProtocolIdentity is not supported; use contractIdentity`,
    );
  }
}

function rejectUnsupportedPointerAliases(
  value: Record<string, unknown>,
  indexPath: string,
): void {
  const artifactIdentity = readOptionalString(value.artifactIdentity);
  if (
    "serviceIr" in value ||
    "serviceIrPath" in value ||
    artifactIdentity?.startsWith("skiff-service-ir-v1")
  ) {
    throw new Error(`${indexPath} legacy serviceIr pointers are not supported`);
  }
  if ("artifactIdentity" in value) {
    throw new Error(
      `${indexPath} artifactIdentity is not supported in artifact pointers`,
    );
  }
  if ("serviceAssemblyRef" in value) {
    throw new Error(
      `${indexPath} serviceAssemblyRef is not supported in artifact pointers`,
    );
  }
}

function readServiceAssemblyPointer(
  value: unknown,
  indexPath: string,
): { path?: string; assemblyIdentity?: string } {
  if (value === undefined) {
    return {};
  }
  const object = readOptionalRecord(value);
  if (!object) {
    throw new Error(`${indexPath} serviceAssembly must be an object`);
  }
  for (const key of ["path", "artifactPath", "identity", "artifactIdentity"]) {
    if (key in object) {
      throw new Error(`${indexPath} serviceAssembly.${key} is not supported`);
    }
  }
  const path = readOptionalString(object.assemblyPath);
  if (path === undefined) {
    throw new Error(`${indexPath} serviceAssembly.assemblyPath is required`);
  }
  const assemblyIdentity = readOptionalString(object.assemblyIdentity);
  if (assemblyIdentity === undefined) {
    throw new Error(
      `${indexPath} serviceAssembly.assemblyIdentity is required`,
    );
  }
  if (assemblyIdentity?.startsWith("skiff-service-ir-v1")) {
    throw new Error(`${indexPath} legacy serviceIr pointers are not supported`);
  }
  return {
    path,
    ...(assemblyIdentity !== undefined ? { assemblyIdentity } : {}),
  };
}

function readServiceUnitPath(
  value: Record<string, unknown>,
  indexPath: string,
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
    throw new Error(
      `${indexPath} serviceUnit requires unitPath/artifactPath/path`,
    );
  }
  return path;
}

function definedPointer(pointer: ArtifactPointerInput): ArtifactPointer {
  const result: ArtifactPointer = {
    indexPath: pointer.indexPath,
  };
  for (const key of [
    "buildId",
    "contractIdentity",
    "fingerprint",
    "generation",
    "serviceVersion",
    "serviceAssembly",
    "serviceAssemblyIdentity",
    "serviceUnit",
    "serviceId",
  ] as const) {
    const value = pointer[key];
    if (value !== undefined) {
      result[key] = value;
    }
  }
  return result;
}
