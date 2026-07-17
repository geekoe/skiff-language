import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { access } from "node:fs/promises";
import { isAbsolute, join, resolve } from "node:path";

import type {
  PackageUnitArtifactPointer,
  ServiceUnitArtifactPointer,
  ValidatedArtifactContent,
  ValidatedServiceArtifactClosure,
} from "./types.js";

const IDENTITY_CLI_ENV = "SKIFF_ARTIFACT_IDENTITY_CLI";
const IDENTITY_CLI_BINARY = process.platform === "win32"
  ? "skiff-artifact-identity.exe"
  : "skiff-artifact-identity";
const DYNAMIC_BUILD_ID_PATTERN =
  /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/;

export interface IdentityCliResolutionOptions {
  identityCliPath?: string;
  releaseMode?: boolean;
}

export interface IdentityCliArtifactInput {
  key: string;
  artifactRoot: string;
  serviceId: string;
  serviceVersion?: string;
  serviceAssembly: {
    assemblyIdentity: string;
    assemblyPath: string;
  };
  serviceUnit: ServiceUnitArtifactPointer;
  packageUnits: readonly PackageUnitArtifactPointer[];
}

/** Validate one complete router load candidate set in exactly one CLI process. */
export async function validateArtifactClosuresWithIdentityCli(
  inputs: readonly IdentityCliArtifactInput[],
  options: IdentityCliResolutionOptions,
): Promise<ReadonlyMap<string, ValidatedServiceArtifactClosure>> {
  if (inputs.length === 0) {
    throw new Error("artifact identity CLI transaction requires at least one service");
  }
  const expected = new Map<string, IdentityCliArtifactInput>();
  for (const input of inputs) {
    if (expected.has(input.key)) {
      throw new Error(`artifact identity CLI input contains duplicate key ${input.key}`);
    }
    expected.set(input.key, input);
  }

  const resolution = resolveIdentityCliPath(options);
  if (resolution.path === undefined) {
    throw new Error(
      `artifact identity CLI is not configured; ${formatIdentityCliCandidates(resolution.candidates)}`,
    );
  }
  await assertIdentityCliExecutable(resolution.path, resolution.candidates);
  const stdout = await runIdentityCli(
    resolution.path,
    { services: inputs },
    resolution.candidates,
  );
  return readValidatedResults(stdout, expected, resolution.candidates);
}

function readValidatedResults(
  stdout: string,
  expected: ReadonlyMap<string, IdentityCliArtifactInput>,
  candidates: readonly IdentityCliCandidate[],
): ReadonlyMap<string, ValidatedServiceArtifactClosure> {
  const parsed = parseJsonObject(stdout, "artifact identity CLI stdout", candidates);
  const rawResults = parsed.results;
  if (!Array.isArray(rawResults) || rawResults.length !== expected.size) {
    throw new Error(
      `artifact identity CLI stdout.results must contain exactly ${expected.size} results; ${formatIdentityCliCandidates(candidates)}`,
    );
  }
  const results = new Map<string, ValidatedServiceArtifactClosure>();
  for (const [index, value] of rawResults.entries()) {
    const record = requireObject(value, `artifact identity CLI stdout.results[${index}]`);
    const key = requireString(record.key, `results[${index}].key`);
    const input = expected.get(key);
    if (input === undefined || results.has(key)) {
      throw new Error(`artifact identity CLI returned unexpected or duplicate key ${key}`);
    }
    const dynamicBuildId = requireString(
      record.dynamicBuildId,
      `results[${index}].dynamicBuildId`,
    );
    if (!DYNAMIC_BUILD_ID_PATTERN.test(dynamicBuildId)) {
      throw new Error(
        `artifact identity CLI results[${index}].dynamicBuildId must be skiff-service-build-v1:sha256:<64 lowercase hex>`,
      );
    }
    const assemblyIdentity = requireString(
      record.assemblyIdentity,
      `results[${index}].assemblyIdentity`,
    );
    if (assemblyIdentity !== input.serviceAssembly.assemblyIdentity) {
      throw new Error(`artifact identity CLI result ${key} assemblyIdentity mismatch`);
    }
    const serviceAssembly = readContent(
      record.serviceAssembly,
      `results[${index}].serviceAssembly`,
      input.serviceAssembly.assemblyPath,
    );
    const serviceUnit = readContent(
      record.serviceUnit,
      `results[${index}].serviceUnit`,
      input.serviceUnit.unitPath,
    );
    if (!Array.isArray(record.packageUnits)) {
      throw new Error(`artifact identity CLI results[${index}].packageUnits must be an array`);
    }
    if (record.packageUnits.length !== input.packageUnits.length) {
      throw new Error(`artifact identity CLI result ${key} packageUnits count mismatch`);
    }
    const expectedPackagePaths = new Set(
      input.packageUnits.map((unit) => unit.unitPath),
    );
    const packageUnits = record.packageUnits.map((content, packageIndex) => {
      const loaded = readContent(
        content,
        `results[${index}].packageUnits[${packageIndex}]`,
      );
      if (!expectedPackagePaths.delete(loaded.path)) {
        throw new Error(
          `artifact identity CLI result ${key} returned unexpected duplicate package path ${loaded.path}`,
        );
      }
      return loaded;
    });
    if (expectedPackagePaths.size !== 0) {
      throw new Error(`artifact identity CLI result ${key} omitted package unit content`);
    }
    results.set(key, {
      key,
      dynamicBuildId,
      assemblyIdentity,
      serviceAssembly,
      serviceUnit,
      packageUnits,
    });
  }
  return results;
}

function readContent(
  value: unknown,
  label: string,
  expectedPath?: string,
): ValidatedArtifactContent {
  const record = requireObject(value, label);
  const path = requireString(record.path, `${label}.path`);
  if (expectedPath !== undefined && path !== expectedPath) {
    throw new Error(`${label}.path must be ${expectedPath}`);
  }
  const content = requireObject(record.value, `${label}.value`);
  return { path, value: content };
}

function parseJsonObject(
  text: string,
  label: string,
  candidates: readonly IdentityCliCandidate[],
): Record<string, unknown> {
  try {
    return requireObject(JSON.parse(text) as unknown, label);
  } catch (error) {
    throw new Error(
      `${label} must be valid JSON object; ${formatIdentityCliCandidates(candidates)}`,
      { cause: error },
    );
  }
}

function requireObject(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requireString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function resolveIdentityCliPath(
  options: IdentityCliResolutionOptions,
): { path?: string; candidates: IdentityCliCandidate[] } {
  const candidates: IdentityCliCandidate[] = [];
  if (options.identityCliPath !== undefined) {
    candidates.push({ source: "config/override", path: options.identityCliPath });
    return { path: options.identityCliPath, candidates };
  }
  const envPath = process.env[IDENTITY_CLI_ENV];
  if (envPath !== undefined && envPath.trim().length > 0) {
    const path = resolveProcessPath(envPath);
    candidates.push({ source: IDENTITY_CLI_ENV, path });
    return { path, candidates };
  }
  if (options.releaseMode === true) {
    candidates.push({ source: "local dev fallback", path: "(disabled in release mode)" });
    return { candidates };
  }
  const fallback = defaultDevIdentityCliPath();
  candidates.push({ source: "local dev fallback", path: fallback });
  return { path: fallback, candidates };
}

function defaultDevIdentityCliPath(): string {
  const devHome = process.env.SKIFF_DEV_HOME?.trim() ||
    join(process.cwd(), ".skiff-instance", "dev-home");
  return join(resolve(devHome), "bin", IDENTITY_CLI_BINARY);
}

function resolveProcessPath(value: string): string {
  const trimmed = value.trim();
  return isAbsolute(trimmed) ? trimmed : resolve(trimmed);
}

async function assertIdentityCliExecutable(
  path: string,
  candidates: readonly IdentityCliCandidate[],
): Promise<void> {
  try {
    await access(path, fsConstants.X_OK);
  } catch (error) {
    throw new Error(
      `artifact identity CLI is not executable at ${path}; ${formatIdentityCliCandidates(candidates)}`,
      { cause: error },
    );
  }
}

function runIdentityCli(
  path: string,
  payload: unknown,
  candidates: readonly IdentityCliCandidate[],
): Promise<string> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(path, ["runtime-program-build-id"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
    child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
    child.on("error", (error) => {
      reject(new Error(
        `failed to spawn artifact identity CLI ${path}: ${error.message}; ${formatIdentityCliCandidates(candidates)}`,
        { cause: error },
      ));
    });
    child.on("exit", (code, signal) => {
      const stderrText = Buffer.concat(stderr).toString("utf8");
      if (code === 0) {
        resolvePromise(Buffer.concat(stdout).toString("utf8"));
      } else {
        reject(new Error(
          `artifact identity CLI ${path} failed with ${signal ?? code}: ${identityCliErrorMessage(stderrText)}; ${formatIdentityCliCandidates(candidates)}`,
        ));
      }
    });
    child.stdin.end(`${JSON.stringify(payload)}\n`);
  });
}

function identityCliErrorMessage(stderr: string): string {
  const trimmed = stderr.trim();
  try {
    const parsed = requireObject(JSON.parse(trimmed) as unknown, "identity CLI error");
    const body = requireObject(parsed.error, "identity CLI error.error");
    const code = typeof body.code === "string" ? body.code : "error";
    const message = typeof body.message === "string" ? body.message : trimmed;
    return `${code}: ${message}`;
  } catch {
    return trimmed.length > 0 ? trimmed : "no stderr";
  }
}

interface IdentityCliCandidate {
  source: string;
  path: string;
}

function formatIdentityCliCandidates(
  candidates: readonly IdentityCliCandidate[],
): string {
  if (candidates.length === 0) {
    return "identity CLI candidates: config/override not set, SKIFF_ARTIFACT_IDENTITY_CLI not set";
  }
  return `identity CLI candidates: ${candidates
    .map((candidate) => `${candidate.source}=${candidate.path}`)
    .join(", ")}`;
}
