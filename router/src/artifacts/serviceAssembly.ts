import type { SkiffRuntimeManifest } from "../manifest/types.js";
import { assertRevisionId } from "../manifest/revisionId.js";
import {
  isPublicationId,
  publicationStorageSegment,
} from "../publicationId.js";
import { readConfigShape, type JsonObject } from "../config/index.js";
import { readJsonAtArtifactPath } from "./artifactPath.js";
import {
  validateServiceAssemblyContentIdentity,
  validateServiceAssemblyIdentity,
  validateServiceAssemblyPathIdentity,
} from "./identity.js";
import {
  accessFromServiceAssembly,
  gatewayFromServiceAssembly,
  operationsFromServiceUnitRoutes,
  timeoutFromServiceAssembly,
} from "./manifestProjection.js";
import {
  computeRuntimeProgramBuildId,
  readRuntimeProgramServiceUnit,
} from "./dynamicBuildId.js";
import {
  assertRecord,
  readOptionalRecord,
  readOptionalString,
  readRequiredString,
} from "./readUtils.js";
import type {
  LoadedServiceAssemblyArtifact,
  LoadRouterArtifactRootOptions,
  SourcedArtifactPointer,
} from "./types.js";
import {
  buildServiceConfigActivation,
  readConfigActivation,
  type PackageConfigActivationInput,
} from "./configActivation.js";

export async function readRouterArtifactValue(
  pointer: SourcedArtifactPointer,
  options: LoadRouterArtifactRootOptions,
): Promise<LoadedServiceAssemblyArtifact> {
  if (pointer.serviceAssembly) {
    const root = pointer.sourceRoot;
    const assembly = await readJsonAtArtifactPath(
      root,
      pointer.serviceAssembly,
      pointer.indexPath,
    );
    return routerManifestFromServiceAssembly(root, assembly, pointer, options);
  }
  throw new Error(`${pointer.indexPath} serviceAssembly is required`);
}

async function routerManifestFromServiceAssembly(
  root: string,
  assembly: unknown,
  pointer: SourcedArtifactPointer,
  options: LoadRouterArtifactRootOptions,
): Promise<LoadedServiceAssemblyArtifact> {
  assertRecord(assembly, `${pointer.indexPath} serviceAssembly`);
  if (assembly.schemaVersion !== "skiff-assembly-v1") {
    throw new Error(
      `${pointer.indexPath} serviceAssembly.schemaVersion must be skiff-assembly-v1`,
    );
  }
  if (assembly.kind !== "service") {
    throw new Error(
      `${pointer.indexPath} serviceAssembly.kind must be service`,
    );
  }
  if ("http" in assembly || "websocket" in assembly) {
    throw new Error(
      `${pointer.indexPath} serviceAssembly top-level http/websocket is not supported; use gateway.http/gateway.websocket`,
    );
  }
  rejectLegacyServiceAssemblyConfigFields(assembly, pointer.indexPath);
  const configShape = readConfigShape(
    assembly.configShape,
    `${pointer.indexPath} serviceAssembly.configShape`,
  );
  const configActivation = readConfigActivation(
    assembly.configActivation,
    `${pointer.indexPath} serviceAssembly.configActivation`,
  );

  const service = readOptionalRecord(assembly.service);
  if (!service) {
    throw new Error(
      `${pointer.indexPath} serviceAssembly.service must be an object`,
    );
  }
  const effectiveAssemblyIdentity = validateServiceAssemblyIdentity(
    pointer,
    readOptionalString(service.assemblyIdentity),
  );
  validateServiceAssemblyContentIdentity(
    assembly,
    effectiveAssemblyIdentity,
    pointer.indexPath,
  );
  const serviceId = readRequiredString(
    service.id,
    `${pointer.indexPath} serviceAssembly.service.id`,
  );
  validateServiceAssemblyPathIdentity(
    pointer.serviceAssembly!,
    serviceId,
    effectiveAssemblyIdentity,
    pointer.indexPath,
  );
  if (pointer.serviceId === undefined) {
    throw new Error(
      `${pointer.indexPath} serviceAssembly pointer must declare serviceId`,
    );
  }
  if (pointer.serviceId !== serviceId) {
    throw new Error(
      `${pointer.indexPath} serviceId must match serviceAssembly.service.id`,
    );
  }
  const revisionId = readRequiredString(
    service.revisionId,
    `${pointer.indexPath} serviceAssembly.service.revisionId`,
  );
  assertRevisionId(
    revisionId,
    `${pointer.indexPath} serviceAssembly.service.revisionId`,
  );
  const protocolIdentity = readRequiredString(
    service.protocolIdentity,
    `${pointer.indexPath} serviceAssembly.service.protocolIdentity`,
  );
  if (pointer.buildId === undefined) {
    throw new Error(
      `${pointer.indexPath} serviceAssembly pointer must declare buildId`,
    );
  }
  const pointerBuildId = readRequiredString(
    pointer.buildId,
    `${pointer.indexPath} buildId`,
  );
  const serviceUnit = await readRuntimeProgramServiceUnit({
    root,
    pointer,
    serviceAssembly: assembly,
  });
  const serviceVersion = readRequiredString(
    serviceUnit.value.version,
    `${serviceUnit.path} service unit.version`,
  );
  const dynamicBuildId = await computeRuntimeProgramBuildId({
    root,
    pointer,
    serviceAssembly: assembly,
    serviceUnit,
    ...(options.identityCliPath !== undefined
      ? { identityCliPath: options.identityCliPath }
      : {}),
    ...(options.releaseMode !== undefined
      ? { releaseMode: options.releaseMode }
      : {}),
  });
  if (
    pointer.contractIdentity !== undefined &&
    pointer.contractIdentity !== protocolIdentity
  ) {
    throw new Error(
      `${pointer.indexPath} contractIdentity must match serviceAssembly.service.protocolIdentity`,
    );
  }

  const operations = operationsFromServiceUnitRoutes(
    assembly,
    serviceUnit.value,
    pointer.indexPath,
    serviceUnit.path,
    protocolIdentity,
  );
  const gateway = gatewayFromServiceAssembly(
    assembly,
    pointer.indexPath,
    operations,
  );
  const access = accessFromServiceAssembly(service);
  const manifest: SkiffRuntimeManifest = {
    schemaVersion: "skiff-runtime-manifest-v1",
    service: {
      id: serviceId,
      revisionId,
      protocolIdentity,
      ...(access !== undefined ? { access } : {}),
    },
    operations,
  };
  if (Object.keys(gateway).length > 0) {
    manifest.gateway = gateway;
  }
  const timeout = timeoutFromServiceAssembly(assembly);
  if (timeout !== undefined) {
    manifest.timeout = timeout;
  }
  const activation = await buildServiceConfigActivation({
    root,
    indexPath: pointer.indexPath,
    serviceId,
    buildId: dynamicBuildId,
    configShape,
    configActivation,
    packageConfigs: await packageConfigActivationInputs(
      root,
      serviceUnit.value,
      pointer.indexPath,
      serviceUnit.path,
    ),
    ...(options.configProfile !== undefined
      ? { configProfile: options.configProfile }
      : {}),
    ...(options.serviceDb !== undefined
      ? { serviceDb: options.serviceDb }
      : {}),
  });
  return {
    buildId: dynamicBuildId,
    pointerBuildId,
    serviceVersion,
    sourcePath: pointer.indexPath,
    manifestValue: manifest,
    ...(activation
      ? {
          activation: {
            operationTargets: operations.map((operation) => operation.target),
            serviceId,
            payload: activation,
          },
        }
      : {}),
  };
}

async function packageConfigActivationInputs(
  root: string,
  serviceUnit: Record<string, unknown>,
  indexPath: string,
  serviceUnitPath: string,
): Promise<PackageConfigActivationInput[]> {
  const packageDependencies = serviceUnitPackageDependencies(
    serviceUnit.packageDependencies ?? serviceUnit.package_dependencies,
    `${serviceUnitPath} serviceUnit.packageDependencies`,
  );
  const inputs: PackageConfigActivationInput[] = [];
  for (const dependency of packageDependencies) {
    const packageUnit = await readPackageUnitForDependency(root, dependency);
    const configMetadata = readPackageConfigMetadata(packageUnit.value);
    const configShape = readConfigShape(
      configMetadata.shape,
      `${packageUnit.path} packageUnit.configAndEffectMetadata.config.shape`,
    );
    const configActivation = readConfigActivation(
      configMetadata.activation,
      `${packageUnit.path} packageUnit.configAndEffectMetadata.config.activation`,
    );
    inputs.push({
      packageId: dependency.id,
      alias: dependency.alias,
      defaultConfig: {},
      configShape,
      configActivation,
    });
  }
  return inputs;
}

interface ServiceUnitPackageDependency {
  id: string;
  version: string;
  alias: string;
}

function serviceUnitPackageDependencies(
  value: unknown,
  label: string,
): ServiceUnitPackageDependency[] {
  if (value === undefined || value === null) {
    return [];
  }
  if (!Array.isArray(value)) {
    throw new Error(`${label} must be an array`);
  }
  return value.map((item, index) => {
    const object = readOptionalRecord(item);
    if (!object) {
      throw new Error(`${label}[${index}] must be an object`);
    }
    rejectLegacyPackageDependencyFields(object, `${label}[${index}]`);
    return {
      id: readRequiredString(object.id, `${label}[${index}].id`),
      version: readRequiredString(object.version, `${label}[${index}].version`),
      alias: readRequiredString(object.alias, `${label}[${index}].alias`),
    };
  });
}

function rejectLegacyPackageDependencyFields(
  object: Record<string, unknown>,
  label: string,
): void {
  const legacyFields = [
    "packageId",
    "package_id",
    "versionConstraint",
    "version_constraint",
    "dependencyRef",
    "dependency_ref",
    "aliases",
  ];
  for (const field of legacyFields) {
    if (Object.prototype.hasOwnProperty.call(object, field)) {
      throw new Error(
        `${label}.${field} is no longer supported; use id/version/alias`,
      );
    }
  }
}

async function readPackageUnitForDependency(
  root: string,
  dependency: ServiceUnitPackageDependency,
): Promise<{ path: string; value: Record<string, unknown> }> {
  const indexPath = packageVersionIndexPath(dependency.id, dependency.version);
  const index = await readJsonAtArtifactPath(root, indexPath, indexPath).catch(
    (error: unknown) => {
      throw new Error(
        `no package unit found for ${dependency.id} version ${dependency.version}`,
        {
          cause: error,
        },
      );
    },
  );
  assertRecord(index, `${indexPath} package unit index`);
  validatePackageIndexIdentity(index, dependency, indexPath);
  const unitPath = packageUnitPathFromIndex(index, indexPath);
  const unit =
    unitPath === indexPath
      ? index
      : await readJsonAtArtifactPath(root, unitPath, indexPath);
  assertRecord(unit, `${unitPath} package unit`);
  if ((unit.schemaVersion ?? unit.schema_version) !== "skiff-package-unit-v1") {
    throw new Error(
      `${unitPath} package unit schemaVersion must be skiff-package-unit-v1`,
    );
  }
  const packageId = readOptionalString(
    unit.packageId ?? unit.package_id ?? unit.id,
  );
  if (packageId !== undefined && packageId !== dependency.id) {
    throw new Error(
      `${unitPath} packageId ${packageId} does not match dependency ${dependency.id}`,
    );
  }
  return { path: unitPath, value: unit };
}

function packageUnitPathFromIndex(
  index: Record<string, unknown>,
  indexPath: string,
): string {
  const packageUnit = index.packageUnit;
  const stringPath = readOptionalString(packageUnit);
  if (stringPath !== undefined) {
    return stringPath;
  }
  const object = readOptionalRecord(packageUnit);
  if (object) {
    const path =
      readOptionalString(object.unitPath) ??
      readOptionalString(object.artifactPath) ??
      readOptionalString(object.path) ??
      readOptionalString(object.packageUnitPath);
    if (path !== undefined) {
      return path;
    }
    throw new Error(
      `${indexPath} packageUnit requires unitPath/artifactPath/path`,
    );
  }
  const packageUnitPath = readOptionalString(index.packageUnitPath);
  if (packageUnitPath !== undefined) {
    return packageUnitPath;
  }
  if (
    (index.schemaVersion ?? index.schema_version) === "skiff-package-unit-v1"
  ) {
    return indexPath;
  }
  throw new Error(
    `${indexPath} package index must declare packageUnit/packageUnitPath or be a PackageUnit`,
  );
}

function validatePackageIndexIdentity(
  index: Record<string, unknown>,
  dependency: ServiceUnitPackageDependency,
  indexPath: string,
): void {
  const packageRecord = readOptionalRecord(index.package);
  const packageId =
    readOptionalString(index.packageId) ??
    readOptionalString(index.id) ??
    readOptionalString(packageRecord?.packageId) ??
    readOptionalString(packageRecord?.id);
  if (packageId !== undefined && packageId !== dependency.id) {
    throw new Error(
      `${indexPath} package id ${packageId} does not match dependency id ${dependency.id}`,
    );
  }
  const version =
    readOptionalString(index.version) ??
    readOptionalString(packageRecord?.version);
  if (version !== undefined && version !== dependency.version) {
    throw new Error(
      `${indexPath} package version ${version} does not match dependency version ${dependency.version}`,
    );
  }
}

function packageVersionIndexPath(packageId: string, version: string): string {
  if (!isPublicationId(packageId)) {
    throw new Error(`package id ${packageId} must be a publication id`);
  }
  if (
    version.length === 0 ||
    version === "." ||
    version === ".." ||
    /[\\/]/.test(version)
  ) {
    throw new Error(
      `package version ${version} is not a safe artifact path segment`,
    );
  }
  return `indexes/packages/${publicationStorageSegment(packageId)}/versions/${version}.json`;
}

function readPackageConfigMetadata(packageUnit: Record<string, unknown>): {
  shape: unknown;
  activation: unknown;
} {
  const metadata = readOptionalRecord(
    packageUnit.configAndEffectMetadata ??
      packageUnit.config_and_effect_metadata,
  );
  const config = readOptionalRecord(metadata?.config);
  return {
    shape: config?.shape,
    activation: config?.activation,
  };
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function cloneJsonObject(value: JsonObject): JsonObject {
  return JSON.parse(JSON.stringify(value)) as JsonObject;
}

function rejectLegacyServiceAssemblyConfigFields(
  assembly: Record<string, unknown>,
  indexPath: string,
): void {
  if (Object.prototype.hasOwnProperty.call(assembly, "envShape")) {
    throw new Error(
      `${indexPath} serviceAssembly.envShape is no longer supported; use configShape`,
    );
  }
  if (Object.prototype.hasOwnProperty.call(assembly, "envActivation")) {
    throw new Error(
      `${indexPath} serviceAssembly.envActivation is no longer supported; use configActivation`,
    );
  }
  if (Object.prototype.hasOwnProperty.call(assembly, "envUses")) {
    throw new Error(
      `${indexPath} serviceAssembly.envUses is no longer supported; use configUses`,
    );
  }
  if (Object.prototype.hasOwnProperty.call(assembly, "valuesPolicy")) {
    throw new Error(
      `${indexPath} serviceAssembly.valuesPolicy is no longer supported; use configShape`,
    );
  }
  if (Object.prototype.hasOwnProperty.call(assembly, "valuesReads")) {
    throw new Error(
      `${indexPath} serviceAssembly.valuesReads is no longer supported; use configUses`,
    );
  }
}
