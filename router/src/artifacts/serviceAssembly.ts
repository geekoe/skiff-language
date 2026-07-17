import type { SkiffRuntimeManifest } from "../manifest/types.js";
import { assertRevisionId } from "../manifest/revisionId.js";
import { readConfigShape } from "../config/index.js";
import {
  accessFromServiceAssembly,
  gatewayFromServiceAssembly,
  operationsFromServiceUnitRoutes,
  timeoutFromServiceAssembly,
} from "./manifestProjection.js";
import {
  assertRecord,
  readOptionalRecord,
  readRequiredString,
} from "./readUtils.js";
import type {
  LoadedServiceAssemblyArtifact,
  LoadRouterArtifactRootOptions,
  SourcedArtifactPointer,
  ValidatedArtifactContent,
  ValidatedServiceArtifactClosure,
} from "./types.js";
import {
  buildServiceConfigActivation,
  readConfigActivation,
  type PackageConfigActivationInput,
} from "./configActivation.js";
import { readServiceTestConfigActivations } from "./serviceTestActivations.js";
import { validateServiceHttp } from "./serviceHttp.js";

export async function readRouterArtifactValue(
  pointer: SourcedArtifactPointer,
  options: LoadRouterArtifactRootOptions,
  validated: ValidatedServiceArtifactClosure,
): Promise<LoadedServiceAssemblyArtifact> {
  return routerManifestFromServiceAssembly(
    pointer.sourceRoot,
    validated.serviceAssembly.value,
    pointer,
    options,
    validated,
  );
}

async function routerManifestFromServiceAssembly(
  root: string,
  assembly: unknown,
  pointer: SourcedArtifactPointer,
  options: LoadRouterArtifactRootOptions,
  validated: ValidatedServiceArtifactClosure,
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
  validateServiceHttp(
    service,
    `${pointer.indexPath} serviceAssembly.service`,
  );
  const embeddedAssemblyIdentity = readRequiredString(
    service.assemblyIdentity,
    `${pointer.indexPath} serviceAssembly.service.assemblyIdentity`,
  );
  if (
    embeddedAssemblyIdentity !== validated.assemblyIdentity ||
    pointer.serviceAssemblyIdentity !== validated.assemblyIdentity
  ) {
    throw new Error(`${pointer.indexPath} validated assembly identity mismatch`);
  }
  const serviceId = readRequiredString(
    service.id,
    `${pointer.indexPath} serviceAssembly.service.id`,
  );
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
  const pointerBuildId = readRequiredString(
    pointer.buildId,
    `${pointer.indexPath} buildId`,
  );
  const serviceUnit = validated.serviceUnit;
  const serviceVersion = readRequiredString(
    serviceUnit.value.version,
    `${serviceUnit.path} service unit.version`,
  );
  const dynamicBuildId = validated.dynamicBuildId;
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
  const packageConfigs = packageConfigActivationInputs(
    serviceUnit.value,
    validated.packageUnits,
    pointer.indexPath,
    serviceUnit.path,
  );
  const serviceTestActivations = await readServiceTestConfigActivations({
    root,
    indexPath: pointer.indexPath,
    serviceId,
    buildId: dynamicBuildId,
    pointerBuildId,
    operationTargets: operations.map((operation) => operation.target),
    configShape,
    configActivation,
    packageConfigs,
  });
  const activation =
    serviceTestActivations.length > 0
      ? undefined
      : await buildServiceConfigActivation({
          root,
          indexPath: pointer.indexPath,
          serviceId,
          buildId: dynamicBuildId,
          configShape,
          configActivation,
          packageConfigs,
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
    ...(serviceTestActivations.length > 0
      ? { activations: serviceTestActivations }
      : {}),
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

function packageConfigActivationInputs(
  serviceUnit: Record<string, unknown>,
  packageUnits: readonly ValidatedArtifactContent[],
  indexPath: string,
  serviceUnitPath: string,
): PackageConfigActivationInput[] {
  const packageDependencies = serviceUnitPackageDependencies(
    serviceUnit.packageDependencies,
    `${serviceUnitPath} serviceUnit.packageDependencies`,
  );
  const inputs: PackageConfigActivationInput[] = [];
  for (const dependency of packageDependencies) {
    const matches = packageUnits.filter((unit) =>
      unit.value.packageId === dependency.id &&
      unit.value.version === dependency.version
    );
    if (matches.length !== 1) {
      throw new Error(
        `${indexPath} validated package closure must contain exactly one ${dependency.id}@${dependency.version}`,
      );
    }
    const packageUnit = matches[0]!;
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

function readPackageConfigMetadata(packageUnit: Record<string, unknown>): {
  shape: unknown;
  activation: unknown;
} {
  const metadata = readOptionalRecord(
    packageUnit.configAndEffectMetadata,
  );
  const config = readOptionalRecord(metadata?.config);
  return {
    shape: config?.shape,
    activation: config?.activation,
  };
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
