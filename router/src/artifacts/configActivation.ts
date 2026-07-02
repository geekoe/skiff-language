import { readFile, stat } from "node:fs/promises";
import { resolve } from "node:path";

import { sha256Hex, stableStringify } from "../manifest/identity.js";
import {
  buildResolvedConfig,
  defaultConfigSourceSpecs,
  parseConfigYamlSource,
  validateConfigShapeEntries,
  type ConfigShape,
  type ConfigSource,
  type JsonObject,
} from "../config/index.js";
import type {
  RuntimeConfigActivationPayload,
  RuntimePackageConfigActivationPayload,
  RuntimeServiceDbConfigInput,
} from "../protocol/envelope.js";
import { serviceIdPath, serviceIdPathSegments } from "./pathProjection.js";

export interface ConfigActivation {
  schemaVersion: "skiff-config-activation-v1";
  hasPaths: string[];
}

export interface PackageConfigActivationInput {
  packageId: string;
  alias: string;
  defaultConfig: JsonObject;
  configShape: ConfigShape;
  configActivation: ConfigActivation;
}

export async function buildServiceConfigActivation(input: {
  root: string;
  indexPath: string;
  serviceId: string;
  buildId: string;
  configShape: ConfigShape;
  configActivation: ConfigActivation;
  packageConfigs?: readonly PackageConfigActivationInput[];
  configProfile?: string;
  serviceDb?: RuntimeServiceDbConfigInput;
}): Promise<RuntimeConfigActivationPayload | undefined> {
  const sources = await readExistingConfigSources(
    input.root,
    input.serviceId,
    input.configProfile,
  );
  return buildServiceConfigActivationFromSources({
    indexPath: input.indexPath,
    serviceId: input.serviceId,
    buildId: input.buildId,
    configShape: input.configShape,
    configActivation: input.configActivation,
    sources,
    ...(input.packageConfigs !== undefined
      ? { packageConfigs: input.packageConfigs }
      : {}),
    ...(input.serviceDb !== undefined ? { serviceDb: input.serviceDb } : {}),
    storageServiceId: input.serviceId,
    ...(input.configProfile !== undefined
      ? { configProfile: input.configProfile }
      : {}),
  });
}

export function buildServiceConfigActivationFromSources(input: {
  indexPath: string;
  serviceId: string;
  buildId: string;
  configShape: ConfigShape;
  configActivation: ConfigActivation;
  packageConfigs?: readonly PackageConfigActivationInput[];
  sources: readonly ConfigSource[];
  serviceDb?: RuntimeServiceDbConfigInput;
  storageServiceId: string;
  activationIdentity?: string;
  configProfile?: string;
}): RuntimeConfigActivationPayload | undefined {
  const packageConfigs = input.packageConfigs ?? [];
  validatePackageAliases(packageConfigs, input.indexPath);
  validateConfigSourceNamespaces(input.sources, input.indexPath);
  const serviceSources = input.sources.flatMap((source) =>
    sourceForService(source, input.indexPath),
  );
  const packagePayloads = buildPackageConfigActivations({
    indexPath: input.indexPath,
    sources: input.sources,
    packageConfigs,
  });
  if (
    input.configShape.entries.length === 0 &&
    input.configActivation.hasPaths.length === 0 &&
    input.serviceDb === undefined &&
    packagePayloads.length === 0 &&
    input.activationIdentity === undefined
  ) {
    return undefined;
  }
  if (
    serviceSources.length === 0 &&
    input.configShape.entries.some((entry) => entry.required)
  ) {
    const expected = defaultConfigSourceSpecs(input.configProfile)
      .map((spec) => serviceConfigSourceLabel(input.serviceId, spec.label))
      .join(", ");
    throw new Error(
      `${input.indexPath} serviceAssembly.configShape requires at least one config source (${expected})`,
    );
  }

  const resolved = buildResolvedConfig({
    configShape: input.configShape.entries,
    sources: serviceSources,
  });

  const serviceConfigShape: ConfigShape = {
    schemaVersion: input.configShape.schemaVersion,
    entries: input.configShape.entries,
  };
  const resolvedConfigId = resolvedConfigIdentity({
    serviceId: input.serviceId,
    buildId: input.buildId,
    resolvedConfig: resolved.resolvedConfig,
    configShape: serviceConfigShape,
  });
  const activationIdentity =
    input.activationIdentity ??
    serviceActivationIdentity({
      serviceId: input.serviceId,
      buildId: input.buildId,
      resolvedConfigIdentity: resolvedConfigId,
      serviceDb: input.serviceDb,
      storageServiceId: input.storageServiceId,
      packageConfigs: packagePayloads,
    });

  const payload: RuntimeConfigActivationPayload = {
    serviceId: input.serviceId,
    buildId: input.buildId,
    activationIdentity,
    resolvedConfigIdentity: resolvedConfigId,
    resolvedConfig: resolved.resolvedConfig,
    redactedResolvedConfig: resolved.redactedResolvedConfig,
    redactionProjectionIdentity: resolved.redactionProjectionIdentity,
    configShape: serviceConfigShape,
  };
  if (input.serviceDb !== undefined) {
    payload.serviceDb = {
      mongoUrl: input.serviceDb.mongoUrl,
      storageServiceId: input.storageServiceId,
    };
  }
  if (packagePayloads.length > 0) {
    payload.packageConfigs = packagePayloads;
  }
  return payload;
}

// Deterministic identity for a resolved config. The same resolved config
// content (under the same service/build/package and shape) always yields the
// same identity, so reloading an unchanged build does not mint a fresh one.
function resolvedConfigIdentity(input: {
  serviceId?: string;
  buildId?: string;
  packageId?: string;
  alias?: string;
  resolvedConfig: JsonObject;
  configShape: ConfigShape;
}): string {
  return `skiff-config-resolved-v1:opaque:${sha256Hex(
    stableStringify({
      serviceId: input.serviceId ?? null,
      buildId: input.buildId ?? null,
      packageId: input.packageId ?? null,
      alias: input.alias ?? null,
      resolvedConfig: input.resolvedConfig,
      configShape: input.configShape,
    }),
  )}`;
}

// Deterministic activation identity for a service activation. It folds in the
// service/build plus everything that distinguishes one activation of that
// build from another (resolved config, serviceDb binding, and the per-package
// resolved config identities). Same build + same config => same activation
// identity => same runtime_id => the runtime/router dedup-by-id paths collapse
// repeated reloads instead of accumulating activations. A genuine config
// change produces a different resolvedConfigIdentity and therefore a different
// activation identity.
function serviceActivationIdentity(input: {
  serviceId: string;
  buildId: string;
  resolvedConfigIdentity: string;
  serviceDb: RuntimeServiceDbConfigInput | undefined;
  storageServiceId: string;
  packageConfigs: readonly RuntimePackageConfigActivationPayload[];
}): string {
  return `skiff-runtime-activation-v1:opaque:${sha256Hex(
    stableStringify({
      serviceId: input.serviceId,
      buildId: input.buildId,
      resolvedConfigIdentity: input.resolvedConfigIdentity,
      serviceDb:
        input.serviceDb !== undefined
          ? {
              mongoUrl: input.serviceDb.mongoUrl,
              storageServiceId: input.storageServiceId,
            }
          : null,
      packageConfigs: input.packageConfigs.map((packageConfig) => ({
        packageId: packageConfig.packageId,
        alias: packageConfig.alias,
        resolvedConfigIdentity: packageConfig.resolvedConfigIdentity,
      })),
    }),
  )}`;
}

function buildPackageConfigActivations(input: {
  indexPath: string;
  sources: readonly ConfigSource[];
  packageConfigs: readonly PackageConfigActivationInput[];
}): RuntimePackageConfigActivationPayload[] {
  const payloads: RuntimePackageConfigActivationPayload[] = [];
  for (const packageConfig of input.packageConfigs) {
    const namespacedSources = input.sources.flatMap((source) =>
      sourceForPackageAlias(source, packageConfig.alias, input.indexPath),
    );
    const defaultSource = defaultConfigSource(packageConfig);
    const packageSources = defaultSource
      ? [defaultSource, ...namespacedSources]
      : namespacedSources;
    if (
      packageSources.length === 0 &&
      packageConfig.configShape.entries.length === 0 &&
      packageConfig.configActivation.hasPaths.length === 0
    ) {
      continue;
    }
    const resolved = buildResolvedConfig({
      configShape: packageConfig.configShape.entries,
      sources: packageSources,
    });
    payloads.push({
      packageId: packageConfig.packageId,
      alias: packageConfig.alias,
      resolvedConfigIdentity: resolvedConfigIdentity({
        packageId: packageConfig.packageId,
        alias: packageConfig.alias,
        resolvedConfig: resolved.resolvedConfig,
        configShape: {
          schemaVersion: packageConfig.configShape.schemaVersion,
          entries: packageConfig.configShape.entries,
        },
      }),
      resolvedConfig: resolved.resolvedConfig,
      redactedResolvedConfig: resolved.redactedResolvedConfig,
      redactionProjectionIdentity: resolved.redactionProjectionIdentity,
      configShape: {
        schemaVersion: packageConfig.configShape.schemaVersion,
        entries: packageConfig.configShape.entries,
      },
    });
  }
  return payloads;
}

function validatePackageAliases(
  packageConfigs: readonly PackageConfigActivationInput[],
  indexPath: string,
): void {
  const seen = new Map<string, string>();
  for (const packageConfig of packageConfigs) {
    validateConfigShapeEntries(
      [
        {
          path: `packages.${packageConfig.alias}`,
          type: "JsonObject",
          required: false,
        },
      ],
      `${indexPath} package config namespace ${packageConfig.packageId}`,
    );
    const previousPackageId = seen.get(packageConfig.alias);
    if (
      previousPackageId !== undefined &&
      previousPackageId !== packageConfig.packageId
    ) {
      throw new Error(
        `${indexPath} package config namespace packages.${packageConfig.alias} is used by both ${previousPackageId} and ${packageConfig.packageId}`,
      );
    }
    seen.set(packageConfig.alias, packageConfig.packageId);
  }
}

function defaultConfigSource(
  packageConfig: PackageConfigActivationInput,
): ConfigSource | undefined {
  if (Object.keys(packageConfig.defaultConfig).length === 0) {
    return undefined;
  }
  return {
    sourceClass: "bundle",
    label: `artifact package config ${packageConfig.packageId}`,
    value: cloneJsonObject(packageConfig.defaultConfig),
  };
}

function sourceForPackageAlias(
  source: ConfigSource,
  alias: string,
  indexPath: string,
): ConfigSource[] {
  if (!Object.prototype.hasOwnProperty.call(source.value, "packages")) {
    return [];
  }
  const packages = source.value.packages;
  if (!isJsonObject(packages)) {
    throw new Error(
      `${indexPath} package config namespace packages in ${source.label} must be an object`,
    );
  }
  if (!Object.prototype.hasOwnProperty.call(packages, alias)) {
    return [];
  }
  const value = packages[alias];
  if (!isJsonObject(value)) {
    throw new Error(
      `${indexPath} package config namespace packages.${alias} in ${source.label} must be an object`,
    );
  }
  return [
    {
      sourceClass: source.sourceClass,
      label: `${source.label}:packages.${alias}`,
      value: cloneJsonObject(value),
    },
  ];
}

function sourceForService(
  source: ConfigSource,
  indexPath: string,
): ConfigSource[] {
  if (!Object.prototype.hasOwnProperty.call(source.value, "service")) {
    return [];
  }
  const value = source.value.service;
  if (!isJsonObject(value)) {
    throw new Error(
      `${indexPath} service config namespace service in ${source.label} must be an object`,
    );
  }
  return [
    {
      sourceClass: source.sourceClass,
      label: `${source.label}:service`,
      value: cloneJsonObject(value),
    },
  ];
}

function validateConfigSourceNamespaces(
  sources: readonly ConfigSource[],
  indexPath: string,
): void {
  for (const source of sources) {
    for (const key of Object.keys(source.value)) {
      if (key !== "service" && key !== "packages") {
        throw new Error(
          `${indexPath} config source ${source.label} top-level key ${key} is invalid; use service or packages`,
        );
      }
    }
    if (
      Object.prototype.hasOwnProperty.call(source.value, "packages") &&
      !isJsonObject(source.value.packages)
    ) {
      throw new Error(
        `${indexPath} package config namespace packages in ${source.label} must be an object`,
      );
    }
    if (
      Object.prototype.hasOwnProperty.call(source.value, "service") &&
      !isJsonObject(source.value.service)
    ) {
      throw new Error(
        `${indexPath} service config namespace service in ${source.label} must be an object`,
      );
    }
  }
}

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function cloneJsonObject(value: JsonObject): JsonObject {
  return JSON.parse(JSON.stringify(value)) as JsonObject;
}

export function readConfigActivation(
  value: unknown,
  label: string,
): ConfigActivation {
  if (value === undefined || value === null) {
    return emptyConfigActivation();
  }
  if (!isRecord(value)) {
    throw new Error(`${label} must be an object`);
  }
  if (value.schemaVersion !== "skiff-config-activation-v1") {
    throw new Error(
      `${label}.schemaVersion must be skiff-config-activation-v1`,
    );
  }
  if (!Array.isArray(value.hasPaths)) {
    throw new Error(`${label}.hasPaths must be an array`);
  }
  return {
    schemaVersion: "skiff-config-activation-v1",
    hasPaths: validateConfigActivationHasPaths(
      value.hasPaths,
      `${label}.hasPaths`,
    ),
  };
}

export function emptyConfigActivation(): ConfigActivation {
  return {
    schemaVersion: "skiff-config-activation-v1",
    hasPaths: [],
  };
}

function validateConfigActivationHasPaths(
  paths: readonly unknown[],
  label: string,
): string[] {
  const unique = new Set<string>();
  for (let index = 0; index < paths.length; index += 1) {
    const path = paths[index];
    if (typeof path !== "string") {
      throw new Error(`${label}[${index}] must be a string`);
    }
    validateConfigShapeEntries(
      [{ path, type: "Json", required: false }],
      `${label}[${index}]`,
    );
    unique.add(path);
  }
  return Array.from(unique).sort((left, right) => left.localeCompare(right));
}

async function readExistingConfigSources(
  root: string,
  serviceId: string,
  configProfile: string | undefined,
): Promise<ConfigSource[]> {
  const sources: ConfigSource[] = [];
  for (const spec of defaultConfigSourceSpecs(configProfile)) {
    const path = serviceConfigSourcePath(root, serviceId, spec.path);
    const pathStat = await stat(path).catch(() => undefined);
    if (!pathStat?.isFile()) {
      continue;
    }
    const text = await readFile(path, "utf8");
    sources.push(
      parseConfigYamlSource(text, {
        label: spec.label,
        sourceClass: spec.sourceClass,
      }),
    );
  }
  return sources;
}

function serviceConfigSourcePath(
  root: string,
  serviceId: string,
  configPath: string,
): string {
  return resolve(
    root,
    "configs",
    "services",
    ...serviceIdPathSegments(serviceId),
    configPath,
  );
}

function serviceConfigSourceLabel(
  serviceId: string,
  configPath: string,
): string {
  return `configs/services/${serviceIdPath(serviceId)}/${configPath}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
