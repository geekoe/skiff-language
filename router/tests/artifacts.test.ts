import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { constants as fsConstants } from "node:fs";
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";

import { stableStringify } from "../src/manifest/identity.js";
import {
  loadManifest as loadRuntimeManifest,
  packageHttpHandlerTarget,
} from "../src/manifest/loadManifest.js";
import { serviceAssemblyHashInput as routerServiceAssemblyHashInput } from "../src/artifacts/identity.js";
import { loadRouterArtifactRoot } from "../src/artifacts/loadArtifactRoot.js";
import { serviceIdPathSegments } from "../src/artifacts/pathProjection.js";
import { readActiveArtifactPointers } from "../src/artifacts/pointers.js";
import { publicationStorageSegment } from "../src/publicationId.js";
import {
  writeCompilerGeneratedWebSocketFixtureArtifactRoot,
  writeCompilerGeneratedWebSocketFixtureDevReloadArtifactRoot,
} from "./helpers/compilerArtifacts.js";

const tempDirs: string[] = [];
const originalIdentityCliEnv = process.env.SKIFF_ARTIFACT_IDENTITY_CLI;
const SERVICE_ID = "example.com/websocket_fixture";
const CONTRACT_IDENTITY = fixtureIdentity("skiff-protocol-v1", SERVICE_ID);
const CHAT_CONTRACT_IDENTITY = fixtureIdentity("skiff-protocol-v1", "chat");
const LEGACY_CONTRACT_IDENTITY = fixtureIdentity("skiff-protocol-v1", "legacy");
const MISMATCH_CONTRACT_IDENTITY = fixtureIdentity(
  "skiff-protocol-v1",
  "mismatch",
);
const CONTRACT_FILE_IR_IDENTITY = fixtureIdentity(
  "skiff-file-ir-v3",
  "contract-file",
);
const CONTRACT_FILE_IR_UNIT = fileIrUnitTypes(CONTRACT_FILE_IR_IDENTITY);
const CONTRACT_FILE_IR_HASH = artifactHash(CONTRACT_FILE_IR_UNIT);
const CONTRACT_FILE_IR_PATH = `units/files/${CONTRACT_FILE_IR_HASH}.json`;
const DEFAULTED_RUNTIME_PROGRAM_BUILD_ID =
  "skiff-service-build-v1:sha256:9e0b19e915403565ffb7963ed393684bd7d88c8b30fd074c5e1f7cf03713d6f1";
const ASSEMBLY_HASH = artifactHash(
  serviceAssemblyHashInput(
    serviceAssembly(SERVICE_ID, { assemblyIdentity: "" }),
  ),
);
const OTHER_ASSEMBLY_HASH =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ASSEMBLY_IDENTITY = `skiff-service-assembly-v1:sha256:${ASSEMBLY_HASH}`;
const OTHER_ASSEMBLY_IDENTITY = `skiff-service-assembly-v1:sha256:${OTHER_ASSEMBLY_HASH}`;
let artifactIdentityCliPromise: Promise<string> | undefined;

beforeAll(async () => {
  if (
    process.env.SKIFF_ARTIFACT_IDENTITY_CLI === undefined ||
    process.env.SKIFF_ARTIFACT_IDENTITY_CLI.trim().length === 0
  ) {
    process.env.SKIFF_ARTIFACT_IDENTITY_CLI = await ensureArtifactIdentityCli();
  }
}, 60_000);

afterEach(async () => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) {
      await rm(dir, { recursive: true, force: true });
    }
  }
});

afterAll(() => {
  if (originalIdentityCliEnv === undefined) {
    delete process.env.SKIFF_ARTIFACT_IDENTITY_CLI;
    return;
  }
  process.env.SKIFF_ARTIFACT_IDENTITY_CLI = originalIdentityCliEnv;
});

async function createArtifactRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "skiff-router-artifacts-"));
  tempDirs.push(root);
  return root;
}

function loadManifest(value: unknown) {
  addDefaultOperationAbiIds(value);
  return loadRuntimeManifest(value);
}

async function ensureArtifactIdentityCli(): Promise<string> {
  artifactIdentityCliPromise ??= buildArtifactIdentityCli();
  return await artifactIdentityCliPromise;
}

async function buildArtifactIdentityCli(): Promise<string> {
  const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
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
  args: string[],
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

function runIdentityCli(
  cliPath: string,
  args: string[],
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

function addDefaultOperationAbiIds(value: unknown): void {
  if (!isRecord(value) || !Array.isArray(value.operations)) {
    return;
  }
  value.operations.forEach((operation, index) => {
    if (!isRecord(operation) || typeof operation.operationAbiId === "string") {
      return;
    }
    const target =
      typeof operation.target === "string"
        ? operation.target
        : typeof operation.entrypoint === "string"
          ? operation.entrypoint
          : typeof operation.operation === "string"
            ? operation.operation
            : `index:${index}`;
    operation.operationAbiId = testOperationAbiId(target);
  });
}

async function writeServiceConfigSource(
  root: string,
  serviceId: string,
  fileName: string,
  text: string,
): Promise<void> {
  const path = join(
    root,
    "configs",
    "services",
    ...serviceIdPathSegments(serviceId),
    fileName,
  );
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, text);
}

describe("router artifact root", () => {
  it("uses canonical JSON ordering for stableStringify with unsorted and undefined fields", () => {
    const value = {
      z: 1,
      c: [
        { z: 2, b: 2, a: 1, c: undefined },
        { y: 1, x: 2 },
      ],
      d: undefined,
      a: { z: 9, b: 2, c: 3, a: undefined, k: 1 },
    };

    expect(stableStringify(value)).toBe(
      '{"a":{"b":2,"c":3,"k":1,"z":9},"c":[{"a":1,"b":2,"z":2},{"x":2,"y":1}],"z":1}',
    );
    expect(
      createHash("sha256").update(stableStringify(value)).digest("hex"),
    ).toBe("448a5f50df6ed68de4ebbee58d8cbac101dc39b6481f9618d1dc0d311967fada");
  }, 120_000);

  it("rejects legacy index-only artifact roots", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "index"));
    await mkdir(join(root, "manifests"));
    await writeFile(
      join(root, "manifests", "chat.json"),
      JSON.stringify(routerManifest("example.com/chat"), null, 2),
    );
    await writeFile(
      join(root, "index", `chat-${identityHash(CHAT_CONTRACT_IDENTITY)}.json`),
      JSON.stringify(
        {
          schemaVersion: "skiff-artifact-index-v1",
          serviceId: "example.com/chat",
          contractIdentity: CHAT_CONTRACT_IDENTITY,
          routerManifest: "manifests/chat.json",
          generation: "generation-chat",
          fingerprint: "fingerprint-chat",
        },
        null,
        2,
      ),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /artifact versions dir .* must be a directory/,
    );
  });

  it("prefers service assembly over legacy router manifest pointers", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await mkdir(join(root, "manifests"));
    await writeFile(
      join(root, "manifests", "legacy.json"),
      JSON.stringify(routerManifest("example.com/legacy"), null, 2),
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      routerManifest: "manifests/legacy.json",
    });
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID),
    );
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
    expect(
      loaded.manifest.operations.some(
        (operation) => operation.operation === "Ping.ping",
      ),
    ).toBe(false);
  }, 120_000);

  it("can derive router manifest data from an indexed service assembly", async () => {
    const root = await createArtifactRoot();
    const generated = await writeCompilerGeneratedWebSocketFixtureArtifactRoot(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.id).toBe("example.com/websocket_fixture");
    expect(loaded.manifest.websocketEntry).toMatchObject({
      serviceId: "example.com/websocket_fixture",
    });
    expect(
      loaded.manifest.websocketEntry?.path === undefined ||
        loaded.manifest.websocketEntry.path === "/ws",
    ).toBe(true);
    const receiveMessageParameter =
      loaded.manifest.websocketEntry?.receive?.operationManifest.parameters.find(
        (parameter) => parameter.name === "message",
      );
    expect(receiveMessageParameter?.schema).toMatchObject({
      oneOf: [
        {
          properties: {
            tag: { enum: ["text"] },
            text: { type: "string" },
          },
        },
        {
          properties: {
            tag: { enum: ["binary"] },
            base64: { type: "string" },
          },
        },
      ],
    });
    const connectRequestParameter =
      loaded.manifest.websocketEntry?.connect?.operationManifest.parameters[0];
    expect(connectRequestParameter?.name).toBe("request");
    expect(connectRequestParameter?.schema).toMatchObject({
      type: "object",
      properties: {
        connectionId: { type: "string" },
        url: { type: "string" },
        query: { type: "array" },
        headers: { type: "array" },
        cookies: { type: "array" },
        version: { type: "string", nullable: true },
      },
      xSkiffSymbol: "std.websocket.WebSocketConnectRequest",
    });
    const connectRequestSchema = connectRequestParameter?.schema as
      | { required?: string[] }
      | undefined;
    expect(connectRequestSchema?.required).toEqual(
      expect.arrayContaining([
        "connectionId",
        "cookies",
        "headers",
        "query",
        "url",
      ]),
    );
    expect(
      loaded.manifest.websocketEntry?.connect?.operationManifest.target,
    ).toMatch(/^entry\.[a-z0-9_~]+\.websocket\.connect$/);
    expect(
      loaded.manifest.websocketEntry?.receive.operationManifest.target,
    ).toMatch(/^entry\.[a-z0-9_~]+\.websocket\.receive$/);
    expect(loaded.manifest.operations).toHaveLength(2);
    expect(loaded.manifest.rawHttpEntries).toHaveLength(0);
    expect(loaded.control.artifactRoots).toEqual([root]);
    expect(loaded.control.fingerprint).toBe(
      generated.serviceAssembly.assemblyIdentity,
    );
  }, 120_000);

  it("projects service assembly access metadata into the loaded manifest", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.access = {
      visibility: "internal",
      organizationRole: "owner",
    };
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.access).toEqual({
      visibility: "internal",
      organizationRole: "owner",
    });
  }, 120_000);

  it("loads dev reload services from ordered multiple artifact roots", async () => {
    const primaryRoot = await createArtifactRoot();
    const overlayRoot = await createArtifactRoot();
    const overlayServiceId = "example.com/chat";

    const primaryAssembly = await writeServiceAssembly(
      primaryRoot,
      serviceAssembly(SERVICE_ID),
      SERVICE_ID,
    );
    await writeDevReloadPointer(primaryRoot, primaryAssembly, {
      serviceId: SERVICE_ID,
    });
    await writeContractFile(primaryRoot);

    const overlayAssembly = await writeServiceAssembly(
      overlayRoot,
      serviceAssembly(overlayServiceId),
      overlayServiceId,
    );
    await writeDevReloadPointer(overlayRoot, overlayAssembly, {
      serviceId: overlayServiceId,
    });
    await writeContractFile(overlayRoot);

    const loaded = await loadRouterArtifactRoot([primaryRoot, overlayRoot], {
      devReload: true,
    });

    expect(loaded.control.artifactRoots).toEqual([primaryRoot, overlayRoot]);
    expect(loaded.manifest.service.id).toBe("__multi__");
    expect(
      loaded.manifest.rawHttpEntries.map((entry) => entry.serviceId),
    ).toEqual([SERVICE_ID, overlayServiceId]);
  }, 120_000);

  it("lets later artifact roots override duplicate dev reload service ids", async () => {
    const primaryRoot = await createArtifactRoot();
    const overlayRoot = await createArtifactRoot();
    const serviceStorageSegment = publicationStorageSegment(SERVICE_ID);
    const primaryEntrypoint = `runtime.${serviceStorageSegment}.WebSocketFixtureHttpApi.primaryHandle`;
    const overlayEntrypoint = `runtime.${serviceStorageSegment}.WebSocketFixtureHttpApi.overlayHandle`;

    const primaryAssemblyValue = serviceAssembly(SERVICE_ID) as any;
    primaryAssemblyValue.operations.find(
      (operation: any) => operation.operation === "WebSocketFixtureHttpApi.handle",
    )!.entrypoint = primaryEntrypoint;
    const primaryAssembly = await writeServiceAssembly(
      primaryRoot,
      primaryAssemblyValue,
    );
    await writeDevReloadPointer(primaryRoot, primaryAssembly);
    await writeContractFile(primaryRoot);

    const overlayAssemblyValue = serviceAssembly(SERVICE_ID) as any;
    overlayAssemblyValue.operations.find(
      (operation: any) => operation.operation === "WebSocketFixtureHttpApi.handle",
    )!.entrypoint = overlayEntrypoint;
    const overlayAssembly = await writeServiceAssembly(
      overlayRoot,
      overlayAssemblyValue,
    );
    await writeDevReloadPointer(overlayRoot, overlayAssembly);
    await writeContractFile(overlayRoot);

    const loaded = await loadRouterArtifactRoot([primaryRoot, overlayRoot], {
      devReload: true,
    });

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
    expect(loaded.manifest.rawHttpEntries).toHaveLength(1);
    expect(loaded.manifest.rawHttpEntries[0]?.operationManifest.target).toBe(
      overlayEntrypoint,
    );
  }, 120_000);

  it("loads release pointers and config for URL-like service ids through full path mapping", async () => {
    const root = await createArtifactRoot();
    const serviceId = "skiff.run/account";
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const assemblyValue = serviceAssembly(serviceId) as any;
    assemblyValue.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [{ path: "region", type: "string", required: true }],
    };
    assemblyValue.configUses = ["region"];
    await writeServiceConfigSource(
      root,
      serviceId,
      "config.prod.yml",
      ["service:", "  region: iad"].join("\n"),
    );
    const assembly = await writeServiceAssembly(root, assemblyValue, serviceId);
    const pointer = {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId,
      contractIdentity: contractIdentityForService(serviceId),
      serviceAssembly: {
        assemblyIdentity: assembly.identity,
        assemblyPath: assembly.path,
      },
    };
    const buildId = fixtureIdentity(
      "skiff-service-build-v1",
      stableStringify(pointer),
    );
    await writeIndexPointer(root, pointer);
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root, {
      configProfile: "prod",
    });

    expect(loaded.manifest.service.id).toBe(serviceId);
    expect(loaded.control.serviceConfig?.[0]?.resolvedConfig).toEqual({
      region: "iad",
    });
    expect(
      await readFile(
        join(
          root,
          "versions",
          "services",
          "skiff~run~~account",
          `${serviceTestVersion(serviceId)}.json`,
        ),
        "utf8",
      ),
    ).toContain(`"serviceId": "${serviceId}"`);
    expect(
      await readFile(
        join(
          root,
          "configs",
          "services",
          "skiff~run~~account",
          "config.prod.yml",
        ),
        "utf8",
      ),
    ).toContain("region: iad");
    expect(
      await readFile(
        join(
          root,
          "builds",
          "services",
          "skiff~run~~account",
          `${identityHash(buildId)}.json`,
        ),
        "utf8",
      ),
    ).toContain(`"serviceId": "${serviceId}"`);
  });

  it("rejects URL-like service version pointers whose path service id mismatches the payload", async () => {
    const root = await createArtifactRoot();
    const buildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await mkdir(join(root, "versions", "services", "skiff~run~~wrong"), {
      recursive: true,
    });
    await writeFile(
      join(
        root,
        "versions",
        "services",
        "skiff~run~~wrong",
        "account-test.json",
      ),
      JSON.stringify(
        {
          schemaVersion: "skiff-service-version-pointer-v1",
          serviceId: "skiff.run/account",
          version: "account-test",
          buildId,
          updatedAt: "2026-05-05T00:00:00.000Z",
          updatedBy: "test",
        },
        null,
        2,
      ),
    );

    await expect(
      readActiveArtifactPointers(root, { releaseMode: true }),
    ).rejects.toThrow(/serviceId must match versions\/services path/);
  });

  it("uses typed serviceUnit entrypoints from the new route schema", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const typedTarget = testServiceRouteTarget(
      SERVICE_ID,
      "WebSocketFixtureHttpApi.handle",
    );
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        entrypoints: {
          "WebSocketFixtureHttpApi.handle": typedTarget,
        },
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const operation = loaded.manifest.operations.find(
      (candidate) => candidate.operation === "WebSocketFixtureHttpApi.handle",
    );

    expect(operation?.target).toBe(typedTarget);
    expect(loaded.manifest.rawHttpEntries[0]?.operationManifest.target).toBe(
      typedTarget,
    );
    expect(loaded.manifest.rawHttpEntries[0]?.buildId).toBe(
      loaded.versionByService
        ?.get(SERVICE_ID)
        ?.get(serviceTestVersion(SERVICE_ID))?.buildId,
    );
  });

  it("projects compiler HTTP route manifests from service assembly gateway metadata", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.gateway.http.routes = [
      {
        method: "POST",
        path: "/session",
        operation: "WebSocketFixtureHttpApi.handle",
        target: `gateway.${publicationStorageSegment(SERVICE_ID)}.http.session`,
        adapter: {
          kind: "rawHttp",
          handler: {
            kind: "serviceFunction",
            modulePath: "internal.websocket_fixture_service",
            symbol: "handle",
          },
          adapterArgs: [{ param: "request", source: { kind: "http.request" } }],
        },
      },
    ];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const route = loaded.manifest.httpRouteEntries[0];

    expect(route).toMatchObject({
      serviceId: SERVICE_ID,
      method: "POST",
      path: "/session",
      operation: "WebSocketFixtureHttpApi.handle",
      gatewayTarget: `gateway.${publicationStorageSegment(SERVICE_ID)}.http.session`,
      operationManifest: {
        target: testServiceRouteTarget(SERVICE_ID, "WebSocketFixtureHttpApi.handle"),
      },
    });
    expect(route?.buildId).toBe(
      loaded.versionByService
        ?.get(SERVICE_ID)
        ?.get(serviceTestVersion(SERVICE_ID))?.buildId,
    );
  });

  it("projects package HTTP route metadata without requiring a Service Unit operation", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const packageTarget = packageHttpHandlerTarget(
      "skiff.run/http-session",
      "issue",
    );
    const packageOperationAbiId = testOperationAbiId(packageTarget);
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.gateway.http.routes = [
      {
        method: "POST",
        path: "/session",
        operation: "http.route.httpSession.issue",
        operationAbiId: packageOperationAbiId,
        target: packageTarget,
        handler: {
          kind: "packageFunction",
          packageId: "skiff.run/http-session",
          alias: "httpSession",
          symbolPath: "issue",
        },
        adapter: {
          kind: "rawHttp",
          handler: {
            kind: "packageFunction",
            packageId: "skiff.run/http-session",
            symbolPath: "issue",
          },
          adapterArgs: [
            { param: "request", source: { kind: "http.request" } },
          ],
        },
      },
    ];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const route = loaded.manifest.httpRouteEntries.find(
      (candidate) => candidate.path === "/session",
    );

    expect(route).toMatchObject({
      serviceId: SERVICE_ID,
      method: "POST",
      path: "/session",
      operationAbiId: packageOperationAbiId,
      dispatchTarget: packageTarget,
      handler: {
        kind: "packageFunction",
        packageId: "skiff.run/http-session",
        alias: "httpSession",
        symbolPath: "issue",
      },
    });
    expect(route?.operation).toBeUndefined();
    expect(route?.operationManifest).toBeUndefined();
  });

  it("rejects serviceAssembly operations without entrypoint", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.operations[0].entrypoint;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.operation\.entrypoint must be a non-empty string/,
    );
  });

  it("rejects legacy serviceUnit routeTarget fields", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
      },
    );
    const unitPath = join(root, serviceUnit.unitPath);
    const unit = JSON.parse(await readFile(unitPath, "utf8"));
    unit.operations[0].routeTarget = "legacy.route.target";
    await writeFile(unitPath, JSON.stringify(unit, null, 2));
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schema_invalid: serviceUnit is invalid: unknown field `routeTarget`/,
    );
  });

  it("rejects serviceAssembly operation ABI id that disagrees with serviceUnit operation", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
      },
    );
    const connectOperation = (assembly.operations as Array<any>).find(
      (operation: any) => operation.operation === "WebSocketFixtureConnection.connect",
    )!;
    connectOperation.operationAbiId = "operation:test:mismatch";
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.operation\.operationAbiId for WebSocketFixtureConnection\.connect must match typed serviceUnit operationAbiId/,
    );
  });

  it("loads Service Unit package dependencies through Package Unit indexes without service assembly packages", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const packageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-build",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      packageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.packages;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
    expect(loaded.control.fingerprint).toBe(writtenAssembly.identity);
  });

  it("resolves Service Unit package dependencies through exact Package Unit indexes", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const firstPackageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-build-a",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      firstPackageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
            config: {
              model: "qwen-plus",
            },
          },
        ],
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            abiIdentity:
              "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            usedSymbols: [
              {
                kind: "function",
                symbolPath: "chat",
              },
            ],
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    const firstLoaded = await loadRouterArtifactRoot(root);
    const firstBinding = firstLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID));

    const secondPackageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-build-b",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      secondPackageUnit,
      "2",
    );
    const secondLoaded = await loadRouterArtifactRoot(root);
    const secondBinding = secondLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID));

    expect(firstBinding?.buildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBinding?.buildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBinding?.buildId).not.toBe(firstBinding?.buildId);
    expect(secondLoaded.manifest.rawHttpEntries[0]?.buildId).toBe(
      secondBinding?.buildId,
    );
  });

  it("includes Service Unit package ABI expectations in the dynamic build id", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const packageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-build",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      packageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeContractFile(root);

    const firstServiceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
          },
        ],
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            abiIdentity:
              "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            usedSymbols: [
              {
                kind: "function",
                symbolPath: "chat",
              },
            ],
          },
        ],
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit: firstServiceUnit,
    });

    const firstLoaded = await loadRouterArtifactRoot(root);
    const firstBuildId = firstLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID))?.buildId;

    const secondServiceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
          },
        ],
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            abiIdentity:
              "skiff-package-abi-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            usedSymbols: [
              {
                kind: "function",
                symbolPath: "chat",
              },
            ],
          },
        ],
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit: secondServiceUnit,
    });

    const secondLoaded = await loadRouterArtifactRoot(root);
    const secondBuildId = secondLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID))?.buildId;

    expect(firstBuildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBuildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBuildId).not.toBe(firstBuildId);
  });

  it("preserves Service Unit package ABI expectation ordering in the dynamic build id", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const llmPackageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-build",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      llmPackageUnit,
      "1",
    );
    const dbPackageUnit = await writePackageUnit(
      root,
      "skiff.run/db",
      "1.0.0",
      "db-build",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/db",
      "1.0.0",
      dbPackageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeContractFile(root);
    const baseOptions = {
      assembly,
      packageDependencies: [
        {
          id: "skiff.run/llm",
          version: "1.0.0",
          alias: "llm",
        },
        {
          id: "skiff.run/db",
          version: "1.0.0",
          alias: "db",
        },
      ],
    };
    const llmExpectation = {
      id: "skiff.run/llm",
      version: "1.0.0",
      abiIdentity:
        "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      usedSymbols: [
        {
          kind: "function",
          symbolPath: "chat",
        },
      ],
    };
    const dbExpectation = {
      id: "skiff.run/db",
      version: "1.0.0",
      abiIdentity:
        "skiff-package-abi-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      usedSymbols: [
        {
          kind: "const",
          symbolPath: "mongo",
        },
      ],
    };

    const firstServiceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        ...baseOptions,
        packageAbiExpectations: [llmExpectation, dbExpectation],
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit: firstServiceUnit,
    });

    const firstLoaded = await loadRouterArtifactRoot(root);
    const firstBuildId = firstLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID))?.buildId;

    const secondServiceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        ...baseOptions,
        packageAbiExpectations: [dbExpectation, llmExpectation],
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit: secondServiceUnit,
    });

    const secondLoaded = await loadRouterArtifactRoot(root);
    const secondBuildId = secondLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID))?.buildId;

    expect(firstBuildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBuildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBuildId).not.toBe(firstBuildId);
  });

  it("preserves Service Unit package ABI expectation used symbol ordering in the dynamic build id", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const packageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-build",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      packageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeContractFile(root);
    const baseOptions = {
      assembly,
      packageDependencies: [
        {
          id: "skiff.run/llm",
          version: "1.0.0",
          alias: "llm",
        },
      ],
    };

    const firstServiceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        ...baseOptions,
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            abiIdentity:
              "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            usedSymbols: [
              {
                kind: "const",
                symbolPath: "llm",
              },
              {
                kind: "function",
                symbolPath: "chat",
              },
            ],
          },
        ],
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit: firstServiceUnit,
    });

    const firstLoaded = await loadRouterArtifactRoot(root);
    const firstBuildId = firstLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID))?.buildId;

    const secondServiceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        ...baseOptions,
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            abiIdentity:
              "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            usedSymbols: [
              {
                kind: "function",
                symbolPath: "chat",
              },
              {
                kind: "const",
                symbolPath: "llm",
              },
            ],
          },
        ],
      },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit: secondServiceUnit,
    });

    const secondLoaded = await loadRouterArtifactRoot(root);
    const secondBuildId = secondLoaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceTestVersion(SERVICE_ID))?.buildId;

    expect(firstBuildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBuildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(secondBuildId).not.toBe(firstBuildId);
  });

  it("does not resolve caret package dependency versions as ranges", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const packageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.2.0",
      "llm-build-range",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.2.0",
      packageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "^1",
            alias: "llm",
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /path_escape: package version \^1 is not a safe artifact path segment/,
    );
  });

  it("rejects legacy Service Unit package dependency fields", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            packageId: "skiff.run/llm",
            versionConstraint: "1.0.0",
            dependencyRef: "llm",
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schema_invalid: serviceUnit is invalid: unknown field `(packageId|versionConstraint|dependencyRef)`/,
    );
  });

  it("rejects legacy Service Unit package symbol usage", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
      },
    );
    const unitPath = join(root, serviceUnit.unitPath);
    const unit = JSON.parse(await readFile(unitPath, "utf8"));
    unit.packageSymbolUsage = [
      {
        id: "skiff.run/llm",
        version: "1.0.0",
        usedSymbols: [
          {
            kind: "function",
            symbolPath: "chat",
          },
        ],
      },
    ];
    await writeFile(unitPath, JSON.stringify(unit, null, 2));
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schema_invalid: serviceUnit is invalid: unknown field `packageSymbolUsage`/,
    );
  });

  it("rejects legacy ABI identity gates in Service Unit package ABI expectations", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            requiredAbiIdentity:
              "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            usedSymbols: [
              {
                kind: "function",
                symbolPath: "chat",
              },
            ],
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schema_invalid: serviceUnit is invalid: unknown field `requiredAbiIdentity`/,
    );
  });

  it("rejects legacy symbol keys in Service Unit package ABI expectations", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageAbiExpectations: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            abiIdentity:
              "skiff-package-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            usedSymbols: [
              {
                kind: "function",
                symbol: "chat",
              },
            ],
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schema_invalid: serviceUnit is invalid: unknown field `symbol`/,
    );
  });

  it("ignores legacy service assembly package locks when Service Unit has no package dependencies", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.packages = [
      {
        id: "skiff.run/missing-legacy-lock",
        version: "1.0.0",
        alias: "legacy",
        assemblyIdentity: fixtureIdentity(
          "skiff-package-assembly-v1",
          "missing-legacy-lock",
        ),
      },
    ];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
  });

  it("rejects Service Unit package dependencies when the Package Unit index is missing", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /artifact_not_found: artifact .*indexes\/packages\/skiff~run~~llm\/versions\/1\.0\.0\.json was not found/,
    );
  });

  it("rejects Package Unit indexes whose package id disagrees with the Service Unit dependency", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const packageUnit = await writePackageUnit(
      root,
      "example.com/other",
      "1.0.0",
      "other-build",
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      packageUnit,
      "1",
    );
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
          },
        ],
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /packageId example\.com\/other does not match dependency skiff\.run\/llm/,
    );
  });

  it("loads service version pointers through immutable build records", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    const writtenAssembly = await writeServiceAssembly(
      root,
      serviceAssembly(SERVICE_ID),
    );
    const buildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const version = "websocket_fixture-ios-1.3.7";
    await writeVersionPointer(root, {
      buildId,
      serviceId: SERVICE_ID,
      version,
    });
    await writeBuildRecord(root, {
      buildId,
      serviceId: SERVICE_ID,
      assembly: writtenAssembly,
      serviceVersion: version,
    });

    const loaded = await loadRouterArtifactRoot(root, { releaseMode: true });

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
    expect(loaded.control).toMatchObject({
      artifactRoots: [root],
      mode: "release",
      fingerprint: buildId,
    });
    const binding = loaded.versionByService?.get(SERVICE_ID)?.get(version);
    expect(binding).toMatchObject({
      pointerBuildId: buildId,
      serviceId: SERVICE_ID,
      version,
    });
    expect(binding?.buildId).toMatch(
      /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(binding?.buildId).not.toBe(buildId);
    expect(loaded.manifest.rawHttpEntries[0]?.buildId).toBe(binding?.buildId);
  });

  it("hashes defaulted ServiceUnit fields with the runtime canonical build vector", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await mkdir(join(root, "units", "services"), { recursive: true });
    await writeContractFile(root);
    const assembly = runtimeProgramServiceAssembly(SERVICE_ID);
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    const serviceVersion = "websocket_fixture-ios-defaults";
    const serviceUnitPath = "units/services/websocket_fixture-defaults.json";
    const fileIrIdentity =
      "skiff-file-ir-v3:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const filePath = "units/files/websocket_fixture-main.json";
    await writeFile(
      join(root, serviceUnitPath),
      JSON.stringify(
        {
          schemaVersion: "skiff-service-unit-v1",
          service: {
            id: SERVICE_ID,
          },
          version: serviceVersion,
          protocolIdentity: CONTRACT_IDENTITY,
          publicationAbi: servicePublicationAbiFromAssembly(
            SERVICE_ID,
            serviceVersion,
            assembly,
          ),
          files: [
            {
              fileIrIdentity,
              modulePath: "svc.main",
              artifactPath: filePath,
            },
          ],
          operations: [
            {
              kind: "localExecutable",
              operation: serviceOperationAbiRef(assembly.operations[0]!),
              executable: {
                fileRef: {
                  fileIrIdentity,
                  modulePath: "svc.main",
                  artifactPath: filePath,
                },
                executableIndex: 0,
                callableAbiId: "callable:svc.main.Api.hello",
                callableKind: "publicFunction",
              },
            },
          ],
          gateway: {},
          config: {},
        },
        null,
        2,
      ),
    );
    const pointerBuildId = fixtureIdentity(
      "skiff-service-build-v1",
      "defaulted-service-unit",
    );
    await writeVersionPointer(root, {
      buildId: pointerBuildId,
      serviceId: SERVICE_ID,
      version: serviceVersion,
    });
    await writeBuildRecordWithServiceUnitPath(root, {
      buildId: pointerBuildId,
      serviceId: SERVICE_ID,
      assembly: writtenAssembly,
      serviceVersion,
      serviceUnitPath,
    });

    const loaded = await loadRouterArtifactRoot(root, { releaseMode: true });

    const binding = loaded.versionByService
      ?.get(SERVICE_ID)
      ?.get(serviceVersion);
    expect(binding?.buildId).toBe(DEFAULTED_RUNTIME_PROGRAM_BUILD_ID);
    expect(loaded.manifest.operations[0]?.target).toBe("svc.main.Api.hello");
  });

  it("rejects service version pointers whose buildId only has a sha256 suffix", async () => {
    const root = await createArtifactRoot();
    await writeVersionPointer(root, {
      buildId:
        "legacy-build:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      serviceId: SERVICE_ID,
      version: "websocket_fixture-ios-1.3.7",
    });

    await expect(
      loadRouterArtifactRoot(root, { releaseMode: true }),
    ).rejects.toThrow(
      /buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>/,
    );
  });

  it("rejects build records whose buildId only has a sha256 suffix", async () => {
    const root = await createArtifactRoot();
    const versionBuildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const buildRecordBuildId =
      "legacy-build:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await writeVersionPointer(root, {
      buildId: versionBuildId,
      serviceId: SERVICE_ID,
      version: "websocket_fixture-ios-1.3.7",
    });
    await mkdir(
      join(root, "builds", "services", ...serviceIdPathSegments(SERVICE_ID)),
      { recursive: true },
    );
    await writeFile(
      join(
        root,
        "builds",
        "services",
        ...serviceIdPathSegments(SERVICE_ID),
        `${identityHash(versionBuildId)}.json`,
      ),
      JSON.stringify(
        {
          schemaVersion: "skiff-service-build-v1",
          serviceId: SERVICE_ID,
          serviceVersion: "websocket_fixture-ios-1.3.7",
          buildId: buildRecordBuildId,
          serviceAssembly: {
            assemblyIdentity: ASSEMBLY_IDENTITY,
            assemblyPath: serviceAssemblyArtifactPath(
              SERVICE_ID,
              ASSEMBLY_HASH,
            ),
          },
        },
        null,
        2,
      ),
    );

    await expect(
      loadRouterArtifactRoot(root, { releaseMode: true }),
    ).rejects.toThrow(
      /buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>/,
    );
  });

  it("rejects legacy contract identity aliases in build records", async () => {
    const root = await createArtifactRoot();
    const buildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await writeVersionPointer(root, {
      buildId,
      serviceId: SERVICE_ID,
      version: "websocket_fixture-ios-1.3.7",
    });
    await mkdir(
      join(root, "builds", "services", ...serviceIdPathSegments(SERVICE_ID)),
      { recursive: true },
    );
    await writeFile(
      join(
        root,
        "builds",
        "services",
        ...serviceIdPathSegments(SERVICE_ID),
        `${identityHash(buildId)}.json`,
      ),
      JSON.stringify(
        {
          schemaVersion: "skiff-service-build-v1",
          serviceId: SERVICE_ID,
          serviceVersion: "websocket_fixture-ios-1.3.7",
          buildId,
          protocolIdentity: CONTRACT_IDENTITY,
          serviceAssembly: {
            assemblyIdentity: ASSEMBLY_IDENTITY,
            assemblyPath: serviceAssemblyArtifactPath(
              SERVICE_ID,
              ASSEMBLY_HASH,
            ),
          },
        },
        null,
        2,
      ),
    );

    await expect(
      loadRouterArtifactRoot(root, { releaseMode: true }),
    ).rejects.toThrow(
      /protocolIdentity is not supported; use contractIdentity/,
    );
  });

  it("rejects legacy service protocol identity aliases in build records", async () => {
    const root = await createArtifactRoot();
    const buildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await writeVersionPointer(root, {
      buildId,
      serviceId: SERVICE_ID,
      version: "websocket_fixture-ios-1.3.7",
    });
    await mkdir(
      join(root, "builds", "services", ...serviceIdPathSegments(SERVICE_ID)),
      { recursive: true },
    );
    await writeFile(
      join(
        root,
        "builds",
        "services",
        ...serviceIdPathSegments(SERVICE_ID),
        `${identityHash(buildId)}.json`,
      ),
      JSON.stringify(
        {
          schemaVersion: "skiff-service-build-v1",
          serviceId: SERVICE_ID,
          serviceVersion: "websocket_fixture-ios-1.3.7",
          buildId,
          serviceProtocolIdentity: CONTRACT_IDENTITY,
          serviceAssembly: {
            assemblyIdentity: ASSEMBLY_IDENTITY,
            assemblyPath: serviceAssemblyArtifactPath(
              SERVICE_ID,
              ASSEMBLY_HASH,
            ),
          },
        },
        null,
        2,
      ),
    );

    await expect(
      loadRouterArtifactRoot(root, { releaseMode: true }),
    ).rejects.toThrow(
      /serviceProtocolIdentity is not supported; use contractIdentity/,
    );
  });

  it("rejects build records whose serviceVersion mismatches the service version pointer", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    const writtenAssembly = await writeServiceAssembly(
      root,
      serviceAssembly(SERVICE_ID),
    );
    const buildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await writeVersionPointer(root, {
      buildId,
      serviceId: SERVICE_ID,
      version: "websocket_fixture-ios-1.3.7",
    });
    await writeBuildRecord(root, {
      buildId,
      serviceId: SERVICE_ID,
      assembly: writtenAssembly,
      serviceVersion: "websocket_fixture-ios-1.3.8",
    });

    await expect(
      readActiveArtifactPointers(root, { releaseMode: true }),
    ).rejects.toThrow(
      /serviceVersion must match service version pointer version/,
    );
  });

  it("rejects build records missing serviceVersion", async () => {
    const root = await createArtifactRoot();
    const buildId =
      "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    await writeVersionPointer(root, {
      buildId,
      serviceId: SERVICE_ID,
      version: "websocket_fixture-ios-1.3.7",
    });
    await mkdir(
      join(root, "builds", "services", ...serviceIdPathSegments(SERVICE_ID)),
      { recursive: true },
    );
    await writeFile(
      join(
        root,
        "builds",
        "services",
        ...serviceIdPathSegments(SERVICE_ID),
        `${identityHash(buildId)}.json`,
      ),
      JSON.stringify(
        {
          schemaVersion: "skiff-service-build-v1",
          serviceId: SERVICE_ID,
          buildId,
          serviceAssembly: {
            assemblyIdentity: ASSEMBLY_IDENTITY,
            assemblyPath: serviceAssemblyArtifactPath(
              SERVICE_ID,
              ASSEMBLY_HASH,
            ),
          },
        },
        null,
        2,
      ),
    );

    await expect(
      readActiveArtifactPointers(root, { releaseMode: true }),
    ).rejects.toThrow(/serviceVersion must be a non-empty string/);
  });

  it("loads dev reload pointers without reading stale legacy index files", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "index"), { recursive: true });
    await writeFile(join(root, "index", "stale.json"), "{not-json");
    const generated =
      await writeCompilerGeneratedWebSocketFixtureDevReloadArtifactRoot(root, "prod");

    const loaded = await loadRouterArtifactRoot(root, {
      devReload: true,
      configProfile: "prod",
    });

    expect(loaded.manifest.service.id).toBe(generated.serviceId);
    expect(loaded.control.devReload).toBe(true);
    expect(loaded.control.fingerprint).toBe(
      generated.serviceAssembly.assemblyIdentity,
    );
  });

  it("loads dev reload pointers for URL-like service ids through full path mapping", async () => {
    const root = await createArtifactRoot();
    const serviceId = "skiff.run/account";
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    const assembly = await writeServiceAssembly(
      root,
      serviceAssembly(serviceId),
      serviceId,
    );
    await writeDevReloadPointer(root, assembly, { profile: "prod", serviceId });

    const loaded = await loadRouterArtifactRoot(root, {
      devReload: true,
      configProfile: "prod",
    });

    expect(loaded.manifest.service.id).toBe(serviceId);
    expect(loaded.control.mode).toBe("dev");
    const serviceVersion = serviceTestVersion(serviceId);
    const serviceBuild = loaded.control.serviceBuilds?.find(
      (build) => build.serviceId === serviceId,
    );
    expect(serviceBuild).toMatchObject({
      serviceId,
      version: serviceVersion,
    });
    const binding = loaded.versionByService?.get(serviceId)?.get(serviceVersion);
    expect(binding).toMatchObject({
      serviceId,
      version: serviceVersion,
      buildId: serviceBuild?.buildId,
      pointerBuildId: serviceBuild?.pointerBuildId,
    });
    expect(
      await readFile(
        join(root, "dev", "services", "skiff~run~~account.json"),
        "utf8",
      ),
    ).toContain(`"serviceId": "${serviceId}"`);
  });

  it("rejects dev reload pointers missing buildId", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const writtenAssembly = await writeServiceAssembly(
      root,
      serviceAssembly(SERVICE_ID),
    );
    await writeDevReloadPointer(root, writtenAssembly, {
      omitBuildId: true,
    });

    await expect(
      loadRouterArtifactRoot(root, { devReload: true }),
    ).rejects.toThrow(
      /dev\/services\/example~com~~websocket_fixture\.json buildId must be a non-empty string/,
    );
  });

  it("rejects dev reload pointers whose contractHash mismatches protocolIdentity", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const writtenAssembly = await writeServiceAssembly(
      root,
      serviceAssembly(SERVICE_ID),
    );
    await writeDevReloadPointer(root, writtenAssembly, {
      contractHash: `sha256:${OTHER_ASSEMBLY_HASH}`,
    });

    await expect(
      loadRouterArtifactRoot(root, { devReload: true }),
    ).rejects.toThrow(/contractHash .* does not match protocolIdentity hash/);
  });

  it("rejects dev reload serviceAssemblyRef even when serviceAssembly is present", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const writtenAssembly = await writeServiceAssembly(
      root,
      serviceAssembly(SERVICE_ID),
    );
    await writeDevReloadPointer(root, writtenAssembly, {
      serviceAssemblyRef: writtenAssembly.path,
    });

    await expect(
      loadRouterArtifactRoot(root, { devReload: true }),
    ).rejects.toThrow(/serviceAssemblyRef is not supported/);
  });

  it("ignores removed metadata fields in service assembly identities", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.providerRequirements;
    delete assembly.effectSummaries;
    delete assembly.transportSelection;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    assembly.transportSelection = {
      mongo: "provider",
    };
    assembly.providerRequirements = { mongo: [] };
    assembly.effectSummaries = { mongo: [] };
    await writeFile(
      join(root, writtenAssembly.path),
      JSON.stringify(assembly, null, 2),
    );
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
    expect(loaded.control.fingerprint).toBe(writtenAssembly.identity);
  });

  it("accepts object-shaped configShape in service assemblies", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeContractFile(root);

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.preludeIdentity =
      "skiff-prelude-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333";
    assembly.prelude = {
      identity: assembly.preludeIdentity,
      schemaIdentity:
        "skiff-prelude-schema-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444",
      types: ["Json"],
      roots: ["config"],
    };
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscopeModel",
          type: "string",
          required: false,
        },
      ],
    };
    assembly.configUses = ["dashscopeModel"];
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.yml",
      ["service:", "  dashscopeModel: qwen-plus"].join("\n"),
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(loadRouterArtifactRoot(root)).resolves.toMatchObject({
      manifest: {
        service: {
          id: SERVICE_ID,
        },
      },
      control: {
        fingerprint: writtenAssembly.identity,
      },
    });
  });

  it("builds router control service config and activation lookup from artifact config sources", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.yml",
      ["service:", "  dashscopeModel: qwen-plus"].join("\n"),
    );
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.prod.secret.yml",
      ["service:", "  dashscopeApiKey: local-secret"].join("\n"),
    );

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscopeModel",
          type: "string",
          required: true,
        },
        {
          path: "dashscopeApiKey",
          type: "string",
          required: true,
        },
      ],
    };
    assembly.configUses = ["dashscopeModel", "dashscopeApiKey"];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    const loaded = await loadRouterArtifactRoot(root, {
      configProfile: "prod",
    });

    expect(loaded.control.serviceConfig).toHaveLength(1);
    const serviceConfig = loaded.control.serviceConfig![0]!;
    expect(serviceConfig).toMatchObject({
      serviceId: SERVICE_ID,
      buildId: expect.stringMatching(
        /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/,
      ),
      resolvedConfig: {
        dashscopeModel: "qwen-plus",
        dashscopeApiKey: "local-secret",
      },
      redactedResolvedConfig: {
        dashscopeModel: "qwen-plus",
        dashscopeApiKey: "[REDACTED]",
      },
    });
    expect(serviceConfig.configShape).toMatchObject({
      schemaVersion: "skiff-config-shape-v1",
      entries: expect.arrayContaining([
        { path: "dashscopeModel", type: "string", required: true },
        { path: "dashscopeApiKey", type: "string", required: true },
      ]),
    });
    expect(serviceConfig.activationIdentity).toMatch(
      /^skiff-runtime-activation-v1:opaque:[A-Za-z0-9._:-]+$/,
    );
    expect(serviceConfig.resolvedConfigIdentity).toMatch(
      /^skiff-config-resolved-v1:opaque:[A-Za-z0-9._:-]+$/,
    );
    expect(serviceConfig.redactionProjectionIdentity).toMatch(
      /^skiff-config-redaction-v1:sha256:[0-9a-f]{64}$/,
    );
    expect(
      loaded.activationByServiceOperation.get({
        serviceId: SERVICE_ID,
        target: testServiceRouteTarget(SERVICE_ID, "WebSocketFixtureHttpApi.handle"),
        buildId: serviceConfig.buildId,
      }),
    ).toBe(serviceConfig.activationIdentity);
  });

  it("mints a deterministic activation identity that is stable across reloads of an unchanged build", async () => {
    async function loadServiceConfig(model: string) {
      const root = await createArtifactRoot();
      await mkdir(join(root, "assemblies", "services"), { recursive: true });
      await mkdir(join(root, "files"), { recursive: true });
      await writeContractFile(root);
      await writeServiceConfigSource(
        root,
        SERVICE_ID,
        "config.yml",
        ["service:", `  dashscopeModel: ${model}`].join("\n"),
      );
      const assembly = serviceAssembly(SERVICE_ID) as any;
      assembly.configShape = {
        schemaVersion: "skiff-config-shape-v1",
        entries: [{ path: "dashscopeModel", type: "string", required: true }],
      };
      assembly.configUses = ["dashscopeModel"];
      const writtenAssembly = await writeServiceAssembly(root, assembly);
      await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
      const loaded = await loadRouterArtifactRoot(root);
      return loaded.control.serviceConfig![0]!;
    }

    // Two independent reloads of the same unchanged build + config must yield
    // the same activation/resolvedConfig identities. This is what stops the
    // router/runtime from accumulating a fresh activation per reload-artifacts.
    const first = await loadServiceConfig("qwen-plus");
    const second = await loadServiceConfig("qwen-plus");
    expect(second.activationIdentity).toBe(first.activationIdentity);
    expect(second.resolvedConfigIdentity).toBe(first.resolvedConfigIdentity);

    // A genuine config change must produce a different activation identity so
    // it is treated as a distinct activation rather than colliding with the old.
    const changed = await loadServiceConfig("qwen-max");
    expect(changed.activationIdentity).not.toBe(first.activationIdentity);
    expect(changed.resolvedConfigIdentity).not.toBe(first.resolvedConfigIdentity);
  });

  it("builds package scoped control config from package alias namespaces", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.prod.secret.yml",
      [
        "packages:",
        "  llm:",
        "    dashscope:",
        "      apiKey: local-secret",
        "      model: qwen-plus",
      ].join("\n"),
    );

    const packageConfigShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscope.apiKey",
          type: "string",
          required: true,
        },
        {
          path: "dashscope.model",
          type: "string",
          required: true,
        },
      ],
    };
    const packageUnit = await writePackageUnit(
      root,
      "skiff.run/llm",
      "1.0.0",
      "llm-config-build",
      {
        configAndEffectMetadata: {
          config: {
            shape: packageConfigShape,
            activation: {
              schemaVersion: "skiff-config-activation-v1",
              hasPaths: ["dashscope.apiKey"],
            },
          },
        },
      },
    );
    await writePackageUnitIndex(
      root,
      "skiff.run/llm",
      "1.0.0",
      packageUnit,
      "1",
    );

    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.configShape;
    delete assembly.packages;
    assembly.packageConfigs = {
      "skiff.run/llm": {
        config: {
          dashscope: {
            model: "ignored-service-assembly-default",
          },
        },
      },
    };
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      {
        assembly,
        packageDependencies: [
          {
            id: "skiff.run/llm",
            version: "1.0.0",
            alias: "llm",
          },
        ],
        config: {
          packageConfigs: {
            llm: {
              config: {
                dashscope: {
                  model: "ignored-service-unit-default",
                },
              },
            },
          },
        },
      },
    );
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });

    const loaded = await loadRouterArtifactRoot(root, {
      configProfile: "prod",
    });

    expect(loaded.control.serviceConfig).toHaveLength(1);
    const serviceConfig = loaded.control.serviceConfig![0]!;
    expect(serviceConfig.resolvedConfig).toEqual({});
    expect(serviceConfig.packageConfigs).toHaveLength(1);
    expect(serviceConfig.packageConfigs![0]).toMatchObject({
      packageId: "skiff.run/llm",
      alias: "llm",
      resolvedConfig: {
        dashscope: {
          apiKey: "local-secret",
          model: "qwen-plus",
        },
      },
      redactedResolvedConfig: {
        dashscope: {
          apiKey: "[REDACTED]",
          model: "[REDACTED]",
        },
      },
      configShape: packageConfigShape,
    });
  });

  it("does not send service config for services without configShape entries", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.prod.yml",
      ["service:", "  model: gpt-5"].join("\n"),
    );
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.prod.secret.yml",
      ["service:", "  openaiApiKey: sk-local"].join("\n"),
    );

    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.configShape;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    const loaded = await loadRouterArtifactRoot(root, {
      configProfile: "prod",
    });

    expect(loaded.control.serviceConfig).toBeUndefined();
  });

  it("sends serviceDb activation payload even when service has no business config", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);

    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.configShape;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    const loaded = await loadRouterArtifactRoot(root, {
      serviceDb: {
        mongoUrl: "mongodb://127.0.0.1:27017/?directConnection=true",
      },
    });

    expect(loaded.control.serviceConfig).toHaveLength(1);
    const serviceConfig = loaded.control.serviceConfig![0]!;
    expect(serviceConfig).toMatchObject({
      serviceId: SERVICE_ID,
      resolvedConfig: {},
      redactedResolvedConfig: {},
      serviceDb: {
        mongoUrl: "mongodb://127.0.0.1:27017/?directConnection=true",
      },
    });
    expect(
      loaded.activationByServiceOperation.get({
        serviceId: SERVICE_ID,
        target: testServiceRouteTarget(SERVICE_ID, "WebSocketFixtureHttpApi.handle"),
        buildId: serviceConfig.buildId,
      }),
    ).toBe(serviceConfig.activationIdentity);
  });

  it("rejects configShape entries when no service-scoped config source files exist", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeFile(
      join(root, "config.yml"),
      "openaiApiKey: stale-root-secret\n",
    );

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [{ path: "openaiApiKey", type: "string", required: true }],
    };
    assembly.configUses = ["openaiApiKey"];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(
      loadRouterArtifactRoot(root, { configProfile: "prod" }),
    ).rejects.toThrow(
      /serviceAssembly\.configShape requires at least one config source \(configs\/services\/example~com~~websocket_fixture\/config\.yml, configs\/services\/example~com~~websocket_fixture\/config\.prod\.yml, configs\/services\/example~com~~websocket_fixture\/config\.prod\.secret\.yml\)/,
    );
  });

  it("rejects config sources with unsupported top-level namespaces", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.yml",
      ["dashscopeModel: qwen-plus"].join("\n"),
    );

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [{ path: "dashscopeModel", type: "string", required: false }],
    };
    assembly.configUses = ["dashscopeModel"];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /config source config\.yml top-level key dashscopeModel is invalid; use service or packages/,
    );
  });

  it("allows services sharing a protocol identity to have separate activation lookups", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.yml",
      ["service:", "  dashscopeModel: qwen-plus"].join("\n"),
    );

    const serviceA = serviceAssembly(SERVICE_ID) as any;
    serviceA.configShape = configShape(
      "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    serviceA.configUses = ["dashscopeModel"];
    const writtenServiceA = await writeServiceAssembly(root, serviceA);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenServiceA));

    const serviceBId = "example.com/websocket_fixture-admin";
    await writeServiceConfigSource(
      root,
      serviceBId,
      "config.yml",
      ["service:", "  dashscopeModel: qwen-admin"].join("\n"),
    );
    const serviceB = serviceAssembly(serviceBId) as any;
    serviceB.service.protocolIdentity = CONTRACT_IDENTITY;
    serviceB.files[0].fileIrIdentity = CONTRACT_FILE_IR_IDENTITY;
    serviceB.files[0].fileIrHash = CONTRACT_FILE_IR_HASH;
    serviceB.files[0].fileIrPath = CONTRACT_FILE_IR_PATH;
    serviceB.configShape = configShape(
      "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    serviceB.configUses = ["dashscopeModel"];
    const writtenServiceB = await writeServiceAssembly(
      root,
      serviceB,
      serviceBId,
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenServiceB),
      serviceId: serviceBId,
      contractIdentity: CONTRACT_IDENTITY,
    });

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.control.serviceConfig).toHaveLength(2);
    const serviceAConfig = loaded.control.serviceConfig?.find(
      (config) => config.serviceId === SERVICE_ID,
    );
    const serviceBConfig = loaded.control.serviceConfig?.find(
      (config) => config.serviceId === serviceBId,
    );
    expect(serviceAConfig?.activationIdentity).toBeDefined();
    expect(serviceBConfig?.activationIdentity).toBeDefined();
    expect(serviceAConfig?.activationIdentity).not.toBe(
      serviceBConfig?.activationIdentity,
    );
    expect(serviceAConfig?.resolvedConfig).toMatchObject({
      dashscopeModel: "qwen-plus",
    });
    expect(serviceBConfig?.resolvedConfig).toMatchObject({
      dashscopeModel: "qwen-admin",
    });
    expect(
      loaded.activationByServiceOperation.get({
        serviceId: SERVICE_ID,
        target: testServiceRouteTarget(SERVICE_ID, "WebSocketFixtureHttpApi.handle"),
        buildId: serviceAConfig!.buildId,
      }),
    ).toBe(serviceAConfig?.activationIdentity);
    expect(
      loaded.activationByServiceOperation.get({
        serviceId: serviceBId,
        target: testServiceRouteTarget(serviceBId, "WebSocketFixtureHttpApi.handle"),
        buildId: serviceBConfig!.buildId,
      }),
    ).toBe(serviceBConfig?.activationIdentity);
  });

  it("loads artifact config with base, profile, and profile secret overlay order", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeContractFile(root);
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.yml",
      [
        "service:",
        "  dashscopeBaseUrl: https://base.example/v1",
        "  dashscopeModel: qwen-turbo",
      ].join("\n"),
    );
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.prod.yml",
      ["service:", "  dashscopeModel: qwen-plus"].join("\n"),
    );
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.prod.secret.yml",
      ["service:", "  dashscopeApiKey: local-secret"].join("\n"),
    );

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscopeBaseUrl",
          type: "string",
          required: true,
        },
        {
          path: "dashscopeModel",
          type: "string",
          required: true,
        },
        {
          path: "dashscopeApiKey",
          type: "string",
          required: true,
        },
      ],
    };
    assembly.configUses = [
      "dashscopeBaseUrl",
      "dashscopeModel",
      "dashscopeApiKey",
    ];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    const loaded = await loadRouterArtifactRoot(root, {
      configProfile: "prod",
    });

    expect(loaded.control.serviceConfig?.[0]?.resolvedConfig).toEqual({
      dashscopeBaseUrl: "https://base.example/v1",
      dashscopeModel: "qwen-plus",
      dashscopeApiKey: "local-secret",
    });
    expect(
      loaded.control.serviceConfig?.[0]?.redactedResolvedConfig,
    ).toMatchObject({
      dashscopeApiKey: "[REDACTED]",
    });
  });

  it("rejects array-shaped configShape in service assemblies", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeContractFile(root);

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.configShape = [
      {
        path: "dashscopeModel",
        type: "string",
        required: false,
      },
    ];
    assembly.configUses = ["dashscopeModel"];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.configShape must be an object/,
    );
  });

  it("rejects legacy service assembly values fields", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeContractFile(root);

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.valuesPolicy = {
      schemaVersion: "skiff-values-policy-v1",
      requirements: [],
    };
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.valuesPolicy is no longer supported; use configShape/,
    );
  });

  it("accepts compiler service assembly identities that include full prelude metadata", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeContractFile(root);

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.preludeIdentity =
      "skiff-prelude-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333";
    assembly.prelude = {
      identity: assembly.preludeIdentity,
      schemaIdentity:
        "skiff-prelude-schema-v1:sha256:4444444444444444444444444444444444444444444444444444444444444444",
      types: ["Json", ["Secret", "String"].join("")],
      roots: ["config"],
    };
    assembly.service.interfaces = {
      entries: [
        {
          module: "api.http",
          path: "",
        },
      ],
      interfaces: {
        SampleHttpApi: {
          modulePath: "api.http",
          name: "SampleHttpApi",
        },
      },
    };
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscopeModel",
          type: "string",
          required: false,
        },
      ],
    };
    assembly.configUses = ["dashscopeModel"];
    await writeServiceConfigSource(
      root,
      SERVICE_ID,
      "config.yml",
      ["service:", "  dashscopeModel: qwen-plus"].join("\n"),
    );

    const hash = artifactHash(compilerServiceAssemblyHashInput(assembly));
    const identity = `skiff-service-assembly-v1:sha256:${hash}`;
    assembly.service.assemblyIdentity = identity;
    const assemblyPath = await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      hash,
      assembly,
    );
    await writeIndexPointer(
      root,
      serviceAssemblyIndex({
        identity,
        path: assemblyPath,
      }),
    );

    await expect(loadRouterArtifactRoot(root)).resolves.toMatchObject({
      control: {
        fingerprint: identity,
      },
    });
  });

  it("includes service api metadata in service assembly identities", async () => {
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.api = {
      entries: [
        {
          module: "api.http",
          path: "",
        },
      ],
      interfaces: {
        WebSocketFixtureHttpApi: {
          modulePath: "api.http",
          name: "WebSocketFixtureHttpApi",
        },
      },
    };

    const withApi = artifactHash(routerServiceAssemblyHashInput(assembly));
    delete assembly.service.api;
    const withoutApi = artifactHash(routerServiceAssemblyHashInput(assembly));

    expect(withApi).not.toBe(withoutApi);
  });

  it("ignores legacy service interfaces in service assembly identities", async () => {
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const withoutApi = routerServiceAssemblyHashInput(assembly);
    assembly.service.interfaces = {
      entries: [
        {
          module: "legacy.http",
          path: "",
        },
      ],
    };

    const input = routerServiceAssemblyHashInput(assembly);

    expect(input).toEqual(withoutApi);
    expect(input.service).not.toHaveProperty("api");
    expect(input.service).not.toHaveProperty("interfaces");
  });

  it("keeps service api identity input independent from legacy interfaces", async () => {
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.api = {
      entries: [
        {
          module: "api.http",
          path: "",
        },
      ],
    };
    const withApi = routerServiceAssemblyHashInput(assembly);
    assembly.service.interfaces = {
      entries: [
        {
          module: "legacy.http",
          path: "",
        },
      ],
    };

    const input = routerServiceAssemblyHashInput(assembly);

    expect(input).toEqual(withApi);
    expect(input.service).toMatchObject({ api: assembly.service.api });
    expect(input.service).not.toHaveProperty("interfaces");
  });

  it("rejects service assemblies whose config shape is tampered without changing identity", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeContractFile(root);

    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.preludeIdentity =
      "skiff-prelude-v1:sha256:3333333333333333333333333333333333333333333333333333333333333333";
    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscopeModel",
          type: "string",
          required: false,
        },
      ],
    };
    assembly.configUses = ["dashscopeModel"];
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    assembly.configShape = {
      schemaVersion: "skiff-config-shape-v1",
      entries: [
        {
          path: "dashscopeModel",
          type: "number",
          required: false,
        },
      ],
    };
    await writeFile(
      join(root, writtenAssembly.path),
      JSON.stringify(assembly, null, 2),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly content sha256 .* does not match assemblyIdentity hash/,
    );
  });

  it("rejects service assembly identities that omitted missing sourceMap from the hash input", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID, {
      assemblyIdentity: "",
    }) as any;
    const legacyHash = artifactHash(
      serviceAssemblyHashInputWithoutSourceMap(assembly),
    );
    const legacyIdentity = `skiff-service-assembly-v1:sha256:${legacyHash}`;
    assembly.service.assemblyIdentity = legacyIdentity;
    const path = await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      legacyHash,
      assembly,
    );
    await writeIndexPointer(
      root,
      serviceAssemblyIndex({ identity: legacyIdentity, path }),
    );
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly content sha256 .* does not match assemblyIdentity hash/,
    );
  });

  it("rejects service assembly identities that omitted db metadata from the hash input", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID, {
      assemblyIdentity: "",
    }) as any;
    assembly.db = [
      {
        modulePath: "internal.example",
        sourceRole: "service",
        kind: "object",
        type: { kind: "localType", typeIndex: 0 },
        typeName: "Thread",
        collectionName: "thread",
        key: {
          name: "id",
          type: { kind: "builtin", name: "string" },
        },
        fields: [
          {
            name: "ownerUserId",
            type: { kind: "builtin", name: "string" },
          },
          {
            name: "title",
            type: { kind: "builtin", name: "string" },
          },
        ],
        retention: null,
        indexes: [],
      },
    ];
    const legacyHash = artifactHash(
      serviceAssemblyHashInputWithoutDb(assembly),
    );
    const legacyIdentity = `skiff-service-assembly-v1:sha256:${legacyHash}`;
    assembly.service.assemblyIdentity = legacyIdentity;
    const path = await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      legacyHash,
      assembly,
    );
    await writeIndexPointer(
      root,
      serviceAssemblyIndex({ identity: legacyIdentity, path }),
    );
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly content sha256 .* does not match assemblyIdentity hash/,
    );
  });

  it("rejects service assembly pointers whose contract identity differs from the assembly protocol identity", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      contractIdentity: MISMATCH_CONTRACT_IDENTITY,
    });
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID),
    );
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /contractIdentity must match serviceAssembly\.service\.protocolIdentity/,
    );
  });

  it("rejects unsupported split pointer schema versions", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      schemaVersion: "skiff-artifact-index-v2",
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schemaVersion must be skiff-service-build-v1/,
    );
  });

  it("rejects split pointers without schemaVersion", async () => {
    const root = await createArtifactRoot();
    const pointer = serviceAssemblyIndex();
    delete (pointer as Record<string, unknown>).schemaVersion;
    await writeIndexPointer(root, pointer);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /schemaVersion must be skiff-service-build-v1/,
    );
  });

  it("rejects build records that omit service id", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const pointer = {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: SERVICE_ID,
      contractIdentity: CONTRACT_IDENTITY,
      serviceAssembly: {
        assemblyIdentity: ASSEMBLY_IDENTITY,
        assemblyPath: serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
      },
    };
    delete (pointer as Record<string, unknown>).serviceId;
    await writeIndexPointer(root, pointer as Record<string, unknown>);
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceId must be a non-empty string/,
    );
  });

  it("rejects service assembly pointers whose service id differs from the assembly service id", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      serviceId: "example.com/other",
    });
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceId must match serviceAssembly\.service\.id/,
    );
  });

  it("rejects legacy top-level artifactIdentity service assembly aliases", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      artifactIdentity: OTHER_ASSEMBLY_IDENTITY,
    });
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /artifactIdentity is not supported in artifact pointers/,
    );
  });

  it("rejects service assembly identities without sha256 format", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const badIdentity = "skiff-service-assembly-v1:not-sha";
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      serviceAssembly: {
        assemblyIdentity: badIdentity,
        assemblyPath: serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
      },
    });
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID, { assemblyIdentity: badIdentity }),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /must include :sha256:/,
    );
  });

  it("rejects service assembly paths whose hash disagrees with assembly identity", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      serviceAssembly: {
        assemblyIdentity: ASSEMBLY_IDENTITY,
        assemblyPath: serviceAssemblyArtifactPath(
          SERVICE_ID,
          OTHER_ASSEMBLY_HASH,
        ),
      },
    });
    await writeServiceAssemblyValue(
      root,
      SERVICE_ID,
      OTHER_ASSEMBLY_HASH,
      serviceAssembly(SERVICE_ID),
    );

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /identity hash .* does not match assemblyIdentity hash/,
    );
  });

  it("rejects service assemblies whose content disagrees with assembly identity", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.operations[0].entrypoint = `service.${SERVICE_ID}.Tampered.connect`;
    await writeIndexPointer(root, serviceAssemblyIndex());
    await writeServiceAssemblyValue(root, SERVICE_ID, ASSEMBLY_HASH, assembly);
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly content sha256/,
    );
  });

  it("rejects non-canonical serviceAssembly revision ids", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.revisionId = "revision-service-assembly";
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.service\.revisionId must be <64 lowercase hex>/,
    );
  });

  it("loads compiler service assemblies with http response maxBytes", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.http = {
      response: {
        maxBytes: 134217728,
      },
    };
    const writtenAssembly = await writeCompilerServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);

    expect(loaded.manifest.service.id).toBe(SERVICE_ID);
    expect(loaded.control.fingerprint).toBe(writtenAssembly.identity);
  });

  it("rejects service assemblies with unsupported service http request metadata", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.http = {
      request: {
        maxBytes: 1024,
      },
    };
    const writtenAssembly = await writeCompilerServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.service\.http does not support serviceAssembly\.service\.http\.request/,
    );
  });

  it("rejects service assemblies with unsupported service http response metadata", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.service.http = {
      response: {
        maxBytes: 134217728,
        foo: true,
      },
    };
    const writtenAssembly = await writeCompilerServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.service\.http\.response does not support serviceAssembly\.service\.http\.response\.foo/,
    );
  });

  it("rejects contract identities without the protocol identity prefix", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      contractIdentity: `skiff-contract-v1:sha256:${identityHash(CONTRACT_IDENTITY)}`,
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /contractIdentity prefix must be skiff-protocol-v1/,
    );
  });

  it("rejects service assemblies with top-level gateway projections", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.http = {
      raw: {
        operation: "WebSocketFixtureHttpApi.handle",
        target: `gateway.${publicationStorageSegment(SERVICE_ID)}.http.raw`,
      },
    };
    await writeIndexPointer(root, serviceAssemblyIndex());
    await writeServiceAssemblyValue(root, SERVICE_ID, ASSEMBLY_HASH, assembly);
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /top-level http\/websocket/,
    );
  });

  it("uses service assembly operation schemas without type projection fallback", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.gateway.http;
    const rawHttpOperation = (assembly.operations as Array<any>).find(
      (operation: any) => operation.operation === "WebSocketFixtureHttpApi.handle",
    )!;
    rawHttpOperation.parameters[0] = {
      name: "request",
      schema: {
        type: "object",
        properties: {
          fromAssemblySchema: { type: "string" },
        },
        required: ["fromAssemblySchema"],
        additionalProperties: false,
      },
    };
    rawHttpOperation.response = {
      type: "object",
      properties: {
        fromAssemblyResponse: { type: "boolean" },
      },
      required: ["fromAssemblyResponse"],
      additionalProperties: false,
    };
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const operation = loaded.manifest.operations.find(
      (operation) => operation.operation === "WebSocketFixtureHttpApi.handle",
    );

    expect(operation?.parameters[0]?.schema).toMatchObject({
      properties: {
        fromAssemblySchema: { type: "string" },
      },
    });
    expect(operation?.parameters[0]?.schema).not.toHaveProperty(
      "properties.method",
    );
    expect(operation?.response).toMatchObject({
      properties: {
        fromAssemblyResponse: { type: "boolean" },
      },
    });
  });

  it("projects serverStream service assembly operation mode into the runtime manifest", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const receiveOperation = (assembly.operations as Array<any>).find(
      (operation: any) => operation.operation === "WebSocketFixtureConnection.receive",
    )!;
    receiveOperation.mode = "serverStream";
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const operation = loaded.manifest.operations.find(
      (operation) => operation.operation === "WebSocketFixtureConnection.receive",
    );

    expect(operation?.mode).toBe("serverStream");
    expect(
      loaded.manifest.websocketEntry?.receive?.operationManifest.mode,
    ).toBe("serverStream");
  });

  it("does not derive dispatch mode from serviceUnit publicationAbi maySuspend", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    const serviceUnitAssembly = structuredClone(assembly);
    serviceUnitAssembly.operations[0].mode = "serverStream";
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    const serviceUnit = await writeServiceUnit(
      root,
      SERVICE_ID,
      serviceTestVersion(SERVICE_ID),
      { assembly: serviceUnitAssembly },
    );
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(writtenAssembly),
      serviceUnit,
    });
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const operation = loaded.manifest.operations.find(
      (operation) => operation.operation === assembly.operations[0].operation,
    );

    expect(operation?.mode).toBe("unary");
  });

  it("rejects serviceUnit operations with missing mode", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.operations[0].mode;
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.operation\.mode must be unary or serverStream/,
    );
  });

  it("rejects invalid service assembly operation mode before route projection", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    assembly.operations[0].mode = "clientStream";
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.operation\.mode must be unary or serverStream/,
    );
  });

  it("normalizes service assembly websocket context wrappers before gateway identity validation", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.gateway.http;

    const contextSchema = connectionContextSchema();
    const websocket = assembly.gateway.websocket;
    websocket.context = {
      type: "ConnectionContext",
      schema: contextSchema,
    };
    websocket.connect.adapterArgs = [
      { param: "request", source: { kind: "websocket.connectRequest" } },
    ];
    const connectOperation = (assembly.operations as Array<any>).find(
      (operation: any) => operation.operation === "WebSocketFixtureConnection.connect",
    )!;
    connectOperation.parameters = [
      {
        name: "request",
        schema: webSocketConnectRequestSchema(),
      },
    ];

    const directManifest = loadManifest({
      schemaVersion: "skiff-runtime-manifest-v1",
      service: {
        id: assembly.service.id,
        revisionId: assembly.service.revisionId,
        protocolIdentity: assembly.service.protocolIdentity,
      },
      operations: assembly.operations.map((operation: any) => ({
        ...operation,
        target: operation.entrypoint,
        mode: operation.mode ?? "unary",
      })),
      gateway: {
        websocket: {
          ...websocket,
          context: contextSchema,
        },
      },
    } as any);
    const directEntry = directManifest.websocketEntry;
    expect(directEntry?.connect?.adapterArgs).toEqual([
      { param: "request", source: { kind: "websocket.connectRequest" } },
    ]);
    websocket.connect.gatewayEntryIdentity =
      directEntry!.connect!.gatewayEntryIdentity;
    websocket.receive.gatewayEntryIdentity =
      directEntry!.receive.gatewayEntryIdentity;
    websocket.gatewayEntryIdentity = directEntry!.gatewayEntryIdentity;

    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const loadedEntry = loaded.manifest.websocketEntry;
    expect(loadedEntry?.context).toEqual(contextSchema);
    expect(loadedEntry?.connect?.adapterArgs).toEqual([
      { param: "request", source: { kind: "websocket.connectRequest" } },
    ]);
    expect(loadedEntry?.connect?.gatewayEntryIdentity).toBe(
      directEntry!.connect!.gatewayEntryIdentity,
    );
    expect(loadedEntry?.gatewayEntryIdentity).toBe(
      directEntry!.gatewayEntryIdentity,
    );
  });

  it("uses service assembly oneOf schemas without type projection fallback", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await mkdir(join(root, "files"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.gateway.http;
    const rawHttpOperation = (assembly.operations as Array<any>).find(
      (operation: any) => operation.operation === "WebSocketFixtureHttpApi.handle",
    )!;
    rawHttpOperation.parameters[0] = {
      name: "request",
      schema: {
        oneOf: [
          {
            type: "object",
            properties: {
              tag: { type: "string", enum: ["empty"] },
            },
            required: ["tag"],
            additionalProperties: false,
          },
        ],
      },
    };
    rawHttpOperation.response = {
      oneOf: [
        {
          type: "object",
          properties: {
            tag: { type: "string", enum: ["empty"] },
          },
          required: ["tag"],
          additionalProperties: false,
        },
      ],
    };
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));
    await writeContractFile(root);

    const loaded = await loadRouterArtifactRoot(root);
    const operation = loaded.manifest.operations.find(
      (operation) => operation.operation === "WebSocketFixtureHttpApi.handle",
    );

    expect(operation?.parameters[0]?.schema).toMatchObject({
      oneOf: [
        {
          properties: {
            tag: { enum: ["empty"] },
          },
        },
      ],
    });
    expect(operation?.parameters[0]?.schema).not.toEqual({ type: "any" });
    expect(operation?.response).toMatchObject({
      oneOf: [
        {
          properties: {
            tag: { enum: ["empty"] },
          },
        },
      ],
    });
  });

  it("rejects operation parameters without compiler-projected schema", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.operations[0].parameters[0].schema;
    assembly.operations[0].parameters[0].type = {
      kind: "localType",
      typeIndex: 0,
    };
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.operation\.parameter\.schema is required/,
    );
  });

  it("rejects operations without compiler-projected response schema", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    const assembly = serviceAssembly(SERVICE_ID) as any;
    delete assembly.operations[0].response;
    assembly.operations[0].returnType = { kind: "localType", typeIndex: 1 };
    const writtenAssembly = await writeServiceAssembly(root, assembly);
    await writeIndexPointer(root, serviceAssemblyIndex(writtenAssembly));

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.operation\.response is required/,
    );
  });

  it("rejects serviceAssembly string pointers", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: SERVICE_ID,
      contractIdentity: CONTRACT_IDENTITY,
      serviceAssembly: serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly must be an object/,
    );
  });

  it("rejects serviceAssemblyRef pointers", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: SERVICE_ID,
      contractIdentity: CONTRACT_IDENTITY,
      serviceAssemblyRef: serviceAssemblyArtifactPath(
        SERVICE_ID,
        ASSEMBLY_HASH,
      ),
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssemblyRef is not supported/,
    );
  });

  it("rejects serviceAssemblyRef pointers even when serviceAssembly is present", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      ...serviceAssemblyIndex(),
      serviceAssemblyRef: serviceAssemblyArtifactPath(
        SERVICE_ID,
        ASSEMBLY_HASH,
      ),
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssemblyRef is not supported/,
    );
  });

  it.each(["path", "artifactPath", "identity", "artifactIdentity"] as const)(
    "rejects serviceAssembly.%s aliases even when canonical fields are present",
    async (aliasKey) => {
      const root = await createArtifactRoot();
      await writeIndexPointer(root, {
        ...serviceAssemblyIndex(),
        serviceAssembly: {
          assemblyIdentity: ASSEMBLY_IDENTITY,
          assemblyPath: serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
          [aliasKey]: aliasKey.includes("Identity")
            ? ASSEMBLY_IDENTITY
            : serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
        },
      });

      await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
        new RegExp(`serviceAssembly\\.${aliasKey} is not supported`),
      );
    },
  );

  it("rejects serviceAssemblyPath shorthand without serviceAssembly", async () => {
    const root = await createArtifactRoot();
    await mkdir(join(root, "assemblies", "services"), { recursive: true });
    await writeIndexPointer(root, {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: SERVICE_ID,
      contractIdentity: CONTRACT_IDENTITY,
      serviceAssemblyPath: serviceAssemblyArtifactPath(
        SERVICE_ID,
        ASSEMBLY_HASH,
      ),
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly is required/,
    );
  });

  it("does not derive router manifest data from legacy serviceIr pointers", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: "example.com/legacy",
      contractIdentity: LEGACY_CONTRACT_IDENTITY,
      artifactIdentity: "skiff-service-ir-v1:sha256:legacy",
      serviceIr: "files/legacy.json",
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /legacy serviceIr/,
    );
  });

  it("rejects legacy serviceIr pointers even when serviceAssembly is present", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: SERVICE_ID,
      contractIdentity: CONTRACT_IDENTITY,
      artifactIdentity: "skiff-service-ir-v1:sha256:legacy",
      serviceIr: "blobs/sha256/legacy.json",
      serviceAssembly: {
        assemblyIdentity: ASSEMBLY_IDENTITY,
        assemblyPath: serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
      },
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /legacy serviceIr/,
    );
  });

  it("rejects legacy serviceIr identity inside serviceAssembly pointer", async () => {
    const root = await createArtifactRoot();
    await writeIndexPointer(root, {
      schemaVersion: "skiff-artifact-index-v1",
      serviceId: SERVICE_ID,
      contractIdentity: CONTRACT_IDENTITY,
      serviceAssembly: {
        artifactIdentity: "skiff-service-ir-v1:sha256:legacy",
        assemblyPath: serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
      },
    });

    await expect(loadRouterArtifactRoot(root)).rejects.toThrow(
      /serviceAssembly\.artifactIdentity is not supported/,
    );
  });
});

function routerManifest(serviceId: string) {
  return {
    schemaVersion: "skiff-runtime-manifest-v1",
    service: {
      id: serviceId,
      revisionId: revisionIdFixture(`${serviceId}:router-manifest`),
      protocolIdentity: contractIdentityForService(serviceId),
    },
    operations: [
      {
        operation: "Ping.ping",
        target: `service.${publicationStorageSegment(serviceId)}.Ping.ping`,
        mode: "unary",
        parameters: [],
        response: { type: "object", additionalProperties: true },
      },
    ],
    gateway: {},
  };
}

function serviceAssemblyIndex(assembly?: WrittenAssembly) {
  return {
    schemaVersion: "skiff-artifact-index-v1",
    serviceId: SERVICE_ID,
    contractIdentity: CONTRACT_IDENTITY,
    serviceAssembly: {
      assemblyIdentity: assembly?.identity ?? ASSEMBLY_IDENTITY,
      assemblyPath:
        assembly?.path ??
        serviceAssemblyArtifactPath(SERVICE_ID, ASSEMBLY_HASH),
    },
  };
}

interface WrittenAssembly {
  identity: string;
  path: string;
}

async function writeServiceAssembly(
  root: string,
  assembly: Record<string, any>,
  serviceId = SERVICE_ID,
): Promise<WrittenAssembly> {
  const hash = artifactHash(serviceAssemblyHashInput(assembly));
  const identity = `skiff-service-assembly-v1:sha256:${hash}`;
  assembly.service.assemblyIdentity = identity;
  const path = serviceIdArtifactFilePath(
    ["assemblies", "services"],
    serviceId,
    hash,
  );
  await mkdir(dirname(join(root, path)), { recursive: true });
  await writeFile(join(root, path), JSON.stringify(assembly, null, 2));
  return { identity, path };
}

async function writeCompilerServiceAssembly(
  root: string,
  assembly: Record<string, any>,
  serviceId = SERVICE_ID,
): Promise<WrittenAssembly> {
  const hash = artifactHash(compilerServiceAssemblyHashInput(assembly));
  const identity = `skiff-service-assembly-v1:sha256:${hash}`;
  assembly.service.assemblyIdentity = identity;
  const path = serviceIdArtifactFilePath(
    ["assemblies", "services"],
    serviceId,
    hash,
  );
  await mkdir(dirname(join(root, path)), { recursive: true });
  await writeFile(join(root, path), JSON.stringify(assembly, null, 2));
  return { identity, path };
}

async function writeIndexPointer(
  root: string,
  pointer: Record<string, unknown>,
) {
  const serviceId = readPointerString(pointer, "serviceId") ?? SERVICE_ID;
  const buildId = fixtureIdentity(
    "skiff-service-build-v1",
    stableStringify(pointer),
  );
  const version = serviceTestVersion(serviceId);
  await writeVersionPointer(root, { buildId, serviceId, version });
  const serviceUnit =
    pointer.serviceUnit ??
    pointer.serviceUnitPath ??
    (await writeServiceUnitForPointer(root, pointer, serviceId, version));

  const buildRecord: Record<string, unknown> = {
    ...pointer,
    buildId,
    serviceVersion: version,
    serviceUnit,
  };
  const schemaVersion = readPointerString(pointer, "schemaVersion");
  if (schemaVersion === "skiff-artifact-index-v1") {
    buildRecord.schemaVersion = "skiff-service-build-v1";
  } else if (schemaVersion === undefined) {
    delete buildRecord.schemaVersion;
  }
  const serviceAssembly = pointer.serviceAssembly;
  if (
    buildRecord.fingerprint === undefined &&
    serviceAssembly !== null &&
    typeof serviceAssembly === "object" &&
    "assemblyIdentity" in serviceAssembly &&
    typeof serviceAssembly.assemblyIdentity === "string"
  ) {
    buildRecord.fingerprint = serviceAssembly.assemblyIdentity;
  }

  const serviceIdSegments = serviceIdPathSegments(serviceId);
  await mkdir(join(root, "builds", "services", ...serviceIdSegments), {
    recursive: true,
  });
  await writeFile(
    join(
      root,
      "builds",
      "services",
      ...serviceIdSegments,
      `${identityHash(buildId)}.json`,
    ),
    JSON.stringify(buildRecord, null, 2),
  );
}

async function writeDevReloadPointer(
  root: string,
  assembly: WrittenAssembly,
  options: {
    buildId?: string;
    contractHash?: string;
    omitBuildId?: boolean;
    profile?: string;
    serviceId?: string;
    serviceAssemblyRef?: string;
  } = {},
) {
  const serviceId = options.serviceId ?? SERVICE_ID;
  const protocolIdentity = contractIdentityForService(serviceId);
  const serviceUnit = await writeServiceUnit(
    root,
    serviceId,
    serviceTestVersion(serviceId),
    {
      assembly: await readWrittenAssemblyValue(root, assembly),
    },
  );
  const pointerPath = serviceIdJsonPath(root, ["dev", "services"], serviceId);
  await mkdir(dirname(pointerPath), { recursive: true });
  await writeFile(
    pointerPath,
    JSON.stringify(
      {
        mode: "dev",
        serviceId,
        profile: options.profile ?? "dev",
        contractHash: options.contractHash ?? identityHash(protocolIdentity),
        protocolIdentity,
        serviceAssembly: {
          assemblyIdentity: assembly.identity,
          assemblyPath: assembly.path,
        },
        serviceUnit,
        ...(options.omitBuildId === true
          ? {}
          : {
              buildId:
                options.buildId ??
                `skiff-service-build-v1:sha256:${identityHash(assembly.identity)}`,
            }),
        ...(options.serviceAssemblyRef !== undefined
          ? { serviceAssemblyRef: options.serviceAssemblyRef }
          : {}),
      },
      null,
      2,
    ),
  );
}

async function writeVersionPointer(
  root: string,
  version: { buildId: string; serviceId: string; version: string },
) {
  const serviceIdSegments = serviceIdPathSegments(version.serviceId);
  await mkdir(join(root, "versions", "services", ...serviceIdSegments), {
    recursive: true,
  });
  await writeFile(
    join(
      root,
      "versions",
      "services",
      ...serviceIdSegments,
      `${version.version}.json`,
    ),
    JSON.stringify(
      {
        schemaVersion: "skiff-service-version-pointer-v1",
        serviceId: version.serviceId,
        version: version.version,
        buildId: version.buildId,
        updatedAt: "2026-05-05T00:00:00.000Z",
        updatedBy: "test",
      },
      null,
      2,
    ),
  );
}

async function writeBuildRecord(
  root: string,
  input: {
    buildId: string;
    serviceId: string;
    assembly: WrittenAssembly;
    serviceVersion: string;
  },
) {
  const serviceUnit = await writeServiceUnit(
    root,
    input.serviceId,
    input.serviceVersion,
    {
      assembly: await readWrittenAssemblyValue(root, input.assembly),
    },
  );
  const serviceIdSegments = serviceIdPathSegments(input.serviceId);
  await mkdir(join(root, "builds", "services", ...serviceIdSegments), {
    recursive: true,
  });
  await writeFile(
    join(
      root,
      "builds",
      "services",
      ...serviceIdSegments,
      `${identityHash(input.buildId)}.json`,
    ),
    JSON.stringify(
      {
        schemaVersion: "skiff-service-build-v1",
        serviceId: input.serviceId,
        serviceVersion: input.serviceVersion,
        buildId: input.buildId,
        serviceAssembly: {
          assemblyIdentity: input.assembly.identity,
          assemblyPath: input.assembly.path,
        },
        serviceUnit,
        createdAt: "2026-05-05T00:00:00.000Z",
      },
      null,
      2,
    ),
  );
}

async function writeBuildRecordWithServiceUnitPath(
  root: string,
  input: {
    buildId: string;
    serviceId: string;
    assembly: WrittenAssembly;
    serviceVersion: string;
    serviceUnitPath: string;
  },
) {
  const serviceIdSegments = serviceIdPathSegments(input.serviceId);
  await mkdir(join(root, "builds", "services", ...serviceIdSegments), {
    recursive: true,
  });
  await writeFile(
    join(
      root,
      "builds",
      "services",
      ...serviceIdSegments,
      `${identityHash(input.buildId)}.json`,
    ),
    JSON.stringify(
      {
        schemaVersion: "skiff-service-build-v1",
        serviceId: input.serviceId,
        serviceVersion: input.serviceVersion,
        buildId: input.buildId,
        serviceAssembly: {
          assemblyIdentity: input.assembly.identity,
          assemblyPath: input.assembly.path,
        },
        serviceUnit: {
          unitPath: input.serviceUnitPath,
        },
        createdAt: "2026-05-05T00:00:00.000Z",
      },
      null,
      2,
    ),
  );
}

async function writeServiceUnitForPointer(
  root: string,
  pointer: Record<string, unknown>,
  serviceId: string,
  version: string,
) {
  const assembly =
    (await readPointerAssemblyValue(root, pointer)) ??
    serviceAssembly(serviceId);
  return await writeServiceUnit(root, serviceId, version, { assembly });
}

async function readPointerAssemblyValue(
  root: string,
  pointer: Record<string, unknown>,
): Promise<Record<string, any> | undefined> {
  const serviceAssembly = pointer.serviceAssembly;
  if (!serviceAssembly || typeof serviceAssembly !== "object") {
    return undefined;
  }
  const assemblyPath = (serviceAssembly as Record<string, unknown>)
    .assemblyPath;
  if (typeof assemblyPath !== "string" || assemblyPath.length === 0) {
    return undefined;
  }
  return await readAssemblyValueAtPath(root, assemblyPath);
}

async function readWrittenAssemblyValue(
  root: string,
  assembly: WrittenAssembly,
): Promise<Record<string, any>> {
  const value = await readAssemblyValueAtPath(root, assembly.path);
  if (!value) {
    throw new Error(`expected written assembly ${assembly.path}`);
  }
  return value;
}

async function readAssemblyValueAtPath(
  root: string,
  assemblyPath: string,
): Promise<Record<string, any> | undefined> {
  try {
    return JSON.parse(
      await readFile(join(root, assemblyPath), "utf8"),
    ) as Record<string, any>;
  } catch {
    return undefined;
  }
}

async function writeServiceUnit(
  root: string,
  serviceId = SERVICE_ID,
  version = `${serviceId}-test`,
  options: {
    assembly?: Record<string, any>;
    entrypoints?: Record<string, string>;
    packageDependencies?: unknown[];
    packageAbiExpectations?: unknown[];
    config?: unknown;
  } = {},
) {
  const assembly = options.assembly ?? serviceAssembly(serviceId);
  const operations = serviceUnitOperationsFromAssembly(
    serviceId,
    assembly,
    options.entrypoints,
  );
  const value = {
    schemaVersion: "skiff-service-unit-v1",
    service: {
      id: serviceId,
    },
    version,
    protocolIdentity: contractIdentityForService(serviceId),
    publicationAbi: servicePublicationAbiFromAssembly(
      serviceId,
      version,
      assembly,
    ),
    files: [],
    packageDependencies: options.packageDependencies ?? [],
    ...(options.packageAbiExpectations !== undefined
      ? { packageAbiExpectations: options.packageAbiExpectations }
      : {}),
    operations,
    gateway: serviceUnitGatewayFromAssembly(assembly),
    config: options.config ?? {},
  };
  const unitHash = artifactHash(value);
  const unitIdentity = `skiff-service-unit-v1:sha256:${unitHash}`;
  const unitPath = serviceIdArtifactFilePath(
    ["units", "services"],
    serviceId,
    unitHash,
  );
  await mkdir(dirname(join(root, unitPath)), { recursive: true });
  await writeFile(join(root, unitPath), JSON.stringify(value, null, 2));
  return {
    schemaVersion: "skiff-service-unit-v1",
    unitIdentity,
    unitHash,
    unitPath,
  };
}

function serviceUnitOperationsFromAssembly(
  serviceId: string,
  assembly: Record<string, any>,
  entrypoints: Record<string, string> = {},
) {
  const operations = Array.isArray(assembly.operations)
    ? assembly.operations
    : serviceAssembly(serviceId).operations;
  return operations.map((operation: any, index: number) => {
    const operationName = String(operation.operation);
    const implementation = implementationFromAssemblyOperation(
      operation,
      serviceId,
      operationName,
    );
    const symbol =
      entrypoints[operationName] ??
      implementation.symbol ??
      `__skiff_service_operation_adapter_${operationName.replaceAll(".", "_")}`;
    return {
      kind: "localExecutable",
      operation: serviceOperationAbiRef(operation),
      executable: {
        fileRef: {
          fileIrIdentity: CONTRACT_FILE_IR_IDENTITY,
          modulePath: implementation.modulePath,
          artifactPath: CONTRACT_FILE_IR_PATH,
        },
        executableIndex: index,
        callableAbiId: `callable:${implementation.modulePath}.${symbol}`,
        callableKind: "publicFunction",
      },
    };
  });
}

function servicePublicationAbiFromAssembly(
  serviceId: string,
  version: string,
  assembly: Record<string, any>,
) {
  const operations = Array.isArray(assembly.operations)
    ? assembly.operations
    : serviceAssembly(serviceId).operations;
  const operationAbi = operations.map((operation: any) => ({
    operation: serviceOperationAbiRef(operation),
    publicSignature: {
      params: Array.isArray(operation.parameters)
        ? operation.parameters.map((parameter: any) => ({
            name: String(parameter.name),
            ty: { kind: "builtin", name: "Json" },
          }))
        : [],
      returnType: { kind: "builtin", name: "Json" },
      maySuspend: operation.mode === "serverStream",
    },
  }));
  return {
    schemaVersion: "skiff-publication-abi-unit-v1",
    publicationId: serviceId,
    version,
    abiIdentity: fixtureIdentity(
      "skiff-publication-abi-v1",
      `${serviceId}:${version}:publication-abi`,
    ),
    operationExports: operationAbi.map((entry) => entry.operation),
    operationAbi,
    sourceCallOperationIndex: operationAbi.map((entry) => ({
      sourceCallPath: entry.operation.publicPath,
      operation: entry.operation,
    })),
  };
}

function serviceOperationAbiRef(operation: Record<string, any>) {
  const operationName = String(operation.operation);
  return {
    operationAbiId:
      typeof operation.operationAbiId === "string"
        ? operation.operationAbiId
        : operationAbiIdForServiceOperation(operationName),
    kind: "publicFunction",
    publicPath: operationName,
    displayName: operationName,
  };
}

function implementationFromAssemblyOperation(
  operation: Record<string, any>,
  serviceId: string,
  operationName: string,
): { modulePath: string; symbol: string } {
  const implementation = operation.implementation;
  if (
    implementation &&
    typeof implementation === "object" &&
    typeof implementation.modulePath === "string" &&
    typeof implementation.symbol === "string"
  ) {
    return {
      modulePath: implementation.modulePath,
      symbol: implementation.symbol,
    };
  }
  const target = testServiceRouteTarget(serviceId, operationName);
  const dotIndex = target.lastIndexOf(".");
  return {
    modulePath: target.slice(0, dotIndex),
    symbol: target.slice(dotIndex + 1),
  };
}

function serviceUnitGatewayFromAssembly(assembly: Record<string, any>) {
  const websocket = assembly.gateway?.websocket;
  if (!websocket) {
    return {};
  }
  const receiveOperation = websocket.receive?.operation;
  const connectOperation = websocket.connect?.operation;
  return {
    webSockets: {
      default: {
        path: websocket.path,
        operation: receiveOperation,
        ...(receiveOperation !== undefined
          ? { operationAbiId: operationAbiIdForServiceOperation(receiveOperation) }
          : {}),
        ...(connectOperation !== undefined
          ? { connectOperation: websocket.connect.operation }
          : {}),
        ...(connectOperation !== undefined
          ? {
              connectOperationAbiId:
                operationAbiIdForServiceOperation(connectOperation),
            }
          : {}),
      },
    },
  };
}

async function writeContractFile(
  root: string,
  value: unknown = CONTRACT_FILE_IR_UNIT,
) {
  await mkdir(join(root, "units", "files"), { recursive: true });
  await writeFile(
    join(root, CONTRACT_FILE_IR_PATH),
    JSON.stringify(value, null, 2),
  );
}

interface WrittenPackageAssembly {
  identity: string;
  path: string;
  value: Record<string, unknown>;
}

interface WrittenPackageUnit {
  buildIdentity: string;
  unitPath: string;
  value: Record<string, unknown>;
}

async function writePackageUnit(
  root: string,
  id: string,
  version: string,
  seed: string,
  options: {
    dependencies?: unknown[];
    configAndEffectMetadata?: unknown;
  } = {},
): Promise<WrittenPackageUnit> {
  const value: Record<string, any> = {
    schemaVersion: "skiff-package-unit-v1",
    packageId: id,
    version,
    buildIdentity: fixtureIdentity("skiff-package-build-v1", `${seed}:placeholder`),
    abiIdentity: fixtureIdentity("skiff-package-abi-v1", `${seed}:placeholder`),
    publicationAbi: packagePublicationAbi(id, version, seed),
    files: [],
    implementationLinks: {
      types: {},
      constants: {},
      functions: {},
      implMethods: {},
    },
    dependencies: options.dependencies ?? [],
    configAndEffectMetadata: options.configAndEffectMetadata ?? {},
  };
  const identities = await computePackageUnitIdentities(value);
  value.buildIdentity = identities.buildIdentity;
  value.abiIdentity = identities.abiIdentity;
  const unitHash = artifactHash(value);
  const packageIdPathSegment = publicationStorageSegment(id);
  const unitPath = `units/packages/${packageIdPathSegment}/${unitHash}.json`;
  await mkdir(join(root, "units", "packages", packageIdPathSegment), {
    recursive: true,
  });
  await writeFile(join(root, unitPath), JSON.stringify(value, null, 2));
  return { buildIdentity: identities.buildIdentity, unitPath, value };
}

async function computePackageUnitIdentities(
  packageUnit: Record<string, unknown>,
): Promise<{ buildIdentity: string; abiIdentity: string }> {
  const cliPath =
    process.env.SKIFF_ARTIFACT_IDENTITY_CLI?.trim() || await ensureArtifactIdentityCli();
  const stdout = await runIdentityCli(cliPath, ["package-unit-identities"], {
    packageUnit,
  });
  const value = JSON.parse(stdout) as unknown;
  if (
    !isRecord(value) ||
    typeof value.buildIdentity !== "string" ||
    typeof value.abiIdentity !== "string"
  ) {
    throw new Error("artifact identity CLI returned invalid package identities");
  }
  return {
    buildIdentity: value.buildIdentity,
    abiIdentity: value.abiIdentity,
  };
}

function packagePublicationAbi(id: string, version: string, seed: string) {
  return {
    schemaVersion: "skiff-publication-abi-unit-v1",
    publicationId: id,
    version,
    abiIdentity: fixtureIdentity("skiff-publication-abi-v1", seed),
  };
}

async function writePackageUnitIndex(
  root: string,
  id: string,
  version: string,
  unit: WrittenPackageUnit,
  generation: string,
) {
  const path = join(
    root,
    "indexes",
    "packages",
    publicationStorageSegment(id),
    "versions",
  );
  await mkdir(path, { recursive: true });
  await writeFile(
    join(path, `${version}.json`),
    JSON.stringify(
      {
        schemaVersion: "skiff-package-version-index-v1",
        id,
        version,
        generation,
        packageUnit: {
          buildIdentity: unit.buildIdentity,
          unitPath: unit.unitPath,
        },
      },
      null,
      2,
    ),
  );
}

async function writePackageAssembly(
  root: string,
  id: string,
  version: string,
  dependencies: Array<{
    id: string;
    version: string;
    alias: string;
    assemblyIdentity: string;
  }> = [],
  options: {
    db?: unknown;
    configShape?: unknown;
    configUses?: unknown;
    configActivation?: unknown;
  } = {},
): Promise<WrittenPackageAssembly> {
  const hashInput = {
    schemaVersion: "skiff-assembly-v1",
    kind: "package",
    package: {
      id,
      version,
    },
    exports: { entries: [] },
    files: [],
    dependencies: {
      packages: dependencies,
    },
    ...(Object.prototype.hasOwnProperty.call(options, "db")
      ? { db: options.db }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(options, "configShape")
      ? { configShape: options.configShape }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(options, "configUses")
      ? { configUses: options.configUses }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(options, "configActivation")
      ? { configActivation: options.configActivation }
      : {}),
    sourceMap: null,
  };
  const hash = artifactHash(hashInput);
  const identity = `skiff-package-assembly-v1:sha256:${hash}`;
  const value = {
    ...hashInput,
    package: {
      ...hashInput.package,
      assemblyIdentity: identity,
    },
    publicAbi: {
      types: [],
      interfaces: [],
      functions: [],
      consts: [],
      providerFunctions: [],
      publicEffects: [],
      configUses: [],
    },
  };
  const path = `assemblies/packages/${id}/${hash}.json`;
  await mkdir(join(root, "assemblies", "packages", id), { recursive: true });
  await writeFile(join(root, path), JSON.stringify(value, null, 2));
  return { identity, path, value };
}

async function writeLegacyPackageAssemblyWithoutSourceMapHash(
  root: string,
  id: string,
  version: string,
): Promise<WrittenPackageAssembly> {
  const hashInput = {
    schemaVersion: "skiff-assembly-v1",
    kind: "package",
    package: {
      id,
      version,
    },
    exports: { entries: [] },
    files: [],
    dependencies: {
      packages: [],
    },
  };
  const hash = artifactHash(hashInput);
  const identity = `skiff-package-assembly-v1:sha256:${hash}`;
  const value = {
    ...hashInput,
    package: {
      ...hashInput.package,
      assemblyIdentity: identity,
    },
    publicAbi: {
      types: [],
      interfaces: [],
      functions: [],
      consts: [],
      providerFunctions: [],
      publicEffects: [],
      configUses: [],
    },
  };
  const path = `assemblies/packages/${id}/${hash}.json`;
  await mkdir(join(root, "assemblies", "packages", id), { recursive: true });
  await writeFile(join(root, path), JSON.stringify(value, null, 2));
  return { identity, path, value };
}

async function writePackageIndex(
  root: string,
  id: string,
  version: string,
  assembly: WrittenPackageAssembly,
) {
  const path = join(root, "indexes", "packages", id, "versions");
  await mkdir(path, { recursive: true });
  await writeFile(
    join(path, `${version}.json`),
    JSON.stringify(
      {
        schemaVersion: "skiff-package-version-index-v1",
        package: {
          id,
          version,
        },
        packageAssembly: {
          assemblyIdentity: assembly.identity,
          assemblyPath: assembly.path,
        },
      },
      null,
      2,
    ),
  );
}

function readPointerString(
  pointer: Record<string, unknown>,
  key: string,
): string | undefined {
  const value = pointer[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function serviceTestVersion(serviceId: string): string {
  const segments = serviceIdPathSegments(serviceId);
  const lastSegment = segments.at(-1);
  if (lastSegment === undefined) {
    throw new Error(
      `serviceId ${serviceId} must have at least one path segment`,
    );
  }
  return `${lastSegment}-test`;
}

function serviceIdJsonPath(
  root: string,
  prefix: string[],
  serviceId: string,
): string {
  const segments = serviceIdPathSegments(serviceId);
  const lastSegment = segments.at(-1);
  if (lastSegment === undefined) {
    throw new Error(
      `serviceId ${serviceId} must have at least one path segment`,
    );
  }
  return join(root, ...prefix, ...segments.slice(0, -1), `${lastSegment}.json`);
}

function serviceIdArtifactFilePath(
  prefix: string[],
  serviceId: string,
  fileStem: string,
): string {
  return join(
    ...prefix,
    ...serviceIdPathSegments(serviceId),
    `${fileStem}.json`,
  );
}

function serviceAssemblyArtifactPath(serviceId: string, hash: string): string {
  return serviceIdArtifactFilePath(["assemblies", "services"], serviceId, hash);
}

async function writeServiceAssemblyValue(
  root: string,
  serviceId: string,
  hash: string,
  assembly: Record<string, unknown>,
): Promise<string> {
  const path = serviceAssemblyArtifactPath(serviceId, hash);
  await mkdir(dirname(join(root, path)), { recursive: true });
  await writeFile(join(root, path), JSON.stringify(assembly, null, 2));
  return path;
}

function contractIdentityForService(serviceId: string): string {
  return serviceId === SERVICE_ID
    ? CONTRACT_IDENTITY
    : fixtureIdentity("skiff-protocol-v1", serviceId);
}

function configShape(_hash: string) {
  return {
    schemaVersion: "skiff-config-shape-v1",
    entries: [
      {
        path: "dashscopeModel",
        type: "string",
        required: true,
      },
    ],
  };
}

function fixtureIdentity(prefix: string, seed: string): string {
  return `${prefix}:sha256:${createHash("sha256").update(seed).digest("hex")}`;
}

function revisionIdFixture(seed: string): string {
  return createHash("sha256").update(seed).digest("hex");
}

function artifactHash(value: unknown): string {
  return createHash("sha256").update(stableStringify(value)).digest("hex");
}

function testOperationAbiId(target: string): string {
  return `operation:test:${target}`;
}

function operationAbiIdForServiceOperation(operation: string): string {
  return testOperationAbiId(`service:${operation}`);
}

function isRecord(value: unknown): value is Record<string, any> {
  return typeof value === "object" && value !== null;
}

function serviceAssemblyHashInput(assembly: Record<string, any>) {
  const service = assembly.service ?? {};
  const serviceInput: Record<string, any> = {
    id: service.id ?? null,
    revisionId: service.revisionId ?? null,
    protocolIdentity: service.protocolIdentity ?? null,
  };
  if (Object.prototype.hasOwnProperty.call(service, "access")) {
    serviceInput.access = service.access ?? null;
  }
  if (Object.prototype.hasOwnProperty.call(service, "api")) {
    serviceInput.api = service.api ?? null;
  }
  return {
    schemaVersion: assembly.schemaVersion ?? null,
    kind: assembly.kind ?? null,
    service: serviceInput,
    files: assembly.files ?? null,
    preludeIdentity: assembly.preludeIdentity ?? null,
    prelude: assembly.prelude ?? null,
    packageConfigs: assembly.packageConfigs ?? null,
    configShape: assembly.configShape ?? null,
    configUses: assembly.configUses ?? null,
    configActivation: assembly.configActivation ?? null,
    configRequirements: assembly.configRequirements ?? null,
    db: assembly.db ?? null,
    operations: assembly.operations ?? null,
    gateway: assembly.gateway ?? null,
    timeout: assembly.timeout ?? null,
    dependencyLock: assembly.dependencyLock ?? null,
    serviceUnit: assembly.serviceUnit ?? null,
    sourceMap: assembly.sourceMap ?? null,
  };
}

function serviceAssemblyHashInputWithoutSourceMap(
  assembly: Record<string, any>,
) {
  const input = serviceAssemblyHashInput(assembly);
  delete input.sourceMap;
  return input;
}

function serviceAssemblyHashInputWithoutDb(assembly: Record<string, any>) {
  const input = serviceAssemblyHashInput(assembly);
  delete input.db;
  return input;
}

function compilerServiceAssemblyHashInput(assembly: Record<string, any>) {
  const service = assembly.service ?? {};
  const input = serviceAssemblyHashInput(assembly);
  input.service = {
    id: service.id ?? null,
    revisionId: service.revisionId ?? null,
    protocolIdentity: service.protocolIdentity ?? null,
    ...(Object.prototype.hasOwnProperty.call(service, "access")
      ? { access: service.access ?? null }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(service, "api")
      ? { api: service.api ?? null }
      : {}),
    ...serviceHttpHashInput(service),
  };
  return input;
}

function serviceHttpHashInput(service: Record<string, any>) {
  return service.http?.response && "maxBytes" in service.http.response
    ? {
        http: {
          response: {
            maxBytes: service.http.response.maxBytes,
          },
        },
      }
    : {};
}

function identityHash(identity: string): string {
  const marker = ":sha256:";
  const index = identity.lastIndexOf(marker);
  return index === -1 ? identity : identity.slice(index + marker.length);
}

function testServiceRouteTarget(serviceId: string, operation: string): string {
  return `runtime.${publicationStorageSegment(serviceId)}.${operation}`;
}

function testServiceImplementation(serviceId: string, operation: string) {
  const [receiverType, name] = operation.split(".");
  return {
    modulePath: `runtime.${publicationStorageSegment(serviceId)}`,
    symbol: `${receiverType}.${name}`,
    receiver: { type: receiverType, binding: "self" },
  };
}

function serviceAssembly(
  serviceId: string,
  options: { assemblyIdentity?: string } = {},
) {
  return {
    schemaVersion: "skiff-assembly-v1",
    kind: "service",
    service: {
      id: serviceId,
      revisionId: revisionIdFixture(`${serviceId}:service-assembly`),
      protocolIdentity: contractIdentityForService(serviceId),
      assemblyIdentity: options.assemblyIdentity ?? ASSEMBLY_IDENTITY,
    },
    files: [
      {
        sourcePath: "api/connection.skiff",
        modulePath: "api.connection",
        fileIrIdentity: CONTRACT_FILE_IR_IDENTITY,
        fileIrHash: CONTRACT_FILE_IR_HASH,
        fileIrPath: CONTRACT_FILE_IR_PATH,
        role: "contract",
      },
    ],
    operations: [
      {
        operation: "WebSocketFixtureConnection.connect",
        operationAbiId: operationAbiIdForServiceOperation(
          "WebSocketFixtureConnection.connect",
        ),
        mode: "unary",
        entrypoint: testServiceRouteTarget(
          serviceId,
          "WebSocketFixtureConnection.connect",
        ),
        implementation: testServiceImplementation(
          serviceId,
          "WebSocketFixtureConnection.connect",
        ),
        parameters: [
          {
            name: "input",
            schema: connectionConnectInputSchema(),
          },
        ],
        response: gatewayConnectResultSchema(connectionContextSchema()),
      },
      {
        operation: "WebSocketFixtureConnection.receive",
        operationAbiId: operationAbiIdForServiceOperation(
          "WebSocketFixtureConnection.receive",
        ),
        mode: "unary",
        entrypoint: testServiceRouteTarget(
          serviceId,
          "WebSocketFixtureConnection.receive",
        ),
        implementation: testServiceImplementation(
          serviceId,
          "WebSocketFixtureConnection.receive",
        ),
        parameters: [
          {
            name: "context",
            schema: connectionContextSchema(),
          },
          {
            name: "message",
            schema: connectionMessageSchema(),
          },
        ],
        response: { type: "null" },
      },
      {
        operation: "WebSocketFixtureHttpApi.handle",
        operationAbiId: operationAbiIdForServiceOperation(
          "WebSocketFixtureHttpApi.handle",
        ),
        mode: "unary",
        entrypoint: testServiceRouteTarget(
          serviceId,
          "WebSocketFixtureHttpApi.handle",
        ),
        implementation: testServiceImplementation(
          serviceId,
          "WebSocketFixtureHttpApi.handle",
        ),
        parameters: [
          {
            name: "request",
            schema: httpRequestSchema(),
          },
        ],
        response: httpResponseSchema(),
      },
    ],
    gateway: {
      http: {
        raw: {
          operation: "WebSocketFixtureHttpApi.handle",
          target: `gateway.${publicationStorageSegment(serviceId)}.http.raw`,
        },
      },
      websocket: {
        id: "client",
        path: "/ws",
        serviceParam: "service",
        context: connectionContextSchema(),
        connect: {
          operation: "WebSocketFixtureConnection.connect",
          operationAbiId: operationAbiIdForServiceOperation(
            "WebSocketFixtureConnection.connect",
          ),
          adapterArgs: [
            { param: "input", source: { kind: "websocket.connectRequest" } },
          ],
        },
        receive: {
          operation: "WebSocketFixtureConnection.receive",
          operationAbiId: operationAbiIdForServiceOperation(
            "WebSocketFixtureConnection.receive",
          ),
          adapterArgs: [
            { param: "context", source: { kind: "websocket.connectionContext" } },
            { param: "message", source: { kind: "websocket.message" } },
          ],
        },
      },
    },
    packages: {},
    timeout: null,
    dependencyLock: [],
  };
}

function runtimeProgramServiceAssembly(serviceId: string) {
  return {
    schemaVersion: "skiff-assembly-v1",
    kind: "service",
    service: {
      id: serviceId,
      revisionId: revisionIdFixture(
        `${serviceId}:runtime-program-service-assembly`,
      ),
      protocolIdentity: contractIdentityForService(serviceId),
      assemblyIdentity: ASSEMBLY_IDENTITY,
    },
    files: [
      {
        sourcePath: "internal/service.skiff",
        modulePath: "svc.main",
        fileIrIdentity: CONTRACT_FILE_IR_IDENTITY,
        fileIrHash: CONTRACT_FILE_IR_HASH,
        fileIrPath: CONTRACT_FILE_IR_PATH,
        role: "implementation",
      },
    ],
    operations: [
      {
        operation: "Api.hello",
        operationAbiId: operationAbiIdForServiceOperation("Api.hello"),
        mode: "unary",
        entrypoint: "svc.main.Api.hello",
        implementation: {
          modulePath: "svc.main",
          symbol: "Api.hello",
          receiver: { type: "Api", binding: "self" },
        },
        parameters: [],
        response: { type: "object", additionalProperties: true },
      },
    ],
    gateway: {},
  };
}

function connectionConnectInputSchema() {
  return {
    type: "object",
    properties: {
      deviceId: { type: "string" },
    },
    required: ["deviceId"],
    additionalProperties: false,
  };
}

function connectionContextSchema() {
  return {
    type: "object",
    properties: {
      userId: { type: "string" },
    },
    required: ["userId"],
    additionalProperties: false,
  };
}

function connectionMessageSchema() {
  return {
    oneOf: [
      {
        type: "object",
        required: ["tag", "text"],
        properties: {
          tag: { type: "string", enum: ["text"] },
          text: { type: "string" },
        },
        additionalProperties: false,
      },
      {
        type: "object",
        required: ["tag", "base64"],
        properties: {
          tag: { type: "string", enum: ["binary"] },
          base64: { type: "string" },
        },
        additionalProperties: false,
      },
    ],
  };
}

function webSocketConnectRequestSchema() {
  return {
    type: "object",
    required: ["connectionId", "url", "query", "headers", "cookies"],
    properties: {
      connectionId: { type: "string" },
      url: { type: "string" },
      query: { type: "array", items: httpHeaderSchema() },
      headers: { type: "array", items: httpHeaderSchema() },
      cookies: { type: "array", items: httpHeaderSchema() },
      version: { type: "string", nullable: true },
    },
    additionalProperties: false,
  };
}

function gatewayConnectResultSchema(context: Record<string, unknown>) {
  return {
    oneOf: [
      {
        type: "object",
        required: ["tag", "context"],
        properties: {
          tag: { type: "string", enum: ["accept"] },
          context,
          businessIdentity: { type: "string", nullable: true },
          connectionPolicy: websocketConnectionPolicySchema(),
        },
        additionalProperties: false,
      },
      {
        type: "object",
        required: ["tag", "code", "reason"],
        properties: {
          tag: { type: "string", enum: ["reject"] },
          code: { type: "integer" },
          reason: { type: "string" },
        },
        additionalProperties: false,
      },
    ],
  };
}

function websocketConnectionPolicySchema() {
  return {
    type: "object",
    required: ["maxConnections", "overflow"],
    properties: {
      maxConnections: { type: "integer" },
      overflow: { type: "string" },
      closeCode: { type: "integer" },
      closeReason: { type: "string" },
    },
    additionalProperties: false,
  };
}

function httpHeaderSchema() {
  return {
    type: "object",
    required: ["name", "value"],
    properties: {
      name: { type: "string" },
      value: { type: "string" },
    },
    additionalProperties: false,
  };
}

function httpBodySchema() {
  return {
    type: "string",
    contentEncoding: "base64",
    xSkiffSymbol: "std.bytes.bytes",
  };
}

function httpRequestSchema() {
  return {
    type: "object",
    required: ["method", "url", "path", "query", "headers", "body"],
    properties: {
      method: { type: "string" },
      url: { type: "string" },
      path: { type: "string" },
      query: { type: "array", items: httpHeaderSchema() },
      headers: { type: "array", items: httpHeaderSchema() },
      body: httpBodySchema(),
    },
    additionalProperties: false,
  };
}

function httpResponseSchema() {
  return {
    type: "object",
    required: ["status", "headers", "body"],
    properties: {
      status: { type: "integer" },
      headers: { type: "array", items: httpHeaderSchema() },
      body: httpBodySchema(),
    },
    additionalProperties: false,
  };
}

function fileIrUnitTypes(fileIrIdentity: string) {
  return {
    schemaVersion: "skiff-file-ir-v3",
    fileIrIdentity,
    sourceAstHash: fixtureIdentity(
      "skiff-source-ast-v1",
      "contract-file-source",
    ),
    modulePath: "api.connection",
    sourceMap: {},
    declarations: {
      types: [
        {
          kind: "record",
          name: "ConnectionConnectInput",
          fields: [
            {
              name: "deviceId",
              type: { kind: "builtin", name: "string" },
            },
          ],
        },
        {
          kind: "record",
          name: "ConnectionContext",
          fields: [
            {
              name: "userId",
              type: { kind: "builtin", name: "string" },
            },
          ],
        },
      ],
    },
    exports: {},
    typeTable: [],
    executables: [],
    externalRefs: {},
  };
}
