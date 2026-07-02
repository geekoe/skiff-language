import type {
  LoadedManifest,
  SkiffRuntimeManifest,
} from "../manifest/types.js";
import type {
  RuntimeConfigActivationPayload,
  FileBackendControlConfig,
  RuntimeServiceDbActivationPayload,
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

export interface LoadRouterArtifactRootOptions {
  devReload?: boolean;
  identityCliPath?: string;
  releaseMode?: boolean;
  telemetry?: TelemetryControlConfig;
  fileBackend?: FileBackendControlConfig;
  configProfile?: string;
  serviceDb?: RuntimeServiceDbActivationPayload;
}

export interface LoadedServiceAssemblyArtifact {
  buildId: string;
  manifestValue: SkiffRuntimeManifest;
  pointerBuildId?: string;
  serviceVersion: string;
  sourcePath: string;
  activation?: {
    operationTargets: string[];
    serviceId: string;
    payload: RuntimeConfigActivationPayload;
  };
}

export interface ArtifactPointer {
  buildId?: string;
  contractIdentity?: string;
  fingerprint?: string;
  generation?: string;
  indexPath: string;
  serviceVersion?: string;
  serviceAssembly?: string;
  serviceAssemblyIdentity?: string;
  serviceUnit?: string;
  serviceId?: string;
}

export interface SourcedArtifactPointer extends ArtifactPointer {
  sourceRoot: string;
}

export type ArtifactPointerInput = {
  [Key in keyof ArtifactPointer]: ArtifactPointer[Key] | undefined;
} & {
  indexPath: string;
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
