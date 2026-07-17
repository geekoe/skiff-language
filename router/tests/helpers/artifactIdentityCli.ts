import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { access } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

let artifactIdentityCliPromise: Promise<string> | undefined;

export async function ensureArtifactIdentityCli(): Promise<string> {
  artifactIdentityCliPromise ??= buildArtifactIdentityCli();
  return await artifactIdentityCliPromise;
}

export function runIdentityCli(
  cliPath: string,
  args: readonly string[],
  input: unknown,
): Promise<string> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(cliPath, args, {
      stdio: ["pipe", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolvePromise(stdout);
        return;
      }
      reject(
        new Error(
          `${cliPath} ${args.join(" ")} failed with ${signal ?? code}: ${stderr}`,
        ),
      );
    });
    child.stdin.end(JSON.stringify(input));
  });
}

async function buildArtifactIdentityCli(): Promise<string> {
  const repoRoot = resolve(
    dirname(fileURLToPath(import.meta.url)),
    "../../..",
  );
  const binary = process.platform === "win32"
    ? "skiff-artifact-identity.exe"
    : "skiff-artifact-identity";
  const cliPath = join(repoRoot, "build", "cargo-target", "debug", binary);
  await runCommand(
    "cargo",
    [
      "build",
      "--manifest-path",
      "artifact-identity/Cargo.toml",
      "--bin",
      "skiff-artifact-identity",
    ],
    repoRoot,
  );
  await access(cliPath, fsConstants.X_OK);
  return cliPath;
}

function runCommand(
  command: string,
  args: readonly string[],
  cwd: string,
): Promise<void> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd,
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolvePromise();
        return;
      }
      reject(new Error(`${command} ${args.join(" ")} failed with ${signal ?? code}`));
    });
  });
}
