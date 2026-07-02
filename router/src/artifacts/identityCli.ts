import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { access } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";

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

export async function computeRuntimeProgramBuildIdWithIdentityCli(input: {
  artifactRoot: string;
  serviceUnit: Record<string, unknown>;
} & IdentityCliResolutionOptions): Promise<string> {
  const resolution = resolveIdentityCliPath(input);
  if (resolution.path === undefined) {
    throw new Error(
      `artifact identity CLI is not configured; ${formatIdentityCliCandidates(resolution.candidates)}`,
    );
  }
  await assertIdentityCliExecutable(resolution.path, resolution.candidates);
  const key = "service";
  const stdout = await runIdentityCli(resolution.path, {
    artifactRoot: input.artifactRoot,
    services: [
      {
        key,
        serviceUnit: input.serviceUnit,
      },
    ],
  }, resolution.candidates);
  return readDynamicBuildId(stdout, key, resolution.candidates);
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
  const env = process.env;
  const devHome =
    env.SKIFF_DEV_HOME && env.SKIFF_DEV_HOME.trim().length > 0
      ? env.SKIFF_DEV_HOME
      : join(env.HOME || env.USERPROFILE || homedir(), ".skiff", "dev");
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
      reject(
        new Error(
          `failed to spawn artifact identity CLI ${path}: ${error.message}; ${formatIdentityCliCandidates(candidates)}`,
          { cause: error },
        ),
      );
    });
    child.on("exit", (code, signal) => {
      const stderrText = Buffer.concat(stderr).toString("utf8");
      if (code === 0) {
        resolvePromise(Buffer.concat(stdout).toString("utf8"));
        return;
      }
      reject(
        new Error(
          `artifact identity CLI ${path} failed with ${signal ?? code}: ${identityCliErrorMessage(stderrText)}; ${formatIdentityCliCandidates(candidates)}`,
        ),
      );
    });
    child.stdin.end(`${JSON.stringify(payload)}\n`);
  });
}

function readDynamicBuildId(
  stdout: string,
  expectedKey: string,
  candidates: readonly IdentityCliCandidate[],
): string {
  let parsed: unknown;
  try {
    parsed = JSON.parse(stdout);
  } catch (error) {
    throw new Error(
      `artifact identity CLI returned invalid JSON stdout; ${formatIdentityCliCandidates(candidates)}`,
      { cause: error },
    );
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(
      `artifact identity CLI stdout must be an object; ${formatIdentityCliCandidates(candidates)}`,
    );
  }
  const results = (parsed as Record<string, unknown>).results;
  if (!Array.isArray(results) || results.length !== 1) {
    throw new Error(
      `artifact identity CLI stdout.results must contain exactly one result; ${formatIdentityCliCandidates(candidates)}`,
    );
  }
  const result = results[0];
  if (!result || typeof result !== "object" || Array.isArray(result)) {
    throw new Error(
      `artifact identity CLI stdout.results[0] must be an object; ${formatIdentityCliCandidates(candidates)}`,
    );
  }
  const record = result as Record<string, unknown>;
  if (record.key !== expectedKey) {
    throw new Error(
      `artifact identity CLI stdout.results[0].key must be ${expectedKey}; ${formatIdentityCliCandidates(candidates)}`,
    );
  }
  if (
    typeof record.dynamicBuildId !== "string" ||
    !DYNAMIC_BUILD_ID_PATTERN.test(record.dynamicBuildId)
  ) {
    throw new Error(
      `artifact identity CLI stdout.results[0].dynamicBuildId must be skiff-service-build-v1:sha256:<64 lowercase hex>; ${formatIdentityCliCandidates(candidates)}`,
    );
  }
  return record.dynamicBuildId;
}

function identityCliErrorMessage(stderr: string): string {
  const trimmed = stderr.trim();
  if (trimmed.length === 0) {
    return "no stderr";
  }
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return trimmed;
    }
    const error = (parsed as Record<string, unknown>).error;
    if (!error || typeof error !== "object" || Array.isArray(error)) {
      return trimmed;
    }
    const message = (error as Record<string, unknown>).message;
    const code = (error as Record<string, unknown>).code;
    if (typeof message === "string" && typeof code === "string") {
      return `${code}: ${message}`;
    }
    if (typeof message === "string") {
      return message;
    }
    return trimmed;
  } catch {
    return trimmed;
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
