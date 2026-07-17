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
  PackageUnitArtifactPointer,
  ServiceUnitArtifactPointer,
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
  const fingerprint = readOptionalString(value.fingerprint);
  const serviceVersion = value.serviceVersion === undefined
    ? undefined
    : readRequiredString(
      value.serviceVersion,
      `${pointerPath} serviceVersion`,
    );
  const generation =
    readOptionalString(value.generation) ??
    readOptionalString(value.revision) ??
    readOptionalString(value.version);
  return definedPointer({
    buildId,
    indexPath: pointerPath,
    contractIdentity: protocolIdentity,
    ...(fingerprint !== undefined ? { fingerprint } : {}),
    ...(generation !== undefined ? { generation } : {}),
    ...(serviceVersion !== undefined ? { serviceVersion } : {}),
    serviceAssembly: serviceAssembly.path,
    serviceAssemblyIdentity: serviceAssembly.assemblyIdentity,
    serviceUnit: readServiceUnitPointer(value, pointerPath),
    serviceId: readRequiredString(value.serviceId, `${pointerPath} serviceId`),
    packageUnits: readPackageUnitPointers(value.packageUnits, pointerPath),
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
  const generation =
    readOptionalString(value.generation) ??
    readOptionalString(value.revision) ??
    readOptionalString(value.version);
  return definedPointer({
    buildId,
    ...(contractIdentity !== undefined ? { contractIdentity } : {}),
    fingerprint: readOptionalString(value.fingerprint) ?? buildId,
    ...(generation !== undefined ? { generation } : {}),
    indexPath: buildPath,
    serviceVersion: serviceVersion.version,
    serviceAssembly: serviceAssembly.path,
    serviceAssemblyIdentity: serviceAssembly.assemblyIdentity,
    serviceUnit: readServiceUnitPointer(value, buildPath),
    serviceId,
    packageUnits: readPackageUnitPointers(value.packageUnits, buildPath),
  });
}

function readPackageUnitPointers(
  value: unknown,
  pointerPath: string,
): PackageUnitArtifactPointer[] {
  if (value === undefined) {
    throw new Error(`${pointerPath} packageUnits is required`);
  }
  if (!Array.isArray(value)) {
    throw new Error(`${pointerPath} packageUnits must be an array`);
  }
  return value.map((item, index) => {
    const label = `${pointerPath} packageUnits[${index}]`;
    const object = readOptionalRecord(item);
    if (!object) {
      throw new Error(`${label} must be an object`);
    }
    const allowed = new Set([
      "schemaVersion",
      "packageId",
      "version",
      "buildIdentity",
      "abiIdentity",
      "unitHash",
      "unitPath",
    ]);
    const unsupported = Object.keys(object).filter((key) => !allowed.has(key));
    if (unsupported.length > 0) {
      throw new Error(`${label} does not support ${unsupported.join(", ")}`);
    }
    const schemaVersion = readRequiredString(
      object.schemaVersion,
      `${label}.schemaVersion`,
    );
    if (schemaVersion !== "skiff-package-unit-v1") {
      throw new Error(
        `${label}.schemaVersion must be skiff-package-unit-v1`,
      );
    }
    return {
      schemaVersion,
      packageId: readRequiredString(object.packageId, `${label}.packageId`),
      version: readRequiredString(object.version, `${label}.version`),
      buildIdentity: readRequiredString(
        object.buildIdentity,
        `${label}.buildIdentity`,
      ),
      abiIdentity: readRequiredString(
        object.abiIdentity,
        `${label}.abiIdentity`,
      ),
      unitHash: readRequiredString(object.unitHash, `${label}.unitHash`),
      unitPath: readRequiredString(object.unitPath, `${label}.unitPath`),
    };
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
): { path: string; assemblyIdentity: string } {
  if (value === undefined) {
    throw new Error(`${indexPath} serviceAssembly is required`);
  }
  const object = readOptionalRecord(value);
  if (!object) {
    throw new Error(`${indexPath} serviceAssembly must be an object`);
  }
  const allowed = new Set(["assemblyIdentity", "assemblyPath"]);
  const unsupported = Object.keys(object).filter((key) => !allowed.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `${indexPath} serviceAssembly does not support ${unsupported.join(", ")}`,
    );
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
    assemblyIdentity,
  };
}

function readServiceUnitPointer(
  value: Record<string, unknown>,
  indexPath: string,
): ServiceUnitArtifactPointer {
  if ("serviceUnitPath" in value) {
    throw new Error(
      `${indexPath} serviceUnitPath is not supported; use serviceUnit.unitPath`,
    );
  }
  const object = readOptionalRecord(value.serviceUnit);
  if (!object) {
    throw new Error(`${indexPath} serviceUnit must be an object`);
  }
  const allowed = new Set([
    "schemaVersion",
    "unitIdentity",
    "unitHash",
    "unitPath",
  ]);
  const unsupported = Object.keys(object).filter((key) => !allowed.has(key));
  if (unsupported.length > 0) {
    throw new Error(
      `${indexPath} serviceUnit does not support ${unsupported.join(", ")}`,
    );
  }
  const schemaVersion = readRequiredString(
    object.schemaVersion,
    `${indexPath} serviceUnit.schemaVersion`,
  );
  if (schemaVersion !== "skiff-service-unit-v1") {
    throw new Error(
      `${indexPath} serviceUnit.schemaVersion must be skiff-service-unit-v1`,
    );
  }
  return {
    schemaVersion,
    unitIdentity: readRequiredString(
      object.unitIdentity,
      `${indexPath} serviceUnit.unitIdentity`,
    ),
    unitHash: readRequiredString(
      object.unitHash,
      `${indexPath} serviceUnit.unitHash`,
    ),
    unitPath: readRequiredString(
      object.unitPath,
      `${indexPath} serviceUnit.unitPath`,
    ),
  };
}

function definedPointer(pointer: ArtifactPointerInput): ArtifactPointer {
  const result: ArtifactPointer = {
    buildId: pointer.buildId,
    indexPath: pointer.indexPath,
    serviceAssembly: pointer.serviceAssembly,
    serviceAssemblyIdentity: pointer.serviceAssemblyIdentity,
    serviceUnit: pointer.serviceUnit,
    serviceId: pointer.serviceId,
    packageUnits: pointer.packageUnits,
  };
  for (const key of [
    "contractIdentity",
    "fingerprint",
    "generation",
    "serviceVersion",
  ] as const) {
    const value = pointer[key];
    if (value !== undefined) {
      result[key] = value;
    }
  }
  return result;
}
