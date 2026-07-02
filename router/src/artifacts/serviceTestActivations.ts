import { readFile, stat } from "node:fs/promises";
import { resolve } from "node:path";

import {
  parseConfigYamlSource,
  type ConfigShape,
} from "../config/index.js";
import { isPublicationId } from "../publicationId.js";
import { resolveArtifactPath } from "./artifactPath.js";
import {
  buildServiceConfigActivationFromSources,
  type ConfigActivation,
  type PackageConfigActivationInput,
} from "./configActivation.js";
import { serviceIdPathSegments } from "./pathProjection.js";
import {
  assertRecord,
  readRequiredArray,
  readRequiredString,
} from "./readUtils.js";
import type { LoadedServiceConfigActivation } from "./types.js";

const SERVICE_TEST_ACTIVATIONS_SCHEMA_VERSION =
  "skiff-service-test-activations-v1";
const ACTIVATION_IDENTITY_PATTERN =
  /^skiff-runtime-activation-v1:opaque:[A-Za-z0-9._:-]+$/;
const POINTER_BUILD_ID_PATTERN =
  /^skiff-service-build-v1:sha256:([a-f0-9]{64})$/;

export async function readServiceTestConfigActivations(input: {
  root: string;
  indexPath: string;
  serviceId: string;
  buildId: string;
  pointerBuildId: string;
  operationTargets: readonly string[];
  configShape: ConfigShape;
  configActivation: ConfigActivation;
  packageConfigs: readonly PackageConfigActivationInput[];
}): Promise<LoadedServiceConfigActivation[]> {
  const sidecarPath = serviceTestActivationsPath(
    input.serviceId,
    input.pointerBuildId,
  );
  const absoluteSidecarPath = resolve(input.root, sidecarPath);
  const sidecarStat = await stat(absoluteSidecarPath).catch(() => undefined);
  if (!sidecarStat?.isFile()) {
    return [];
  }
  const text = await readFile(absoluteSidecarPath, "utf8");
  const value = JSON.parse(text) as unknown;
  assertRecord(value, `${sidecarPath} service-test activations`);
  if (value.schemaVersion !== SERVICE_TEST_ACTIVATIONS_SCHEMA_VERSION) {
    throw new Error(
      `${sidecarPath}.schemaVersion must be ${SERVICE_TEST_ACTIVATIONS_SCHEMA_VERSION}`,
    );
  }
  if (value.serviceId !== input.serviceId) {
    throw new Error(`${sidecarPath}.serviceId must be ${input.serviceId}`);
  }
  if (value.pointerBuildId !== input.pointerBuildId) {
    throw new Error(
      `${sidecarPath}.pointerBuildId must be ${input.pointerBuildId}`,
    );
  }
  const operationTargets = new Set(input.operationTargets);
  const usedTargets = new Set<string>();
  const cases = readRequiredArray(value.cases, `${sidecarPath}.cases`);
  return Promise.all(
    cases.map(async (rawCase, index) => {
      const label = `${sidecarPath}.cases[${index}]`;
      assertRecord(rawCase, label);
      const activationIdentity = readRequiredString(
        rawCase.activationIdentity,
        `${label}.activationIdentity`,
      );
      if (!ACTIVATION_IDENTITY_PATTERN.test(activationIdentity)) {
        throw new Error(
          `${label}.activationIdentity must be skiff-runtime-activation-v1:opaque:<opaque id>`,
        );
      }
      const operationTarget = readRequiredString(
        rawCase.operationTarget,
        `${label}.operationTarget`,
      );
      if (!operationTargets.has(operationTarget)) {
        throw new Error(
          `${label}.operationTarget ${operationTarget} is not in service manifest operations`,
        );
      }
      if (usedTargets.has(operationTarget)) {
        throw new Error(
          `${label}.operationTarget ${operationTarget} is declared more than once`,
        );
      }
      usedTargets.add(operationTarget);
      const storageServiceId = readRequiredString(
        rawCase.storageServiceId,
        `${label}.storageServiceId`,
      );
      if (!isPublicationId(storageServiceId)) {
        throw new Error(`${label}.storageServiceId must be a publication id`);
      }
      const serviceDb = readServiceDb(rawCase.serviceDb, label);
      const configPath = readRequiredString(rawCase.configPath, `${label}.configPath`);
      const configText = await readFile(
        await resolveArtifactPath(input.root, configPath, input.indexPath),
        "utf8",
      );
      const configSource = parseConfigYamlSource(configText, {
        label: configPath,
        sourceClass: "bundle",
      });
      const payload = buildServiceConfigActivationFromSources({
        indexPath: input.indexPath,
        serviceId: input.serviceId,
        buildId: input.buildId,
        configShape: input.configShape,
        configActivation: input.configActivation,
        packageConfigs: input.packageConfigs,
        sources: [configSource],
        ...(serviceDb !== undefined ? { serviceDb } : {}),
        storageServiceId,
        activationIdentity,
      });
      if (payload === undefined) {
        throw new Error(`${label} did not produce a service config activation`);
      }
      return {
        operationTargets: [operationTarget],
        serviceId: input.serviceId,
        payload,
      };
    }),
  );
}

export function serviceTestActivationsPath(
  serviceId: string,
  pointerBuildId: string,
): string {
  const hash = pointerBuildIdHash(pointerBuildId);
  return [
    "dev",
    "service-test-activations",
    ...serviceIdPathSegments(serviceId),
    `${hash}.json`,
  ].join("/");
}

function pointerBuildIdHash(pointerBuildId: string): string {
  const match = POINTER_BUILD_ID_PATTERN.exec(pointerBuildId);
  if (!match) {
    throw new Error(
      `pointerBuildId must be skiff-service-build-v1:sha256:<64 lowercase hex>`,
    );
  }
  return match[1]!;
}

function readServiceDb(
  value: unknown,
  label: string,
): { mongoUrl: string } | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  assertRecord(value, `${label}.serviceDb`);
  const mongoUrl = readRequiredString(value.mongoUrl, `${label}.serviceDb.mongoUrl`);
  return { mongoUrl };
}
