import { createHash } from "node:crypto";
import { stat } from "node:fs/promises";
import { relative, resolve } from "node:path";

import {
  loadManifest,
  mergeLoadedManifests,
} from "../manifest/loadManifest.js";
import {
  buildActivationLookup,
  serviceConfigActivations,
  validateServingManifestUniqueness,
} from "./activationLookup.js";
import { readActiveArtifactPointers } from "./pointers.js";
import { readRouterArtifactValue } from "./serviceAssembly.js";
import type {
  ActiveArtifactPointers,
  LoadedServiceAssemblyArtifact,
  LoadedRouterArtifacts,
  LoadRouterArtifactRootOptions,
  ServiceVersionBuildBinding,
  RuntimeControlMetadata,
  SourcedArtifactPointer,
} from "./types.js";

export { activationLookupKey } from "./activationLookup.js";
export type { ActivationLookup } from "./activationLookup.js";
export type {
  LoadedRouterArtifacts,
  LoadRouterArtifactRootOptions,
  ServiceVersionBuildBinding,
  RuntimeControlMetadata,
} from "./types.js";

export type RouterArtifactRootInput = string | readonly string[];

export async function loadRouterArtifactRoot(
  artifactRoots: RouterArtifactRootInput,
  options: LoadRouterArtifactRootOptions = {},
): Promise<LoadedRouterArtifacts> {
  const roots = normalizeRouterArtifactRoots(artifactRoots);
  for (const root of roots) {
    const rootStat = await stat(root).catch((error: unknown) => {
      throw new Error(`artifact root ${root} is not readable`, {
        cause: error,
      });
    });
    if (!rootStat.isDirectory()) {
      throw new Error(`artifact root ${root} must be a directory`);
    }
  }

  const activePointers = mergeActiveArtifactPointers(
    await Promise.all(
      roots.map((root) =>
        readActiveArtifactPointers(root, options, {
          allowEmpty: roots.length > 1,
        }),
      ),
    ),
  );
  const pointers = activePointers.pointers;
  const fingerprintHash = createHash("sha256");
  for (const source of activePointers.fingerprintSources) {
    fingerprintHash.update(source.sourceRoot);
    fingerprintHash.update("\0");
    fingerprintHash.update(relative(source.sourceRoot, source.path));
    fingerprintHash.update("\0");
    fingerprintHash.update(source.text);
    fingerprintHash.update("\0");
  }
  if (pointers.length === 0) {
    throw new Error(
      `artifact roots ${roots.join(", ")} active artifact set does not declare any services`,
    );
  }

  const artifacts = await Promise.all(
    pointers.map(async (pointer) => readRouterArtifactValue(pointer, options)),
  );
  const versionByService = buildVersionLookup(
    resolveVersionBindings(activePointers, artifacts),
  );
  validateServingManifestUniqueness(artifacts);
  const manifests = artifacts.map((artifact) => {
    const manifest = loadManifest(artifact.manifestValue);
    const buildId = artifact.buildId;
    const loadedManifest = {
      ...manifest,
      httpRouteEntries: manifest.httpRouteEntries.map((entry) => ({
        ...entry,
        buildId,
      })),
      rawHttpEntries: manifest.rawHttpEntries.map((entry) => ({
        ...entry,
        buildId,
      })),
      websocketEntries: manifest.websocketEntries.map((entry) => ({
        ...entry,
        buildId,
      })),
    };
    if (manifest.websocketEntry !== undefined) {
      return {
        ...loadedManifest,
        websocketEntry: {
          ...manifest.websocketEntry,
          buildId,
        },
      };
    }
    return loadedManifest;
  });
  const manifest =
    manifests.length === 1 ? manifests[0]! : mergeLoadedManifests(manifests);
  const serviceConfig = artifacts.flatMap((artifact) =>
    serviceConfigActivations(artifact).map((activation) => activation.payload),
  );
  const activationByServiceOperation = buildActivationLookup(artifacts);
  const firstGeneration = firstCommonValue(
    pointers.map((pointer) => pointer.generation),
  );
  const firstFingerprint =
    firstCommonValue(
      pointers.map(
        (pointer) => pointer.fingerprint ?? pointer.serviceAssemblyIdentity,
      ),
    ) ?? `sha256:${fingerprintHash.digest("hex")}`;

  const control: RuntimeControlMetadata = {
    artifactRoots: roots,
    mode: activePointers.mode,
    serviceBuilds: artifacts.map((artifact) => ({
      serviceId: artifact.manifestValue.service.id,
      version: artifact.serviceVersion,
      buildId: artifact.buildId,
      ...(artifact.pointerBuildId !== undefined
        ? { pointerBuildId: artifact.pointerBuildId }
        : {}),
      sourcePath: artifact.sourcePath,
    })),
  };
  if (options.devReload !== undefined) {
    control.devReload = options.devReload;
  }
  if (firstGeneration !== undefined) {
    control.generation = firstGeneration;
  }
  if (firstFingerprint !== undefined) {
    control.fingerprint = firstFingerprint;
  }
  if (serviceConfig.length > 0) {
    control.serviceConfig = serviceConfig;
  }
  if (options.telemetry !== undefined) {
    control.telemetry = options.telemetry;
  }
  if (options.fileBackend !== undefined) {
    control.fileBackend = options.fileBackend;
  }
  return {
    manifest,
    control,
    activationByServiceOperation,
    ...(versionByService !== undefined ? { versionByService } : {}),
  };
}

export function normalizeRouterArtifactRoots(
  input: RouterArtifactRootInput,
): string[] {
  const rawRoots = typeof input === "string" ? [input] : [...input];
  if (rawRoots.length === 0) {
    throw new Error("artifact roots must contain at least one path");
  }
  const roots = rawRoots.map((root, index) => {
    if (typeof root !== "string" || root.trim().length === 0) {
      throw new Error(`artifact roots[${index}] must be a non-empty string`);
    }
    return resolve(root.trim());
  });
  const seen = new Set<string>();
  for (const root of roots) {
    if (seen.has(root)) {
      throw new Error(`artifact roots must not contain duplicate path ${root}`);
    }
    seen.add(root);
  }
  return roots;
}

function mergeActiveArtifactPointers(
  sets: readonly ActiveArtifactPointers[],
): ActiveArtifactPointers {
  const fingerprintSources = sets.flatMap((set) => set.fingerprintSources);
  const mode = firstCommonMode(sets.map((set) => set.mode));
  if (mode === undefined) {
    throw new Error("artifact roots must use the same activation mode");
  }
  if (mode === "dev") {
    return {
      fingerprintSources,
      mode,
      pointers: mergeDevPointers(sets),
    };
  }
  const serviceVersionBindings = mergeServiceVersionBindings(sets);
  return {
    fingerprintSources,
    mode,
    pointers: mergeReleasePointers(sets, serviceVersionBindings),
    serviceVersionBindings,
  };
}

function mergeDevPointers(
  sets: readonly ActiveArtifactPointers[],
): SourcedArtifactPointer[] {
  const byService = new Map<string, SourcedArtifactPointer>();
  for (const set of sets) {
    for (const pointer of set.pointers) {
      if (pointer.serviceId === undefined) {
        throw new Error(`${pointer.indexPath} serviceId is required`);
      }
      byService.delete(pointer.serviceId);
      byService.set(pointer.serviceId, pointer);
    }
  }
  return Array.from(byService.values());
}

function mergeServiceVersionBindings(
  sets: readonly ActiveArtifactPointers[],
): ServiceVersionBuildBinding[] {
  const byServiceVersion = new Map<string, ServiceVersionBuildBinding>();
  for (const set of sets) {
    for (const binding of set.serviceVersionBindings ?? []) {
      const key = `${binding.serviceId}\0${binding.version}`;
      byServiceVersion.delete(key);
      byServiceVersion.set(key, binding);
    }
  }
  return Array.from(byServiceVersion.values());
}

function mergeReleasePointers(
  sets: readonly ActiveArtifactPointers[],
  serviceVersionBindings: readonly ServiceVersionBuildBinding[],
): SourcedArtifactPointer[] {
  const activeBuildKeys = new Set(
    serviceVersionBindings.map(
      (binding) => `${binding.serviceId}\0${binding.buildId}`,
    ),
  );
  const byServiceBuild = new Map<string, SourcedArtifactPointer>();
  for (const set of sets) {
    for (const pointer of set.pointers) {
      if (pointer.serviceId === undefined || pointer.buildId === undefined) {
        continue;
      }
      const key = `${pointer.serviceId}\0${pointer.buildId}`;
      if (!activeBuildKeys.has(key)) {
        continue;
      }
      byServiceBuild.delete(key);
      byServiceBuild.set(key, pointer);
    }
  }
  return Array.from(byServiceBuild.values());
}

function firstCommonValue(
  values: Array<string | undefined>,
): string | undefined {
  const defined = values.filter(
    (value): value is string => value !== undefined,
  );
  if (defined.length === 0) {
    return undefined;
  }
  const [first] = defined;
  return defined.every((value) => value === first) ? first : undefined;
}

function firstCommonMode(
  values: Array<"dev" | "release">,
): "dev" | "release" | undefined {
  const [first] = values;
  if (first === undefined) {
    return undefined;
  }
  return values.every((value) => value === first) ? first : undefined;
}

function buildVersionLookup(
  versions: readonly ServiceVersionBuildBinding[] | undefined,
):
  | ReadonlyMap<string, ReadonlyMap<string, ServiceVersionBuildBinding>>
  | undefined {
  if (!versions || versions.length === 0) {
    return undefined;
  }
  const byService = new Map<string, Map<string, ServiceVersionBuildBinding>>();
  for (const version of versions) {
    let serviceVersions = byService.get(version.serviceId);
    if (!serviceVersions) {
      serviceVersions = new Map<string, ServiceVersionBuildBinding>();
      byService.set(version.serviceId, serviceVersions);
    }
    const existing = serviceVersions.get(version.version);
    if (existing && existing.buildId !== version.buildId) {
      throw new Error(
        `duplicate service version pointer ${version.serviceId}:${version.version} resolves to multiple builds`,
      );
    }
    serviceVersions.set(version.version, version);
  }
  return byService;
}

function resolveDynamicVersionBindings(
  versions: readonly ServiceVersionBuildBinding[] | undefined,
  artifacts: readonly {
    buildId: string;
    pointerBuildId?: string;
    manifestValue: { service: { id: string } };
  }[],
): ServiceVersionBuildBinding[] | undefined {
  if (!versions || versions.length === 0) {
    return undefined;
  }
  const dynamicBuildIdByPointer = new Map<string, string>();
  for (const artifact of artifacts) {
    if (artifact.pointerBuildId === undefined) {
      continue;
    }
    dynamicBuildIdByPointer.set(
      `${artifact.manifestValue.service.id}\0${artifact.pointerBuildId}`,
      artifact.buildId,
    );
  }
  return versions.map((version) => {
    const dynamicBuildId = dynamicBuildIdByPointer.get(
      `${version.serviceId}\0${version.buildId}`,
    );
    if (dynamicBuildId === undefined) {
      throw new Error(
        `service version pointer ${version.serviceId}:${version.version} build ${version.buildId} did not resolve to a runtime program buildId`,
      );
    }
    return {
      ...version,
      buildId: dynamicBuildId,
      pointerBuildId: version.buildId,
    };
  });
}

function resolveVersionBindings(
  activePointers: ActiveArtifactPointers,
  artifacts: readonly LoadedServiceAssemblyArtifact[],
): ServiceVersionBuildBinding[] | undefined {
  if (activePointers.serviceVersionBindings !== undefined) {
    return resolveDynamicVersionBindings(
      activePointers.serviceVersionBindings,
      artifacts,
    );
  }
  if (activePointers.mode !== "dev") {
    return undefined;
  }
  return artifacts.map((artifact) => ({
    serviceId: artifact.manifestValue.service.id,
    version: artifact.serviceVersion,
    buildId: artifact.buildId,
    ...(artifact.pointerBuildId !== undefined
      ? { pointerBuildId: artifact.pointerBuildId }
      : {}),
  }));
}
