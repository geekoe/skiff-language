const PROTOCOL_IDENTITY_PREFIX = "skiff-service-protocol-v2";
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

function isSha256Hash(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}
