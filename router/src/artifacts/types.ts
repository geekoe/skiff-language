import type {
  LoadedManifest,
  SkiffRuntimeManifest,
} from "../manifest/types.js";
import type {
  RuntimeConfigActivationPayload,
  FileBackendControlConfig,
  RuntimeServiceDbConfigInput,
  TelemetryControlConfig,
} from "../protocol/envelope.js";
import type { ActivationLookup } from "./activationLookup.js";

export interface RuntimeControlMetadata {
  artifactRoots: readonly string[];
  devReload?: boolean;
  mode?: "dev" | "release";
  generation?: string;
  fingerprint?: string;
  serviceBuilds?: readonly RuntimeControlServiceBuild[];
  serviceConfig?: RuntimeConfigActivationPayload[];
  telemetry?: TelemetryControlConfig;
  fileBackend?: FileBackendControlConfig;
}

export interface RuntimeControlServiceBuild {
  buildId: string;
  pointerBuildId?: string;
  serviceId: string;
  sourcePath: string;
  version?: string;
}

export interface LoadedRouterArtifacts {
  manifest: LoadedManifest;
  control: RuntimeControlMetadata;
  activationByServiceOperation: ActivationLookup;
  versionByService?: ReadonlyMap<
    string,
    ReadonlyMap<string, ServiceVersionBuildBinding>
  >;
}

export interface ServiceVersionBuildBinding {
  buildId: string;
  pointerBuildId?: string;
  serviceId: string;
  version: string;
}

export interface LoadedServiceConfigActivation {
  operationTargets: string[];
  serviceId: string;
  payload: RuntimeConfigActivationPayload;
}

export interface LoadRouterArtifactRootOptions {
  devReload?: boolean;
  identityCliPath?: string;
  releaseMode?: boolean;
  telemetry?: TelemetryControlConfig;
  fileBackend?: FileBackendControlConfig;
  configProfile?: string;
  serviceDb?: RuntimeServiceDbConfigInput;
}

export interface LoadedServiceAssemblyArtifact {
  buildId: string;
  manifestValue: SkiffRuntimeManifest;
  pointerBuildId?: string;
  serviceVersion: string;
  sourcePath: string;
  activation?: LoadedServiceConfigActivation;
  activations?: LoadedServiceConfigActivation[];
}

export interface ArtifactPointer {
  buildId: string;
  contractIdentity?: string;
  fingerprint?: string;
  generation?: string;
  indexPath: string;
  serviceVersion?: string;
  serviceAssembly: string;
  serviceAssemblyIdentity: string;
  serviceUnit: ServiceUnitArtifactPointer;
  serviceId: string;
  packageUnits: readonly PackageUnitArtifactPointer[];
}

export interface SourcedArtifactPointer extends ArtifactPointer {
  sourceRoot: string;
}

export interface PackageUnitArtifactPointer {
  schemaVersion: "skiff-package-unit-v2";
  packageId: string;
  version: string;
  buildIdentity: string;
  abiIdentity: string;
  unitHash: string;
  unitPath: string;
}

export interface ServiceUnitArtifactPointer {
  schemaVersion: "skiff-service-unit-v1";
  unitIdentity: string;
  unitHash: string;
  unitPath: string;
}

export interface ValidatedArtifactContent {
  path: string;
  value: Record<string, unknown>;
}

export interface ValidatedServiceArtifactClosure {
  key: string;
  dynamicBuildId: string;
  assemblyIdentity: string;
  serviceAssembly: ValidatedArtifactContent;
  serviceUnit: ValidatedArtifactContent;
  packageUnits: readonly ValidatedArtifactContent[];
}

export type ArtifactPointerInput = {
  buildId: string;
  indexPath: string;
  serviceAssembly: string;
  serviceAssemblyIdentity: string;
  serviceUnit: ServiceUnitArtifactPointer;
  serviceId: string;
  packageUnits: readonly PackageUnitArtifactPointer[];
  contractIdentity?: string;
  fingerprint?: string;
  generation?: string;
  serviceVersion?: string;
};

export interface ActiveArtifactPointers {
  fingerprintSources: Array<{
    path: string;
    sourceRoot: string;
    text: string;
  }>;
  mode: "dev" | "release";
  pointers: SourcedArtifactPointer[];
  serviceVersionBindings?: ServiceVersionBuildBinding[];
}
