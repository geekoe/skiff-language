import { sha256Hex, stableStringify } from "../manifest/identity.js";
import type { ArtifactPointer } from "./types.js";
import { readOptionalRecord } from "./readUtils.js";
import { serviceIdPathSegments } from "./pathProjection.js";
import { serviceHttpHashInput } from "./serviceHttp.js";

const SERVICE_ASSEMBLY_IDENTITY_PREFIX = "skiff-service-assembly-v1";
const PROTOCOL_IDENTITY_PREFIX = "skiff-protocol-v1";
const SERVICE_BUILD_ID_PATTERN =
  /^skiff-service-build-v1:sha256:([0-9a-f]{64})$/;

export function validateDevReloadContractHash(
  contractHash: string,
  protocolIdentity: string,
  pointerPath: string,
): void {
  const hash = contractHash.startsWith("sha256:")
    ? contractHash.slice("sha256:".length)
    : contractHash;
  if (!isSha256Hash(hash)) {
    throw new Error(
      `${pointerPath} contractHash must be sha256:<64 lowercase hex> or <64 lowercase hex>`,
    );
  }
  const expectedHash = identityHash(protocolIdentity);
  if (hash !== expectedHash) {
    throw new Error(
      `${pointerPath} contractHash ${contractHash} does not match protocolIdentity hash ${expectedHash}`,
    );
  }
}

export function validateServiceAssemblyIdentity(
  pointer: ArtifactPointer,
  assemblyIdentity: string | undefined,
): string {
  const indexIdentity = pointer.serviceAssemblyIdentity;
  if (indexIdentity !== undefined && assemblyIdentity !== undefined) {
    validateIdentityAlias(
      assemblyIdentity,
      indexIdentity,
      "service.assemblyIdentity",
    );
  }

  const identity = indexIdentity ?? assemblyIdentity;
  if (identity === undefined) {
    throw new Error(
      `${pointer.indexPath} serviceAssembly requires assemblyIdentity in index serviceAssembly or assembly service.assemblyIdentity`,
    );
  }
  validateIdentityPrefix(
    identity,
    SERVICE_ASSEMBLY_IDENTITY_PREFIX,
    "serviceAssembly",
  );
  return identity;
}

export function validateServiceAssemblyContentIdentity(
  assembly: Record<string, unknown>,
  assemblyIdentity: string,
  indexPath: string,
): void {
  const expectedHash = identityHashWithLabel(
    assemblyIdentity,
    "serviceAssembly",
  );
  const actualHash = sha256Hex(
    stableStringify(serviceAssemblyHashInput(assembly)),
  );
  if (actualHash !== expectedHash) {
    throw new Error(
      `${indexPath} serviceAssembly content sha256 ${actualHash} does not match assemblyIdentity hash ${expectedHash}`,
    );
  }
}

export function serviceAssemblyHashInput(
  assembly: Record<string, unknown>,
): Record<string, unknown> {
  return {
    schemaVersion: assembly.schemaVersion ?? null,
    kind: assembly.kind ?? null,
    service: serviceHashInput(assembly),
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

function serviceHashInput(
  assembly: Record<string, unknown>,
): Record<string, unknown> {
  const service = readOptionalRecord(assembly.service) ?? {};
  const input: Record<string, unknown> = {
    id: service.id ?? null,
    revisionId: service.revisionId ?? null,
    protocolIdentity: service.protocolIdentity ?? null,
  };
  if ("access" in service) {
    input.access = service.access ?? null;
  }
  if ("api" in service) {
    input.api = service.api ?? null;
  }
  const http = serviceHttpHashInput(service, "serviceAssembly.service");
  if (http) {
    input.http = http.http;
  }
  return input;
}

export function validateServiceAssemblyPathIdentity(
  artifactPath: string,
  serviceId: string,
  assemblyIdentity: string,
  indexPath: string,
): void {
  const parts = artifactPath.split(/[\\/]/);
  if (
    parts.length < 4 ||
    parts[0] !== "assemblies" ||
    parts[1] !== "services"
  ) {
    return;
  }
  const fileName = parts.at(-1)!;
  if (!fileName.endsWith(".json")) {
    throw new Error(
      `${indexPath} serviceAssembly path ${artifactPath} must end with .json`,
    );
  }
  const stem = fileName.slice(0, -".json".length);
  const expectedServiceIdSegments = serviceIdPathSegments(serviceId);
  const pathServiceIdSegments = parts.slice(2, -1);
  if (!sameSegments(pathServiceIdSegments, expectedServiceIdSegments)) {
    throw new Error(
      `${indexPath} serviceAssembly path ${artifactPath} service id path ${pathServiceIdSegments.join("/")} does not match index serviceId ${serviceId}`,
    );
  }
  const identityHash = identityHashWithLabel(
    assemblyIdentity,
    "serviceAssembly",
  );
  if (stem !== identityHash) {
    throw new Error(
      `${indexPath} serviceAssembly path ${artifactPath} identity hash ${stem} does not match assemblyIdentity hash ${identityHash}`,
    );
  }
}

export function identityHash(identity: string): string {
  const marker = ":sha256:";
  const index = identity.lastIndexOf(marker);
  if (index === -1) {
    throw new Error(`contractIdentity must include ${marker}`);
  }
  const prefix = identity.slice(0, index);
  if (prefix !== PROTOCOL_IDENTITY_PREFIX) {
    throw new Error(
      `contractIdentity prefix must be ${PROTOCOL_IDENTITY_PREFIX}, got ${prefix}`,
    );
  }
  const hash = identity.slice(index + marker.length);
  if (!isSha256Hash(hash)) {
    throw new Error(
      "contractIdentity sha256 hash must be 64 lowercase hex characters",
    );
  }
  return hash;
}

export function identityHashWithLabel(identity: string, label: string): string {
  const marker = ":sha256:";
  const index = identity.lastIndexOf(marker);
  if (index === -1) {
    throw new Error(`${label} identity must include ${marker}`);
  }
  const hash = identity.slice(index + marker.length);
  if (!isSha256Hash(hash)) {
    throw new Error(
      `${label} identity sha256 hash must be 64 lowercase hex characters`,
    );
  }
  return hash;
}

export function serviceBuildIdHash(buildId: string, label: string): string {
  const match = SERVICE_BUILD_ID_PATTERN.exec(buildId);
  if (!match) {
    throw new Error(
      `${label} must be skiff-service-build-v1:sha256:<64 lowercase hex>`,
    );
  }
  return match[1]!;
}

function validateIdentityAlias(
  alias: string,
  expected: string,
  label: string,
): void {
  if (alias !== expected) {
    throw new Error(
      `${label} ${alias} does not match serviceAssembly assemblyIdentity ${expected}`,
    );
  }
}

function validateIdentityPrefix(
  identity: string,
  expectedPrefix: string,
  label: string,
): void {
  const marker = ":sha256:";
  const index = identity.lastIndexOf(marker);
  if (index === -1) {
    throw new Error(`${label} assemblyIdentity must include ${marker}`);
  }
  const prefix = identity.slice(0, index);
  if (prefix !== expectedPrefix) {
    throw new Error(
      `${label} assemblyIdentity prefix must be ${expectedPrefix}, got ${prefix}`,
    );
  }
  const hash = identity.slice(index + marker.length);
  if (!isSha256Hash(hash)) {
    throw new Error(
      `${label} identity sha256 hash must be 64 lowercase hex characters`,
    );
  }
}

function isSha256Hash(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

function sameSegments(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((segment, index) => segment === right[index])
  );
}
