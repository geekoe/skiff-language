import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  validateArtifactClosuresWithIdentityCli,
  type IdentityCliArtifactInput,
} from "../src/artifacts/identityCli.js";
import { resolveArtifactPath } from "../src/artifacts/artifactPath.js";
import { FilesystemRuntimeAssemblySnapshotLoader } from "../src/router/filesystemRuntimeAssemblySnapshotLoader.js";
import { ensureArtifactIdentityCli } from "./helpers/artifactIdentityCli.js";
import { writeCompilerGeneratedFixtureArtifactRoot } from "./helpers/compilerArtifacts.js";
import { writeMockIdentityCli } from "./helpers/mockIdentityCli.js";

const DYNAMIC_BUILD_ID =
  "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

describe("artifact identity CLI transaction", () => {
  it("validates multiple artifact roots in one batch and returns loaded content", async () => {
    const temp = await mkdtemp(join(tmpdir(), "skiff-router-identity-batch-"));
    try {
      const first = await writeCandidate(join(temp, "first"), "first");
      const second = await writeCandidate(join(temp, "second"), "second");
      const capturePath = join(temp, "stdin.json");
      const identityCliPath = await writeMockIdentityCli({
        dir: join(temp, "bin"),
        capturePath,
        dynamicBuildId: DYNAMIC_BUILD_ID,
      });

      const results = await validateArtifactClosuresWithIdentityCli(
        [first, second],
        { identityCliPath },
      );

      expect(results.size).toBe(2);
      expect(results.get("first")?.serviceAssembly.value.service).toMatchObject({
        id: "example.com/first",
      });
      expect(results.get("second")?.serviceUnit.value.version).toBe("1.0.0");
      const captured = JSON.parse(await readFile(capturePath, "utf8")) as {
        services: unknown[];
      };
      expect(captured.services).toHaveLength(2);
    } finally {
      await rm(temp, { recursive: true, force: true });
    }
  });

  it("passes all seven package pointer fields and returns package content", async () => {
    const temp = await mkdtemp(join(tmpdir(), "skiff-router-identity-package-"));
    try {
      const candidate = await writeCandidate(temp, "package", true);
      const identityCliPath = await writeMockIdentityCli({ dir: join(temp, "bin") });
      const result = await validateArtifactClosuresWithIdentityCli(
        [candidate],
        { identityCliPath },
      );
      expect(result.get("package")?.packageUnits).toHaveLength(1);
      expect(result.get("package")?.packageUnits[0]?.value.packageId).toBe(
        "example.com/pkg",
      );
    } finally {
      await rm(temp, { recursive: true, force: true });
    }
  });

  it("fails closed on CLI failure, unavailable CLI, and incomplete stdout", async () => {
    const temp = await mkdtemp(join(tmpdir(), "skiff-router-identity-failure-"));
    try {
      const candidate = await writeCandidate(temp, "failure");
      const failingCli = await writeMockIdentityCli({
        dir: join(temp, "fail-bin"),
        exitCode: 2,
        stderrJson: {
          error: { code: "schema_invalid", message: "pointer hash mismatch" },
        },
      });
      await expect(
        validateArtifactClosuresWithIdentityCli([candidate], {
          identityCliPath: failingCli,
        }),
      ).rejects.toThrow(/schema_invalid: pointer hash mismatch/);

      await expect(
        validateArtifactClosuresWithIdentityCli([candidate], {
          identityCliPath: join(temp, "missing-cli"),
        }),
      ).rejects.toThrow(/not executable/);

      const incompleteCli = await writeMockIdentityCli({
        dir: join(temp, "bad-bin"),
        stdoutText: JSON.stringify({ results: [] }),
      });
      await expect(
        validateArtifactClosuresWithIdentityCli([candidate], {
          identityCliPath: incompleteCli,
        }),
      ).rejects.toThrow(/exactly 1 results/);
    } finally {
      await rm(temp, { recursive: true, force: true });
    }
  });

  it("loads exact current records from canonical compiler-authored paths", async () => {
    const temp = await mkdtemp(join(tmpdir(), "skiff-router-artifact-paths-"));
    try {
      const generated = await writeCompilerGeneratedFixtureArtifactRoot(temp);
      await expect(
        resolveArtifactPath(temp, generated.runtimeAssembly.recordPath, "RuntimeAssembly"),
      ).resolves.toEqual(await realpath(join(temp, generated.runtimeAssembly.recordPath)));
      await expect(
        resolveArtifactPath(temp, `../${generated.runtimeAssembly.recordPath}`, "escape"),
      ).rejects.toThrow(/canonical and relative|escapes/);
      await expect(
        new FilesystemRuntimeAssemblySnapshotLoader(temp).load(
          generated.runtimeAssembly.assembly,
        ),
      ).resolves.toMatchObject({
        assemblyIdentity: generated.runtimeAssembly.assembly.assemblyIdentity,
        resolvedContracts: [generated.serviceContract.contract],
        resolvedDeployments: [generated.serviceDeployment.deployment],
      });
    } finally {
      await rm(temp, { recursive: true, force: true });
    }
  }, 120_000);
});

async function writeCandidate(
  root: string,
  key: string,
  withPackage = false,
): Promise<IdentityCliArtifactInput> {
  const serviceId = `example.com/${key}`;
  const serviceUnitPath = `units/services/${key}/service.json`;
  const assemblyPath = `assemblies/services/${key}/assembly.json`;
  const assemblyIdentity =
    "skiff-service-assembly-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
  const serviceUnit = {
    schemaVersion: "skiff-service-unit-v1" as const,
    unitIdentity:
      "skiff-service-unit-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    unitHash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    unitPath: serviceUnitPath,
  };
  await writeJson(root, serviceUnitPath, {
    schemaVersion: "skiff-service-unit-v1",
    service: { id: serviceId },
    version: "1.0.0",
  });
  await writeJson(root, assemblyPath, {
    schemaVersion: "skiff-assembly-v1",
    kind: "service",
    service: { id: serviceId, assemblyIdentity },
    serviceUnit,
  });
  const packageUnits = [];
  if (withPackage) {
    const packageUnit = {
      schemaVersion: "skiff-package-unit-v2" as const,
      packageId: "example.com/pkg",
      version: "1.0.0",
      buildIdentity:
        "skiff-package-build-v10:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
      abiIdentity:
        "skiff-package-local-abi-v7:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
      unitHash: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
      unitPath: "units/packages/pkg/package.json",
    };
    packageUnits.push(packageUnit);
    await writeJson(root, packageUnit.unitPath, packageUnit);
  }
  return {
    key,
    artifactRoot: root,
    serviceId,
    serviceAssembly: { assemblyIdentity, assemblyPath },
    serviceUnit,
    packageUnits,
  };
}

async function writeJson(root: string, path: string, value: unknown): Promise<void> {
  const absolute = join(root, path);
  await mkdir(dirname(absolute), { recursive: true });
  await writeFile(absolute, JSON.stringify(value));
}
