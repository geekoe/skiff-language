// Strict reader for one immutable actor routing projection record (A0 §2,
// A3-aligned). This module is the TS Router's only production source of actor
// method catalog facts. It never reads PackageArtifact / File IR / source /
// executable payload: the frozen projection shape rejects those coordinates at
// the typed boundary, and the loader only opens `records/actor-routing/current.json`.

import { parseStrictJson } from '../protocol/strictJson.js';

export const ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION =
  'skiff-actor-routing-projection-v1';
export const ACTOR_ROUTING_PROJECTION_RECORD_PATH =
  'records/actor-routing/current.json';

// Aligned with the A3 per-record budget.
export const MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES = 16 * 1024 * 1024;

const ACTOR_ABI_IDENTITY_PREFIX = 'skiff-actor-abi-v1:sha256:';
const ACTOR_IMPLEMENTATION_IDENTITY_PREFIX =
  'skiff-actor-implementation-v1:sha256:';
const ACTOR_METHOD_IDENTITY_PREFIX = 'skiff-actor-method-v1:sha256:';
const DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX =
  'skiff-deployment-artifact-v4:sha256:';
const PACKAGE_BUILD_IDENTITY_PREFIX = 'skiff-package-build-v10:sha256:';
const PACKAGE_LOCAL_ABI_IDENTITY_PREFIX =
  'skiff-package-local-abi-v7:sha256:';

export interface ActorRoutingProjectionRef {
  serviceId: string;
  actorAbiIdentity: string;
}

export interface ActorRoutingDeploymentRef {
  serviceId: string;
  contractVersion: string;
  deploymentRevision: string;
  deploymentArtifactIdentity: string;
}

export interface ActorRoutingPackageRef {
  packageId: string;
  packageVersion: string;
  packageBuildId: string;
  packageLocalAbiIdentity: string;
}

export interface ActorRoutingMethod {
  actor: ActorRoutingProjectionRef;
  actorImplementationIdentity: string;
  methodIdentity: string;
  deployment: ActorRoutingDeploymentRef;
  package: ActorRoutingPackageRef;
}

export interface ActorRoutingProjection {
  schemaVersion: string;
  methods: ActorRoutingMethod[];
}

/** Fail-closed result of decoding one projection record. */
export type ActorRoutingProjectionFailure =
  | 'SchemaVersion'
  | 'Malformed'
  | 'NonCanonical'
  | 'Invalid'
  | 'TooLarge';

export class ActorRoutingProjectionError extends Error {
  constructor(
    readonly failure: ActorRoutingProjectionFailure,
    message: string
  ) {
    super(message);
    this.name = 'ActorRoutingProjectionError';
  }
}

/**
 * Strictly decodes one projection record.
 *
 * Chain (A3 `ActorRoutingProjectionStore::load` parity): strict JSON with
 * duplicate-key rejection → exact schema version → typed surface validation
 * (`deny_unknown_fields` + identity prefixes + serviceId consistency + sorted
 * unique entries) → canonical JSON bytes equality.
 */
export function decodeActorRoutingProjectionRecord(
  bytes: Uint8Array
): ActorRoutingProjection {
  let value: unknown;
  try {
    value = parseStrictJson(bytes);
  } catch (error) {
    throw new ActorRoutingProjectionError(
      'Malformed',
      `actor routing projection record is not strict JSON: ${
        error instanceof Error ? error.message : String(error)
      }`
    );
  }
  const root = exactObject(
    value,
    'actor routing projection record'
  );
  exactFields(
    root,
    ['schemaVersion', 'methods'],
    'actor routing projection record'
  );
  const schemaVersion = requiredString(root.schemaVersion, 'schemaVersion');
  if (schemaVersion !== ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION) {
    throw new ActorRoutingProjectionError(
      'SchemaVersion',
      `actor routing projection record has unsupported schemaVersion ${JSON.stringify(schemaVersion)}`
    );
  }
  if (!Array.isArray(root.methods)) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      'actor routing projection record methods must be an array'
    );
  }
  const methods = root.methods.map((raw, index) =>
    decodeMethod(raw, `actor routing projection record methods[${index}]`)
  );
  assertUniqueMethods(methods);
  if (!isSortedMethods(methods)) {
    throw new ActorRoutingProjectionError(
      'NonCanonical',
      'actor routing projection record methods are not sorted by the full typed key'
    );
  }
  const canonical = canonicalJsonBytes(value);
  if (!bytesEqual(canonical, bytes)) {
    throw new ActorRoutingProjectionError(
      'NonCanonical',
      'actor routing projection record is not canonical JSON'
    );
  }
  return { schemaVersion, methods };
}

function decodeMethod(raw: unknown, label: string): ActorRoutingMethod {
  const value = exactObject(raw, label);
  exactFields(
    value,
    [
      'actor',
      'actorImplementationIdentity',
      'methodIdentity',
      'deployment',
      'package',
    ],
    label
  );
  const actor = decodeActor(value.actor, `${label}.actor`);
  const deployment = decodeDeployment(
    value.deployment,
    `${label}.deployment`
  );
  if (actor.serviceId !== deployment.serviceId) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      `${label} actor.serviceId must match its deployment.serviceId`
    );
  }
  const method = {
    actor,
    actorImplementationIdentity: framedIdentity(
      value.actorImplementationIdentity,
      ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
      `${label}.actorImplementationIdentity`
    ),
    methodIdentity: framedIdentity(
      value.methodIdentity,
      ACTOR_METHOD_IDENTITY_PREFIX,
      `${label}.methodIdentity`
    ),
    deployment,
    package: decodePackage(value.package, `${label}.package`),
  };
  return method;
}

function decodeActor(raw: unknown, label: string): ActorRoutingProjectionRef {
  const value = exactObject(raw, label);
  exactFields(value, ['serviceId', 'actorAbiIdentity'], label);
  return {
    serviceId: nonEmptyString(value.serviceId, `${label}.serviceId`),
    actorAbiIdentity: framedIdentity(
      value.actorAbiIdentity,
      ACTOR_ABI_IDENTITY_PREFIX,
      `${label}.actorAbiIdentity`
    ),
  };
}

function decodeDeployment(
  raw: unknown,
  label: string
): ActorRoutingDeploymentRef {
  const value = exactObject(raw, label);
  exactFields(
    value,
    [
      'serviceId',
      'contractVersion',
      'deploymentRevision',
      'deploymentArtifactIdentity',
    ],
    label
  );
  return {
    serviceId: nonEmptyString(value.serviceId, `${label}.serviceId`),
    contractVersion: nonEmptyString(
      value.contractVersion,
      `${label}.contractVersion`
    ),
    deploymentRevision: nonEmptyString(
      value.deploymentRevision,
      `${label}.deploymentRevision`
    ),
    deploymentArtifactIdentity: framedIdentity(
      value.deploymentArtifactIdentity,
      DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
      `${label}.deploymentArtifactIdentity`
    ),
  };
}

function decodePackage(
  raw: unknown,
  label: string
): ActorRoutingPackageRef {
  const value = exactObject(raw, label);
  exactFields(
    value,
    [
      'packageId',
      'packageVersion',
      'packageBuildId',
      'packageLocalAbiIdentity',
    ],
    label
  );
  return {
    packageId: nonEmptyString(value.packageId, `${label}.packageId`),
    packageVersion: nonEmptyString(
      value.packageVersion,
      `${label}.packageVersion`
    ),
    packageBuildId: framedIdentity(
      value.packageBuildId,
      PACKAGE_BUILD_IDENTITY_PREFIX,
      `${label}.packageBuildId`
    ),
    packageLocalAbiIdentity: framedIdentity(
      value.packageLocalAbiIdentity,
      PACKAGE_LOCAL_ABI_IDENTITY_PREFIX,
      `${label}.packageLocalAbiIdentity`
    ),
  };
}

function assertUniqueMethods(methods: ActorRoutingMethod[]): void {
  const seen = new Set<string>();
  for (const method of methods) {
    const key = fullTypedKey(method);
    if (seen.has(key)) {
      throw new ActorRoutingProjectionError(
        'Invalid',
        'actor routing projection record contains duplicate method entries'
      );
    }
    seen.add(key);
  }
}

function isSortedMethods(methods: ActorRoutingMethod[]): boolean {
  for (let index = 1; index < methods.length; index += 1) {
    const previous = methods[index - 1]!;
    const current = methods[index]!;
    if (compareMethods(previous, current) >= 0) return false;
  }
  return true;
}

function compareMethods(left: ActorRoutingMethod, right: ActorRoutingMethod): number {
  return compareStrings(
    fullTypedKey(left),
    fullTypedKey(right)
  );
}

/**
 * Full typed key in the exact A0 entry field order. Sorting is a pure function
 * of these fields, so the typed key comparison is the same total order used by
 * the frozen `ActorRoutingProjection::new`.
 */
function fullTypedKey(method: ActorRoutingMethod): string {
  return [
    method.actor.serviceId,
    method.actor.actorAbiIdentity,
    method.actorImplementationIdentity,
    method.methodIdentity,
    method.deployment.serviceId,
    method.deployment.contractVersion,
    method.deployment.deploymentRevision,
    method.deployment.deploymentArtifactIdentity,
    method.package.packageId,
    method.package.packageVersion,
    method.package.packageBuildId,
    method.package.packageLocalAbiIdentity,
  ].join('\u0000');
}

function compareStrings(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function exactObject(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      `${label} must be an object`
    );
  }
  return value as Record<string, unknown>;
}

function exactFields(
  value: Record<string, unknown>,
  fields: readonly string[],
  label: string
): void {
  const actual = Object.keys(value).sort();
  const expected = [...fields].sort();
  if (actual.join(',') !== expected.join(',')) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      `${label} fields must be exactly ${expected.join(',')}`
    );
  }
}

function requiredString(value: unknown, label: string): string {
  const result = nonEmptyString(value, label);
  return result;
}

function nonEmptyString(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      `${label} must be a non-empty string`
    );
  }
  return value;
}

function framedIdentity(
  value: unknown,
  prefix: string,
  label: string
): string {
  if (typeof value !== 'string' || !value.startsWith(prefix)) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      `${label} is invalid`
    );
  }
  const hex = value.slice(prefix.length);
  if (!/^[0-9a-fA-F]{64}$/.test(hex)) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      `${label} is invalid`
    );
  }
  return value;
}

/**
 * Serializes a parsed JSON value to the same canonical bytes produced by
 * `skiff-canonical-json::canonical_json_bytes`: recursive key sorting, integral
 * number normalization and serde_json-compatible escaping.
 *
 * The frozen projection value domain contains no non-integral floats; any such
 * value fails closed rather than risking a byte-level divergence from the Rust
 * canonical encoder.
 */
export function canonicalJsonBytes(value: unknown): Uint8Array {
  return Buffer.from(canonicalJsonString(value), 'utf8');
}

function canonicalJsonString(value: unknown): string {
  if (value === null) return 'null';
  if (value === true) return 'true';
  if (value === false) return 'false';
  if (typeof value === 'string') return `"${canonicalEscape(value)}"`;
  if (typeof value === 'number') return canonicalNumber(value);
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJsonString).join(',')}]`;
  }
  if (typeof value === 'object') {
    const record = value as Record<string, unknown>;
    const keys = Object.keys(record).sort();
    return `{${keys
      .map((key) => `"${canonicalEscape(key)}":${canonicalJsonString(record[key])}`)
      .join(',')}}`;
  }
  throw new ActorRoutingProjectionError(
    'Invalid',
    'actor routing projection record contains a non-JSON value'
  );
}

function canonicalNumber(value: number): string {
  if (Object.is(value, -0)) return '0';
  if (!Number.isFinite(value)) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      'actor routing projection record contains a non-finite number'
    );
  }
  if (!Number.isInteger(value) || !Number.isSafeInteger(value)) {
    throw new ActorRoutingProjectionError(
      'Invalid',
      'actor routing projection record contains a non-canonical number'
    );
  }
  return String(value);
}

function canonicalEscape(value: string): string {
  let result = '';
  for (const character of value) {
    const code = character.codePointAt(0)!;
    if (character === '"') {
      result += '\\"';
    } else if (character === '\\') {
      result += '\\\\';
    } else if (code === 0x08) {
      result += '\\b';
    } else if (code === 0x0c) {
      result += '\\f';
    } else if (code === 0x0a) {
      result += '\\n';
    } else if (code === 0x0d) {
      result += '\\r';
    } else if (code === 0x09) {
      result += '\\t';
    } else if (code < 0x20) {
      result += `\\u${code.toString(16).padStart(4, '0')}`;
    } else {
      result += character;
    }
  }
  return result;
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
  if (left.byteLength !== right.byteLength) return false;
  for (let index = 0; index < left.byteLength; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}
