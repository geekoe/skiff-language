import { createHash } from 'node:crypto';

import { stableStringify } from '../manifest/identity.js';
import {
  deriveCurrentRuntimeAssemblyGatewayEntryIdentity
} from './runtimeAssemblyDeploymentIdentity.js';
import type {
  RuntimeAssemblyDeploymentRef,
  RuntimeAssemblyWebSocketIngressSelector
} from './runtimeAssemblySnapshot.js';

const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$/;
const WEBSOCKET_ENTRY_IDENTITY_PREFIX =
  'skiff-websocket-entry-v1:sha256:';
const WEBSOCKET_GATEWAY_ENTRY_KEY = 'websocket';

export type RuntimeAssemblyWebSocketRpcProfile = 'jsonrpc-2.0-text';

export interface RuntimeAssemblyWebSocketMethodBinding {
  readonly method: string;
  readonly profile: RuntimeAssemblyWebSocketRpcProfile;
  readonly deployment: RuntimeAssemblyDeploymentRef;
  readonly gatewayEntryKey: string;
  readonly gatewayEntryIdentity: string;
  readonly handler: string;
  readonly websocketEntryId: string;
}

export class RuntimeAssemblyWebSocketMethodTable {
  readonly #bindings: ReadonlyMap<string, RuntimeAssemblyWebSocketMethodBinding>;

  constructor(bindings: readonly RuntimeAssemblyWebSocketMethodBinding[]) {
    const byMethod = new Map<string, RuntimeAssemblyWebSocketMethodBinding>();
    for (const binding of bindings) {
      if (byMethod.has(binding.method)) {
        throw new Error(
          `WebSocket snapshot contains duplicate JSON-RPC method ${JSON.stringify(binding.method)}`
        );
      }
      byMethod.set(binding.method, freezeMethodBinding(binding));
    }
    this.#bindings = byMethod;
  }

  get size(): number {
    return this.#bindings.size;
  }

  capture(): ReadonlyMap<string, RuntimeAssemblyWebSocketMethodBinding> {
    return new Map(
      Array.from(this.#bindings, ([method, binding]) => [
        method,
        freezeMethodBinding(binding)
      ])
    );
  }

  clone(): RuntimeAssemblyWebSocketMethodTable {
    return new RuntimeAssemblyWebSocketMethodTable(
      Array.from(this.#bindings.values())
    );
  }
}

export interface DecodedRuntimeAssemblyWebSocketPhysicalEntry {
  readonly kind: 'websocketConnect';
  readonly gatewayEntryIdentity: string;
  readonly handler?: string;
  readonly rpcProfiles: readonly RuntimeAssemblyWebSocketRpcProfile[];
  readonly websocketEntryId: string;
}

export interface DecodedRuntimeAssemblyWebSocketMethodEntry {
  readonly kind: 'websocketJsonRpc';
  readonly gatewayEntryIdentity: string;
  readonly handler: string;
  readonly profile: RuntimeAssemblyWebSocketRpcProfile;
}

export type DecodedRuntimeAssemblyWebSocketEntry =
  | DecodedRuntimeAssemblyWebSocketPhysicalEntry
  | DecodedRuntimeAssemblyWebSocketMethodEntry;

export function decodeRuntimeAssemblyWebSocketGatewayEntry(input: {
  value: unknown;
  label: string;
  serviceId: string;
  implementationPackageId: string;
  gatewayEntryKey: string;
}): DecodedRuntimeAssemblyWebSocketEntry {
  const { label, serviceId, implementationPackageId, gatewayEntryKey } = input;
  const value = exactObject(input.value, label);
  exactFields(
    value,
    [
      'gatewayEntryIdentity',
      'protocolSurface',
      'handler',
      'pre',
      'guard',
      'adapterPlan'
    ],
    label
  );
  const gatewayEntryIdentity = requiredString(
    value,
    'gatewayEntryIdentity',
    label
  );
  if (!GATEWAY_ENTRY_IDENTITY_PATTERN.test(gatewayEntryIdentity)) {
    throw new Error(`${label}.gatewayEntryIdentity is invalid`);
  }
  const protocolSurface = exactObject(
    value.protocolSurface,
    `${label}.protocolSurface`
  );
  exactFields(
    protocolSurface,
    ['protocol', 'externalErrorProjection'],
    `${label}.protocolSurface`
  );
  decodeFixedErrorProjection(
    protocolSurface.externalErrorProjection,
    `${label}.protocolSurface.externalErrorProjection`
  );
  const protocol = exactObject(
    protocolSurface.protocol,
    `${label}.protocolSurface.protocol`
  );
  exactFields(protocol, ['kind', 'surface'], `${label}.protocolSurface.protocol`);
  let decoded: DecodedRuntimeAssemblyWebSocketEntry;
  if (protocol.kind === 'websocketConnect') {
    decoded = decodePhysicalEntry({
      value,
      surface: protocol.surface,
      label,
      serviceId,
      implementationPackageId,
      gatewayEntryKey,
      gatewayEntryIdentity
    });
  } else if (protocol.kind === 'websocketJsonRpc') {
    decoded = decodeMethodEntry({
      value,
      surface: protocol.surface,
      label,
      implementationPackageId,
      gatewayEntryKey,
      gatewayEntryIdentity
    });
  } else {
    throw new Error(`${label}.protocolSurface protocol kind is not WebSocket`);
  }
  const computedIdentity =
    deriveCurrentRuntimeAssemblyGatewayEntryIdentity(protocolSurface);
  if (gatewayEntryIdentity !== computedIdentity) {
    throw new Error(`${label}.gatewayEntryIdentity does not match its current surface`);
  }
  return decoded;
}

export function isRuntimeAssemblyWebSocketProtocolKind(
  input: unknown
): boolean {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    return false;
  }
  const protocolSurface = (input as Record<string, unknown>).protocolSurface;
  if (
    protocolSurface === null ||
    typeof protocolSurface !== 'object' ||
    Array.isArray(protocolSurface)
  ) {
    return false;
  }
  const protocol = (protocolSurface as Record<string, unknown>).protocol;
  if (protocol === null || typeof protocol !== 'object' || Array.isArray(protocol)) {
    return false;
  }
  const kind = (protocol as Record<string, unknown>).kind;
  return kind === 'websocketConnect' || kind === 'websocketJsonRpc';
}

export function deriveWebSocketEntryId(
  serviceId: string,
  gatewayEntryKey: string
): string {
  const preimage = stableStringify({
    gatewayEntryKey,
    schema: 'skiff-websocket-entry-identity-v1',
    serviceId
  });
  return `${WEBSOCKET_ENTRY_IDENTITY_PREFIX}${createHash('sha256')
    .update(preimage)
    .digest('hex')}`;
}

function decodePhysicalEntry(input: {
  value: Record<string, unknown>;
  surface: unknown;
  label: string;
  serviceId: string;
  implementationPackageId: string;
  gatewayEntryKey: string;
  gatewayEntryIdentity: string;
}): DecodedRuntimeAssemblyWebSocketPhysicalEntry {
  const {
    value,
    label,
    serviceId,
    implementationPackageId,
    gatewayEntryKey,
    gatewayEntryIdentity
  } = input;
  if (gatewayEntryKey !== WEBSOCKET_GATEWAY_ENTRY_KEY) {
    throw new Error(
      `${label} physical WebSocket entry key must be ${WEBSOCKET_GATEWAY_ENTRY_KEY}`
    );
  }
  const surface = exactObject(
    input.surface,
    `${label}.protocolSurface.protocol.surface`
  );
  exactFields(
    surface,
    [
      'connectRequestShape',
      'connectResultShape',
      'connectionPolicyShape',
      'externalSources',
      'downlinkFrames',
      'rpcProfiles'
    ],
    `${label}.protocolSurface.protocol.surface`
  );
  if (
    surface.connectRequestShape !== 'v1' ||
    surface.connectResultShape !== 'v1' ||
    surface.connectionPolicyShape !== 'v1'
  ) {
    throw new Error(`${label}.protocolSurface WebSocket shapes must all be v1`);
  }
  assertExactSourceKinds(
    surface.externalSources,
    ['websocket.connectRequest', 'websocket.connectionId'],
    `${label}.protocolSurface.protocol.surface.externalSources`
  );
  if (
    !Array.isArray(surface.downlinkFrames) ||
    surface.downlinkFrames.length !== 2 ||
    surface.downlinkFrames[0] !== 'binary' ||
    surface.downlinkFrames[1] !== 'text'
  ) {
    throw new Error(
      `${label}.protocolSurface WebSocket downlinkFrames must be binary,text`
    );
  }
  if (
    !Array.isArray(surface.rpcProfiles) ||
    surface.rpcProfiles.length !== 1 ||
    surface.rpcProfiles[0] !== 'jsonrpc-2.0-text'
  ) {
    throw new Error(
      `${label}.protocolSurface WebSocket rpcProfiles must be exactly jsonrpc-2.0-text`
    );
  }
  if (value.pre !== null || value.guard !== null) {
    throw new Error(`${label} physical WebSocket entry cannot declare pre or guard`);
  }
  const handler =
    value.handler === null
      ? undefined
      : requiredString(value, 'handler', label);
  if (handler !== undefined) {
    assertPrivateCurrentPackageHandler(
      handler,
      implementationPackageId,
      `${label}.handler`
    );
  }
  decodeAdapterPlan(
    value.adapterPlan,
    'websocketConnect',
    new Set(['websocket.connectRequest', 'websocket.connectionId']),
    `${label}.adapterPlan`
  );
  const adapterPlan = exactObject(value.adapterPlan, `${label}.adapterPlan`);
  if (
    handler === undefined &&
    Array.isArray(adapterPlan.args) &&
    adapterPlan.args.length !== 0
  ) {
    throw new Error(
      `${label} handler-absent physical WebSocket entry must have no adapter args`
    );
  }
  return {
    kind: 'websocketConnect',
    gatewayEntryIdentity,
    ...(handler === undefined ? {} : { handler }),
    rpcProfiles: Object.freeze(['jsonrpc-2.0-text'] as const),
    websocketEntryId: deriveWebSocketEntryId(serviceId, gatewayEntryKey)
  };
}

function decodeMethodEntry(input: {
  value: Record<string, unknown>;
  surface: unknown;
  label: string;
  implementationPackageId: string;
  gatewayEntryKey: string;
  gatewayEntryIdentity: string;
}): DecodedRuntimeAssemblyWebSocketMethodEntry {
  const {
    value,
    label,
    implementationPackageId,
    gatewayEntryKey,
    gatewayEntryIdentity
  } = input;
  if (gatewayEntryKey === WEBSOCKET_GATEWAY_ENTRY_KEY) {
    throw new Error(
      `${label} WebSocket JSON-RPC method cannot use compiler-owned physical key`
    );
  }
  const surface = exactObject(
    input.surface,
    `${label}.protocolSurface.protocol.surface`
  );
  exactFields(
    surface,
    [
      'profile',
      'dispatchMode',
      'externalSources',
      'paramsSchema',
      'resultSchema'
    ],
    `${label}.protocolSurface.protocol.surface`
  );
  if (surface.profile !== 'jsonrpc-2.0-text') {
    throw new Error(`${label}.protocolSurface JSON-RPC profile is unsupported`);
  }
  if (surface.dispatchMode !== 'unary') {
    throw new Error(`${label}.protocolSurface JSON-RPC dispatchMode must be unary`);
  }
  const sourceKinds = decodeSourceKinds(
    surface.externalSources,
    new Set([
      'websocket.businessIdentity',
      'websocket.connectionId',
      'websocket.jsonRpcParams'
    ]),
    `${label}.protocolSurface.protocol.surface.externalSources`
  );
  if (
    sourceKinds.length === 0 ||
    sourceKinds[sourceKinds.length - 1] !== 'websocket.jsonRpcParams' ||
    sourceKinds.some(
      (kind, index) =>
        index > 0 && compareUtf8(kind, sourceKinds[index - 1]!) <= 0
    )
  ) {
    throw new Error(
      `${label}.protocolSurface JSON-RPC externalSources must be canonical and include websocket.jsonRpcParams`
    );
  }
  const paramsSchema = decodeGatewayExternalSchema(
    surface.paramsSchema,
    `${label}.protocolSurface.protocol.surface.paramsSchema`
  );
  if (!paramsSchema.structured) {
    throw new Error(
      `${label}.protocolSurface JSON-RPC paramsSchema must accept only records or arrays`
    );
  }
  decodeGatewayExternalSchema(
    surface.resultSchema,
    `${label}.protocolSurface.protocol.surface.resultSchema`
  );
  if (value.pre !== null || value.guard !== null) {
    throw new Error(`${label} WebSocket JSON-RPC method cannot declare pre or guard`);
  }
  const handler = requiredString(value, 'handler', label);
  assertPrivateCurrentPackageHandler(
    handler,
    implementationPackageId,
    `${label}.handler`
  );
  const adapterSources = decodeAdapterPlan(
    value.adapterPlan,
    'websocketJsonRpc',
    new Set([
      'websocket.businessIdentity',
      'websocket.connectionId',
      'websocket.jsonRpcParams'
    ]),
    `${label}.adapterPlan`
  );
  if (
    new Set(adapterSources).size !== adapterSources.length ||
    stableStringify([...adapterSources].sort()) !== stableStringify(sourceKinds)
  ) {
    throw new Error(
      `${label}.adapterPlan JSON-RPC sources must exactly match its protocol surface`
    );
  }
  return {
    kind: 'websocketJsonRpc',
    gatewayEntryIdentity,
    handler,
    profile: 'jsonrpc-2.0-text'
  };
}

function decodeAdapterPlan(
  input: unknown,
  expectedKind: 'websocketConnect' | 'websocketJsonRpc',
  allowedSources: ReadonlySet<string>,
  label: string
): readonly string[] {
  const value = exactObject(input, label);
  exactFields(value, ['kind', 'args'], label);
  if (value.kind !== expectedKind) {
    throw new Error(`${label}.kind must be ${expectedKind}`);
  }
  if (!Array.isArray(value.args)) {
    throw new Error(`${label}.args must be an array`);
  }
  const params = new Set<string>();
  const sources: string[] = [];
  for (const [index, inputArgument] of value.args.entries()) {
    const argumentLabel = `${label}.args[${index}]`;
    const argument = exactObject(inputArgument, argumentLabel);
    exactFields(argument, ['param', 'source'], argumentLabel);
    const param = requiredString(argument, 'param', argumentLabel);
    if (/[\s\p{Cc}]/u.test(param) || params.has(param)) {
      throw new Error(`${argumentLabel}.param is invalid or duplicated`);
    }
    params.add(param);
    const source = exactObject(argument.source, `${argumentLabel}.source`);
    exactFields(source, ['kind'], `${argumentLabel}.source`);
    if (typeof source.kind !== 'string' || !allowedSources.has(source.kind)) {
      throw new Error(`${argumentLabel}.source.kind is invalid for ${expectedKind}`);
    }
    sources.push(source.kind);
  }
  return sources;
}

function assertPrivateCurrentPackageHandler(
  handler: string,
  implementationPackageId: string,
  label: string
): void {
  const privatePrefix = `pkg-callable:${implementationPackageId}:top-level:`;
  if (!handler.startsWith(privatePrefix) || handler.length === privatePrefix.length) {
    throw new Error(
      `${label} must be a private callable owned by the current implementation package`
    );
  }
}

function decodeGatewayExternalSchema(
  input: unknown,
  label: string
): { readonly canonical: unknown; readonly structured: boolean } {
  const value = exactObject(input, label);
  const kind = requiredString(value, 'kind', label);
  if (
    kind === 'null' ||
    kind === 'string' ||
    kind === 'number' ||
    kind === 'integer' ||
    kind === 'boolean' ||
    kind === 'bytes'
  ) {
    exactFields(value, ['kind'], label);
    return { canonical: { kind }, structured: false };
  }
  if (kind === 'stringLiteral') {
    exactFields(value, ['kind', 'value'], label);
    if (typeof value.value !== 'string') {
      throw new Error(`${label}.value must be a string`);
    }
    return {
      canonical: { kind, value: value.value },
      structured: false
    };
  }
  if (kind === 'array') {
    exactFields(value, ['kind', 'items'], label);
    const items = decodeGatewayExternalSchema(value.items, `${label}.items`);
    return {
      canonical: { kind, items: items.canonical },
      structured: true
    };
  }
  if (kind === 'record') {
    exactFields(value, ['kind', 'fields', 'required'], label);
    const fields = exactObject(value.fields, `${label}.fields`);
    const canonicalFields: Record<string, unknown> = {};
    for (const [name, field] of Object.entries(fields)) {
      if (
        name.length === 0 ||
        name !== name.trim() ||
        /\p{Cc}/u.test(name)
      ) {
        throw new Error(`${label}.fields contains an invalid field name`);
      }
      canonicalFields[name] = decodeGatewayExternalSchema(
        field,
        `${label}.fields.${name}`
      ).canonical;
    }
    if (!Array.isArray(value.required)) {
      throw new Error(`${label}.required must be an array`);
    }
    const required: string[] = [];
    for (const [index, field] of value.required.entries()) {
      if (
        typeof field !== 'string' ||
        field.length === 0 ||
        !Object.hasOwn(fields, field) ||
        (index > 0 && compareUtf8(field, required[index - 1]!) <= 0)
      ) {
        throw new Error(
          `${label}.required must be a sorted unique subset of fields`
        );
      }
      required.push(field);
    }
    return {
      canonical: { kind, fields: canonicalFields, required },
      structured: true
    };
  }
  if (kind === 'closedUnion') {
    exactFields(value, ['kind', 'branches'], label);
    if (!Array.isArray(value.branches) || value.branches.length === 0) {
      throw new Error(`${label}.branches must be a non-empty array`);
    }
    const branches = value.branches.map((branch, index) =>
      decodeGatewayExternalSchema(branch, `${label}.branches[${index}]`)
    );
    const keys = branches.map((branch) => stableStringify(branch.canonical));
    if (
      keys.some(
        (key, index) =>
          index > 0 && compareUtf8(key, keys[index - 1]!) <= 0
      ) ||
      branches.some((branch) => {
        const branchKind = (branch.canonical as { kind?: unknown }).kind;
        return branchKind === 'null' ||
          branchKind === 'nullable' ||
          branchKind === 'closedUnion';
      })
    ) {
      throw new Error(`${label}.branches must be flat, canonical and unique`);
    }
    return {
      canonical: {
        kind,
        branches: branches.map((branch) => branch.canonical)
      },
      structured: branches.every((branch) => branch.structured)
    };
  }
  if (kind === 'nullable') {
    exactFields(value, ['kind', 'inner'], label);
    const inner = decodeGatewayExternalSchema(value.inner, `${label}.inner`);
    const innerKind = (inner.canonical as { kind?: unknown }).kind;
    if (innerKind === 'null' || innerKind === 'nullable') {
      throw new Error(`${label}.inner is not canonical`);
    }
    return {
      canonical: { kind, inner: inner.canonical },
      structured: false
    };
  }
  throw new Error(`${label}.kind is unsupported`);
}

function assertExactSourceKinds(
  input: unknown,
  expected: readonly string[],
  label: string
): void {
  const actual = decodeSourceKinds(input, new Set(expected), label);
  if (stableStringify(actual) !== stableStringify(expected)) {
    throw new Error(`${label} must be the exact fixed source sequence`);
  }
}

function decodeSourceKinds(
  input: unknown,
  allowed: ReadonlySet<string>,
  label: string
): string[] {
  if (!Array.isArray(input)) {
    throw new Error(`${label} must be an array`);
  }
  return input.map((sourceInput, index) => {
    const source = exactObject(sourceInput, `${label}[${index}]`);
    exactFields(source, ['kind'], `${label}[${index}]`);
    if (typeof source.kind !== 'string' || !allowed.has(source.kind)) {
      throw new Error(`${label}[${index}].kind is invalid`);
    }
    return source.kind;
  });
}

function decodeFixedErrorProjection(input: unknown, label: string): void {
  const value = exactObject(input, label);
  exactFields(value, ['kind', 'version'], label);
  if (value.kind !== 'fixed' || value.version !== 'v1') {
    throw new Error(`${label} must be fixed v1`);
  }
}

function freezeMethodBinding(
  binding: RuntimeAssemblyWebSocketMethodBinding
): RuntimeAssemblyWebSocketMethodBinding {
  const deployment = Object.freeze({ ...binding.deployment });
  return Object.freeze({
    method: binding.method,
    profile: binding.profile,
    deployment,
    gatewayEntryKey: binding.gatewayEntryKey,
    gatewayEntryIdentity: binding.gatewayEntryIdentity,
    handler: binding.handler,
    websocketEntryId: binding.websocketEntryId
  });
}

function exactObject(input: unknown, label: string): Record<string, unknown> {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error(`${label} must be an object`);
  }
  return input as Record<string, unknown>;
}

function exactFields(
  value: Record<string, unknown>,
  expected: readonly string[],
  label: string
): void {
  const actual = Object.keys(value).sort();
  const canonical = [...expected].sort();
  if (
    actual.length !== canonical.length ||
    actual.some((field, index) => field !== canonical[index])
  ) {
    throw new Error(`${label} fields must be exactly ${canonical.join(',')}`);
  }
}

function requiredString(
  value: Record<string, unknown>,
  field: string,
  label: string
): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== 'string' || fieldValue.length === 0) {
    throw new Error(`${label}.${field} must be a non-empty string`);
  }
  return fieldValue;
}

function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

export function methodBindingFromDecoded(input: {
  methodSelector: RuntimeAssemblyWebSocketIngressSelector;
  entry: DecodedRuntimeAssemblyWebSocketMethodEntry;
  deployment: RuntimeAssemblyDeploymentRef;
  gatewayEntryKey: string;
  websocketEntryId: string;
}): RuntimeAssemblyWebSocketMethodBinding {
  if (input.methodSelector.method === null) {
    throw new Error('WebSocket JSON-RPC method binding requires a method selector');
  }
  return {
    method: input.methodSelector.method,
    profile: input.entry.profile,
    deployment: input.deployment,
    gatewayEntryKey: input.gatewayEntryKey,
    gatewayEntryIdentity: input.entry.gatewayEntryIdentity,
    handler: input.entry.handler,
    websocketEntryId: input.websocketEntryId
  };
}
