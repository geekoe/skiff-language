import { readFile, readdir, stat } from "node:fs/promises";
import { basename, extname, relative, resolve } from "node:path";

import {
  readBuildRecordPointer,
  readDevReloadPointer,
  readServiceVersionPointer,
} from "./pointerRecords.js";
import { serviceBuildIdHash } from "./identity.js";
import { serviceIdPathSegments } from "./pathProjection.js";
import type {
  ActiveArtifactPointers,
  ArtifactPointer,
  LoadRouterArtifactRootOptions,
  SourcedArtifactPointer,
} from "./types.js";

export async function readActiveArtifactPointers(
  root: string,
  options: LoadRouterArtifactRootOptions,
  readOptions: { allowEmpty?: boolean } = {},
): Promise<ActiveArtifactPointers> {
  if (options.devReload === true) {
    return readDevReloadPointers(root, readOptions);
  }
  return readServiceVersionPointers(root, readOptions);
}

async function readDevReloadPointers(
  root: string,
  options: { allowEmpty?: boolean } = {},
): Promise<ActiveArtifactPointers> {
  const pointerDir = resolve(root, "dev", "services");
  const pointerDirStat = await stat(pointerDir).catch(() => undefined);
  if (!pointerDirStat?.isDirectory()) {
    if (options.allowEmpty && pointerDirStat === undefined) {
      return { fingerprintSources: [], mode: "dev", pointers: [] };
    }
    throw new Error(
      `artifact dev reload dir ${pointerDir} must be a directory`,
    );
  }
  const pointerPaths = await readJsonFilesRecursive(pointerDir);
  if (pointerPaths.length === 0) {
    if (options.allowEmpty) {
      return { fingerprintSources: [], mode: "dev", pointers: [] };
    }
    throw new Error(
      `artifact dev reload dir ${pointerDir} must contain <storage-projected-service-id>.json`,
    );
  }

  const pointers: SourcedArtifactPointer[] = [];
  const fingerprintSources: ActiveArtifactPointers["fingerprintSources"] = [];
  const seenServices = new Set<string>();
  for (const pointerPath of pointerPaths) {
    const text = await readFile(pointerPath, "utf8");
    fingerprintSources.push({ path: pointerPath, sourceRoot: root, text });
    const pointer = readDevReloadPointer(JSON.parse(text), pointerPath);
    const pointerServiceId = pointer.serviceId;
    if (pointerServiceId === undefined) {
      throw new Error(`${pointerPath} serviceId is required`);
    }
    const pathServiceIdSegments = serviceIdJsonPathSegments(
      pointerDir,
      pointerPath,
    );
    const expectedServiceIdSegments = serviceIdPathSegments(pointerServiceId);
    if (!samePathSegments(pathServiceIdSegments, expectedServiceIdSegments)) {
      throw new Error(`${pointerPath} serviceId must match dev/services path`);
    }
    if (seenServices.has(pointerServiceId)) {
      throw new Error(
        `dev reload dir ${pointerDir} declares duplicate serviceId ${pointerServiceId}`,
      );
    }
    seenServices.add(pointerServiceId);
    pointers.push(withSourceRoot(root, pointer));
  }
  return { fingerprintSources, mode: "dev", pointers };
}

async function readServiceVersionPointers(
  root: string,
  options: { allowEmpty?: boolean } = {},
): Promise<ActiveArtifactPointers> {
  const versionRoot = resolve(root, "versions", "services");
  const versionRootStat = await stat(versionRoot).catch(() => undefined);
  if (!versionRootStat?.isDirectory()) {
    if (options.allowEmpty && versionRootStat === undefined) {
      return {
        fingerprintSources: [],
        mode: "release",
        pointers: [],
        serviceVersionBindings: [],
      };
    }
    throw new Error(`artifact versions dir ${versionRoot} must be a directory`);
  }

  const versionPaths = await readJsonFilesRecursive(versionRoot);
  const pointers: SourcedArtifactPointer[] = [];
  const fingerprintSources: ActiveArtifactPointers["fingerprintSources"] = [];
  const serviceVersionBindings: NonNullable<
    ActiveArtifactPointers["serviceVersionBindings"]
  > = [];
  const seenVersions = new Set<string>();
  const seenBuilds = new Set<string>();

  for (const versionPath of versionPaths) {
    const versionText = await readFile(versionPath, "utf8");
    fingerprintSources.push({
      path: versionPath,
      sourceRoot: root,
      text: versionText,
    });
    const serviceVersion = readServiceVersionPointer(
      JSON.parse(versionText),
      versionPath,
    );
    const versionPathParts = relativePathSegments(versionRoot, versionPath);
    const pathServiceIdSegments = versionPathParts.slice(0, -1);
    const expectedServiceIdSegments = serviceIdPathSegments(
      serviceVersion.serviceId,
    );
    if (!samePathSegments(pathServiceIdSegments, expectedServiceIdSegments)) {
      throw new Error(
        `${versionPath} serviceId must match versions/services path`,
      );
    }
    if (basename(versionPath) !== `${serviceVersion.version}.json`) {
      throw new Error(
        `${versionPath} must be named ${serviceVersion.version}.json for version`,
      );
    }
    const versionKey = `${serviceVersion.serviceId}\0${serviceVersion.version}`;
    if (seenVersions.has(versionKey)) {
      throw new Error(
        `service version pointers declare duplicate serviceId/version ${serviceVersion.serviceId}:${serviceVersion.version}`,
      );
    }
    seenVersions.add(versionKey);

    const buildPath = resolve(
      root,
      "builds",
      "services",
      ...serviceIdPathSegments(serviceVersion.serviceId),
      `${serviceBuildIdHash(serviceVersion.buildId, `${versionPath} buildId`)}.json`,
    );
    const buildText = await readFile(buildPath, "utf8").catch(
      (error: unknown) => {
        throw new Error(
          `${versionPath} points to unreadable build record ${buildPath}`,
          {
            cause: error,
          },
        );
      },
    );
    fingerprintSources.push({
      path: buildPath,
      sourceRoot: root,
      text: buildText,
    });
    const pointer = readBuildRecordPointer(
      JSON.parse(buildText),
      buildPath,
      serviceVersion,
    );
    const buildKey = `${serviceVersion.serviceId}\0${serviceVersion.buildId}`;
    if (!seenBuilds.has(buildKey)) {
      seenBuilds.add(buildKey);
      pointers.push(withSourceRoot(root, pointer));
    }
    serviceVersionBindings.push(serviceVersion);
  }

  if (pointers.length === 0) {
    if (options.allowEmpty) {
      return {
        fingerprintSources,
        mode: "release",
        pointers,
        serviceVersionBindings,
      };
    }
    throw new Error(
      `artifact versions dir ${versionRoot} must contain service version pointers`,
    );
  }
  return {
    fingerprintSources,
    mode: "release",
    pointers,
    serviceVersionBindings,
  };
}

function withSourceRoot(
  root: string,
  pointer: ArtifactPointer,
): SourcedArtifactPointer {
  return {
    ...pointer,
    sourceRoot: root,
  };
}

async function readJsonFilesRecursive(root: string): Promise<string[]> {
  const entries = (await readdir(root, { withFileTypes: true })).sort(
    (left, right) => left.name.localeCompare(right.name),
  );
  const paths: string[] = [];
  for (const entry of entries) {
    const path = resolve(root, entry.name);
    if (entry.isDirectory()) {
      paths.push(...(await readJsonFilesRecursive(path)));
    } else if (entry.isFile() && extname(entry.name) === ".json") {
      paths.push(path);
    }
  }
  return paths;
}

function serviceIdJsonPathSegments(root: string, path: string): string[] {
  const parts = relativePathSegments(root, path);
  const fileName = parts.at(-1);
  if (fileName === undefined || !fileName.endsWith(".json")) {
    throw new Error(`${path} must be a .json file`);
  }
  return [...parts.slice(0, -1), fileName.slice(0, -".json".length)];
}

function relativePathSegments(root: string, path: string): string[] {
  return relative(root, path)
    .split(/[\\/]/)
    .filter((part) => part.length > 0);
}

function samePathSegments(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return (
    left.length === right.length &&
    left.every((part, index) => part === right[index])
  );
}
