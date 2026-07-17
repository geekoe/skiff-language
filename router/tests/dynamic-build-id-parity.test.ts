import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  validateArtifactClosuresWithIdentityCli,
  type IdentityCliArtifactInput,
} from "../src/artifacts/identityCli.js";
import { resolveArtifactPath } from "../src/artifacts/artifactPath.js";
import { ensureArtifactIdentityCli } from "./helpers/artifactIdentityCli.js";
import { writeCompilerGeneratedWebSocketFixtureArtifactRoot } from "./helpers/compilerArtifacts.js";
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

  it("uses the shared Rust/router artifact path and coordinate cases", async () => {
    const fixture = JSON.parse(
      await readFile(
        new URL(
          "../../cross-system-fixtures/artifact-reference-validation/cases.json",
          import.meta.url,
        ),
        "utf8",
      ),
    ) as ArtifactReferenceFixture;
    const identityCliPath = await ensureArtifactIdentityCli();
    const temp = await mkdtemp(join(tmpdir(), "skiff-router-artifact-paths-"));
    try {
      const baseRoot = join(temp, "base");
      const closure = await writeCompilerGeneratedClosure(baseRoot);
      for (const [index, testCase] of fixture.cases.entries()) {
        expect(testCase.appliesTo).toEqual(
          expect.arrayContaining(["runtime", "router"]),
        );
        const root = join(temp, String(index));
        await cp(baseRoot, root, { recursive: true });
        const path = renderFixturePath(testCase.path, closure);
        const candidate: IdentityCliArtifactInput = {
          ...closure.input,
          key: `case-${index}`,
          artifactRoot: root,
          serviceAssembly: {
            ...closure.input.serviceAssembly,
            assemblyPath: path,
          },
        };
        if (testCase.materialize === true) {
          await writeJson(root, path, closure.assemblyValue);
        }

        if (testCase.validation === "artifactRelativePath") {
          const resolution = resolveArtifactPath(root, path, testCase.name);
          if (testCase.valid) {
            await expect(resolution, testCase.name).resolves.toEqual(
              expect.any(String),
            );
          } else {
            await expect(resolution, testCase.name).rejects.toThrow();
          }
          continue;
        }

        const validation = validateArtifactClosuresWithIdentityCli(
          [candidate],
          { identityCliPath },
        );
        if (testCase.valid) {
          await expect(validation, testCase.name).resolves.toBeInstanceOf(Map);
        } else {
          await expect(validation, testCase.name).rejects.toThrow();
        }
      }
    } finally {
      await rm(temp, { recursive: true, force: true });
    }
  }, 120_000);
});

interface ArtifactReferenceFixture {
  serviceId: string;
  cases: ArtifactReferenceCase[];
}

interface ArtifactReferenceCase {
  name: string;
  appliesTo: string[];
  validation: "artifactRelativePath" | "serviceAssemblyCoordinate";
  path: string;
  materialize?: boolean;
  valid: boolean;
}

interface CanonicalClosureFixture {
  input: IdentityCliArtifactInput;
  assemblyHash: string;
  serviceStorageSegment: string;
  assemblyValue: Record<string, unknown>;
}

async function writeCompilerGeneratedClosure(
  root: string,
): Promise<CanonicalClosureFixture> {
  const generated = await writeCompilerGeneratedWebSocketFixtureArtifactRoot(root);
  const assemblySegments = generated.serviceAssembly.assemblyPath.split("/");
  const assemblyFile = assemblySegments.at(-1);
  const serviceStorageSegment = assemblySegments.at(-2);
  if (
    assemblyFile === undefined
    || !assemblyFile.endsWith(".json")
    || serviceStorageSegment === undefined
  ) {
    throw new Error(
      `compiler generated non-canonical assembly path ${generated.serviceAssembly.assemblyPath}`,
    );
  }
  const assemblyValue = JSON.parse(
    await readFile(join(root, generated.serviceAssembly.assemblyPath), "utf8"),
  ) as Record<string, unknown>;

  return {
    assemblyHash: assemblyFile.slice(0, -".json".length),
    serviceStorageSegment,
    assemblyValue,
    input: {
      key: "canonical",
      artifactRoot: root,
      serviceId: generated.serviceId,
      serviceAssembly: generated.serviceAssembly,
      serviceUnit: generated.serviceUnit,
      packageUnits: generated.packageUnits,
    },
  };
}

function renderFixturePath(
  template: string,
  closure: CanonicalClosureFixture,
): string {
  return template
    .replaceAll("{serviceStorageSegment}", closure.serviceStorageSegment)
    .replaceAll("{assemblyHash}", closure.assemblyHash)
    .replaceAll("{otherHash}", "b".repeat(64));
}

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
      schemaVersion: "skiff-package-unit-v1" as const,
      packageId: "example.com/pkg",
      version: "1.0.0",
      buildIdentity:
        "skiff-package-build-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
      abiIdentity:
        "skiff-package-local-abi-v2:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
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
