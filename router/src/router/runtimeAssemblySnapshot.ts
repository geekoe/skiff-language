import type {
  EnvironmentActivationState,
  RuntimeAssemblyRef
} from '../protocol/assemblyActivationProtocol.js';

const DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN =
  /^skiff-deployment-artifact-v2:sha256:[0-9a-f]{64}$/;
const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v1:sha256:[0-9a-f]{64}$/;
const RUNTIME_ASSEMBLY_IDENTITY_PATTERN =
  /^skiff-runtime-assembly-v2:sha256:[0-9a-f]{64}$/;
const SERVICE_PROTOCOL_IDENTITY_PATTERN =
  /^skiff-service-protocol-v5:sha256:[0-9a-f]{64}$/;

export interface RuntimeAssemblyHttpIngressSelector {
  protocol: 'http';
  host: string;
  method: string;
  path: string;
}

export interface RuntimeAssemblyWebSocketIngressSelector {
  protocol: 'webSocket';
  host: string;
  method: null;
  path: string;
}

export type RuntimeAssemblyIngressSelector =
  | RuntimeAssemblyHttpIngressSelector
  | RuntimeAssemblyWebSocketIngressSelector;

export interface RuntimeAssemblyDeploymentRef {
  serviceId: string;
  contractVersion: string;
  deploymentRevision: string;
  deploymentArtifactIdentity: string;
}

export interface RuntimeAssemblyContractRef {
  serviceId: string;
  contractVersion: string;
  serviceProtocolIdentity: string;
}

export interface RuntimeAssemblyIngressBinding {
  selector: RuntimeAssemblyIngressSelector;
  deployment: RuntimeAssemblyDeploymentRef;
  gatewayEntryKey: string;
  gatewayEntryIdentity: string;
  adapterKind: 'rawHttp' | 'typedJson' | 'websocketConnect';
  operationMode: 'unary' | 'serverStream';
  handler?: string;
  websocketEntryId?: string;
  timeoutMs?: number;
}

export interface LoadedRuntimeAssembly {
  schemaVersion: 'skiff-runtime-assembly-v2';
  assemblyIdentity: string;
  resolvedDeployments?: readonly RuntimeAssemblyDeploymentRef[];
  resolvedContracts?: readonly RuntimeAssemblyContractRef[];
  gatewayIngress: readonly RuntimeAssemblyIngressBinding[];
  actorMethods?: readonly RuntimeAssemblyActorMethod[];
}

export interface DecodedRuntimeAssemblyRecord {
  schemaVersion: 'skiff-runtime-assembly-v2';
  assemblyIdentity: string;
  resolvedDeployments: readonly RuntimeAssemblyDeploymentRef[];
  resolvedContracts: readonly RuntimeAssemblyContractRef[];
  gatewayIngress: readonly RuntimeAssemblyGatewayIngressDeclaration[];
}

export interface RuntimeAssemblyGatewayIngressDeclaration {
  selector: RuntimeAssemblyIngressSelector;
  deployment: RuntimeAssemblyDeploymentRef;
  gatewayEntryKey: string;
  gatewayEntryIdentity: string;
}

export interface RuntimeAssemblyActorMethod {
  declarationOwner: {
    unit: { kind: 'service' } | { kind: 'package'; value: number };
    file:
      | { kind: 'loadedFileIndex'; value: number }
      | { kind: 'fileIrIdentity'; value: string };
    actorSymbol: string;
  };
  actorAbiIdentity: string;
  actorImplementationIdentity: string;
  methodIdentity: string;
}

export interface RuntimeAssemblySnapshotLoader {
  load(ref: RuntimeAssemblyRef): Promise<LoadedRuntimeAssembly>;
}

export class MemoryRuntimeAssemblySnapshotLoader implements RuntimeAssemblySnapshotLoader {
  private readonly byIdentity = new Map<string, LoadedRuntimeAssembly>();

  constructor(assemblies: readonly LoadedRuntimeAssembly[]) {
    for (const assembly of assemblies) {
      if (
        assembly.schemaVersion !== 'skiff-runtime-assembly-v2' ||
        !RUNTIME_ASSEMBLY_IDENTITY_PATTERN.test(assembly.assemblyIdentity)
      ) {
        throw new Error('memory RuntimeAssembly loader accepts only canonical v2 records');
      }
      if (this.byIdentity.has(assembly.assemblyIdentity)) {
        throw new Error(`duplicate RuntimeAssembly ${assembly.assemblyIdentity}`);
      }
      this.byIdentity.set(assembly.assemblyIdentity, assembly);
    }
  }

  async load(ref: RuntimeAssemblyRef): Promise<LoadedRuntimeAssembly> {
    const assembly = this.byIdentity.get(ref.assemblyIdentity);
    if (assembly === undefined) {
      throw new Error(`RuntimeAssembly ${ref.assemblyIdentity} is unavailable`);
    }
    return structuredClone(assembly);
  }
}

export interface RouterActiveAssemblySnapshot {
  environment: string;
  generation: number;
  assembly: RuntimeAssemblyRef;
  resolvedDeployments?: readonly RuntimeAssemblyDeploymentRef[];
  resolvedContracts?: readonly RuntimeAssemblyContractRef[];
  ingress: RuntimeAssemblyIngressIndex;
  actorMethods?: readonly RuntimeAssemblyActorMethod[];
}

export class RouterActiveAssemblySnapshotStore {
  private snapshot: RouterActiveAssemblySnapshot | undefined;

  get(): RouterActiveAssemblySnapshot {
    if (this.snapshot === undefined) {
      throw new Error('router active RuntimeAssembly snapshot is not initialized');
    }
    return this.snapshot;
  }

  replace(snapshot: RouterActiveAssemblySnapshot): void {
    const current = this.snapshot;
    if (
      current !== undefined &&
      (snapshot.environment !== current.environment ||
        snapshot.generation < current.generation ||
        (snapshot.generation === current.generation &&
          snapshot.assembly.assemblyIdentity !== current.assembly.assemblyIdentity))
    ) {
      throw new Error('router active RuntimeAssembly snapshot cannot fork or move backward');
    }
    this.snapshot = snapshot;
  }
}

export class RuntimeAssemblyIngressIndex {
  private readonly bindings = new Map<string, RuntimeAssemblyIngressBinding>();

  constructor(bindings: readonly RuntimeAssemblyIngressBinding[]) {
    for (const binding of bindings) {
      const key = runtimeAssemblyIngressKey(binding.selector);
      if (this.bindings.has(key)) {
        throw new Error(`RuntimeAssembly contains duplicate gateway ingress ${key}`);
      }
      this.bindings.set(key, binding);
    }
  }

  get(input: RuntimeAssemblyIngressSelector): RuntimeAssemblyIngressBinding | undefined {
    return this.bindings.get(runtimeAssemblyIngressKey(input));
  }

  values(): readonly RuntimeAssemblyIngressBinding[] {
    return Array.from(this.bindings.values());
  }
}

export async function snapshotFromCommittedActivation(
  state: EnvironmentActivationState,
  loader: RuntimeAssemblySnapshotLoader
): Promise<RouterActiveAssemblySnapshot> {
  const assembly = await loader.load(state.committed.assembly);
  if (assembly.assemblyIdentity !== state.committed.assembly.assemblyIdentity) {
    throw new Error('loaded RuntimeAssembly identity does not match committed activation');
  }
  return {
    environment: state.environment,
    generation: state.committed.generation,
    assembly: state.committed.assembly,
    ...(assembly.resolvedDeployments === undefined
      ? {}
      : { resolvedDeployments: assembly.resolvedDeployments }),
    ...(assembly.resolvedContracts === undefined
      ? {}
      : { resolvedContracts: assembly.resolvedContracts }),
    ingress: new RuntimeAssemblyIngressIndex(assembly.gatewayIngress),
    ...(assembly.actorMethods === undefined
      ? {}
      : { actorMethods: assembly.actorMethods })
  };
}

export function runtimeAssemblyIngressKey(
  selector: RuntimeAssemblyIngressSelector
): string {
  const host = canonicalIngressHost(selector.host);
  if (
    typeof selector.path !== 'string' ||
    !selector.path.startsWith('/') ||
    selector.path.includes('?') ||
    selector.path.includes('#')
  ) {
    throw new Error('RuntimeAssembly ingress path must be an absolute URL path');
  }
  if (selector.protocol === 'webSocket') {
    if (selector.method !== null) {
      throw new Error('WebSocket RuntimeAssembly ingress method must be null');
    }
    return `webSocket\u0000${host}\u0000${selector.path}`;
  }
  const method = selector.method.toUpperCase();
  if (method.length === 0) {
    throw new Error('HTTP RuntimeAssembly ingress requires a method');
  }
  return `http\u0000${host}\u0000${method}\u0000${selector.path}`;
}

export function canonicalIngressHost(host: string): string {
  const value = host.trim();
  if (value.length === 0 || value.includes('/') || value.includes('@')) {
    throw new Error('RuntimeAssembly ingress Host is required');
  }
  try {
    const url = new URL(`http://${value}`);
    if (
      url.username.length > 0 ||
      url.password !== '' ||
      url.pathname !== '/' ||
      url.search !== '' ||
      url.hash !== ''
    ) {
      throw new Error('invalid host');
    }
    return url.host.toLowerCase();
  } catch {
    throw new Error(`invalid RuntimeAssembly ingress Host ${value}`);
  }
}

export function decodeRuntimeAssemblyRecord(
  input: unknown,
  expectedAssembly: RuntimeAssemblyRef
): DecodedRuntimeAssemblyRecord {
  const value = exactObject(input, 'RuntimeAssembly');
  exactFields(value, [
    'schemaVersion',
    'assemblyIdentity',
    'roots',
    'resolvedDeployments',
    'resolvedContracts',
    'resolvedPackages',
    'packageLinkPlan',
    'serviceBindingTemplates',
    'activationTemplates',
    'gatewayIngress'
  ], 'RuntimeAssembly');
  if (value.schemaVersion !== 'skiff-runtime-assembly-v2') {
    throw new Error('RuntimeAssembly schemaVersion must be skiff-runtime-assembly-v2');
  }
  const assemblyIdentity = requiredString(value, 'assemblyIdentity');
  if (!RUNTIME_ASSEMBLY_IDENTITY_PATTERN.test(assemblyIdentity)) {
    throw new Error('RuntimeAssembly assemblyIdentity is invalid');
  }
  if (assemblyIdentity !== expectedAssembly.assemblyIdentity) {
    throw new Error('RouterSnapshot assembly does not match the exact requested reference');
  }
  if (
    !Array.isArray(value.roots) ||
    !Array.isArray(value.resolvedDeployments) ||
    !Array.isArray(value.resolvedContracts) ||
    !Array.isArray(value.resolvedPackages) ||
    !Array.isArray(value.serviceBindingTemplates) ||
    !Array.isArray(value.activationTemplates) ||
    !Array.isArray(value.gatewayIngress)
  ) {
    throw new Error(
      'RuntimeAssembly closure and gatewayIngress fields must be arrays'
    );
  }
  exactObject(value.packageLinkPlan, 'RuntimeAssembly.packageLinkPlan');
  const resolvedDeployments = value.resolvedDeployments.map((entry, index) =>
    decodeDeploymentRef(
      entry,
      `RuntimeAssembly.resolvedDeployments[${index}]`
    )
  );
  assertUniqueDeploymentRefs(resolvedDeployments);
  const resolvedContracts = value.resolvedContracts.map((entry, index) =>
    decodeContractRef(entry, `RuntimeAssembly.resolvedContracts[${index}]`)
  );
  assertUniqueContractRefs(resolvedContracts);
  const deploymentKeys = new Set(resolvedDeployments.map(deploymentRefKey));
  const gatewayIngress = value.gatewayIngress.map((entry, index) =>
    decodeGatewayIngressDeclaration(
      entry,
      `RuntimeAssembly.gatewayIngress[${index}]`,
      deploymentKeys
    )
  );
  assertUniqueSelectors(gatewayIngress, 'RuntimeAssembly.gatewayIngress');
  return {
    schemaVersion: 'skiff-runtime-assembly-v2',
    assemblyIdentity,
    resolvedDeployments,
    resolvedContracts,
    gatewayIngress
  };
}

function decodeGatewayIngressDeclaration(
  input: unknown,
  label: string,
  resolvedDeployments: ReadonlySet<string>
): RuntimeAssemblyGatewayIngressDeclaration {
  const value = exactObject(input, label);
  exactFields(
    value,
    ['selector', 'deployment', 'gatewayEntryKey', 'gatewayEntryIdentity'],
    label
  );
  const selector = decodeRuntimeAssemblyIngressSelector(
    value.selector,
    `${label}.selector`
  );
  const deployment = decodeDeploymentRef(value.deployment, `${label}.deployment`);
  if (!resolvedDeployments.has(deploymentRefKey(deployment))) {
    throw new Error(`${label}.deployment is absent from resolvedDeployments`);
  }
  const gatewayEntryKey = decodeRuntimeAssemblyGatewayEntryKey(
    value.gatewayEntryKey,
    label
  );
  const gatewayEntryIdentity = requiredString(value, 'gatewayEntryIdentity');
  if (!GATEWAY_ENTRY_IDENTITY_PATTERN.test(gatewayEntryIdentity)) {
    throw new Error(`${label}.gatewayEntryIdentity is invalid`);
  }
  return {
    selector,
    deployment,
    gatewayEntryKey,
    gatewayEntryIdentity
  };
}

export function decodeRuntimeAssemblyIngressSelector(
  input: unknown,
  label: string
): RuntimeAssemblyIngressSelector {
  const value = exactObject(input, label);
  exactFields(value, ['protocol', 'host', 'method', 'path'], label);
  if (value.protocol !== 'http' && value.protocol !== 'webSocket') {
    throw new Error(`${label}.protocol must be http or webSocket`);
  }
  const host = requiredString(value, 'host');
  if (canonicalIngressHost(host) !== host) {
    throw new Error(`${label}.host must be canonical lowercase Host syntax`);
  }
  const path = requiredString(value, 'path');
  if (/[\s\p{Cc}]/u.test(path)) {
    throw new Error(`${label}.path must not contain whitespace or control characters`);
  }
  let selector: RuntimeAssemblyIngressSelector;
  if (value.protocol === 'webSocket') {
    if (value.method !== null) {
      throw new Error(`${label}.method must be null for webSocket`);
    }
    selector = { protocol: 'webSocket', host, method: null, path };
  } else {
    const method = requiredString(value, 'method');
    if (
      method !== method.toUpperCase() ||
      !/^[A-Z0-9!#$%&'*+\-.^_`|~]+$/.test(method)
    ) {
      throw new Error(`${label}.method must be a canonical uppercase HTTP token`);
    }
    selector = { protocol: 'http', host, method, path };
  }
  runtimeAssemblyIngressKey(selector);
  return selector;
}

function assertUniqueDeploymentRefs(
  references: readonly RuntimeAssemblyDeploymentRef[]
): void {
  const exact = new Set<string>();
  const coordinates = new Map<string, string>();
  for (const reference of references) {
    const key = deploymentRefKey(reference);
    if (exact.has(key)) {
      throw new Error('RuntimeAssembly contains a duplicate resolved deployment');
    }
    exact.add(key);
    const coordinate = [
      reference.serviceId,
      reference.contractVersion,
      reference.deploymentRevision
    ].join('\u0000');
    const identity = coordinates.get(coordinate);
    if (
      identity !== undefined &&
      identity !== reference.deploymentArtifactIdentity
    ) {
      throw new Error(
        'RuntimeAssembly deployment coordinate resolves to multiple identities'
      );
    }
    coordinates.set(coordinate, reference.deploymentArtifactIdentity);
  }
}

function assertUniqueContractRefs(
  references: readonly RuntimeAssemblyContractRef[]
): void {
  const coordinates = new Set<string>();
  for (const reference of references) {
    const coordinate = `${reference.serviceId}\u0000${reference.contractVersion}`;
    if (coordinates.has(coordinate)) {
      throw new Error(
        'RuntimeAssembly contains a duplicate resolved contract coordinate'
      );
    }
    coordinates.add(coordinate);
  }
}

function assertUniqueSelectors(
  bindings: readonly { selector: RuntimeAssemblyIngressSelector }[],
  label: string
): void {
  const selectors = new Set<string>();
  for (const binding of bindings) {
    const key = runtimeAssemblyIngressKey(binding.selector);
    if (selectors.has(key)) {
      throw new Error(`${label} contains duplicate selector ${key}`);
    }
    selectors.add(key);
  }
}

function deploymentRefKey(reference: RuntimeAssemblyDeploymentRef): string {
  return [
    reference.serviceId,
    reference.contractVersion,
    reference.deploymentRevision,
    reference.deploymentArtifactIdentity
  ].join('\u0000');
}

export function decodeRuntimeAssemblyGatewayEntryKey(
  input: unknown,
  label: string
): string {
  if (
    typeof input !== 'string' ||
    input.length === 0 ||
    /[\s\p{Cc}]/u.test(input)
  ) {
    throw new Error(`${label}.gatewayEntryKey is invalid`);
  }
  return input;
}

function decodeDeploymentRef(
  input: unknown,
  label: string
): RuntimeAssemblyDeploymentRef {
  const value = exactObject(input, label);
  exactFields(
    value,
    ['serviceId', 'contractVersion', 'deploymentRevision', 'deploymentArtifactIdentity'],
    label
  );
  const deploymentArtifactIdentity = requiredString(value, 'deploymentArtifactIdentity');
  if (!DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN.test(deploymentArtifactIdentity)) {
    throw new Error(`${label}.deploymentArtifactIdentity is invalid`);
  }
  return {
    serviceId: requiredString(value, 'serviceId'),
    contractVersion: requiredString(value, 'contractVersion'),
    deploymentRevision: requiredString(value, 'deploymentRevision'),
    deploymentArtifactIdentity
  };
}

function decodeContractRef(input: unknown, label: string): RuntimeAssemblyContractRef {
  const value = exactObject(input, label);
  exactFields(
    value,
    ['serviceId', 'contractVersion', 'serviceProtocolIdentity'],
    label
  );
  const serviceProtocolIdentity = requiredString(value, 'serviceProtocolIdentity');
  if (!SERVICE_PROTOCOL_IDENTITY_PATTERN.test(serviceProtocolIdentity)) {
    throw new Error(`${label}.serviceProtocolIdentity is invalid`);
  }
  return {
    serviceId: requiredString(value, 'serviceId'),
    contractVersion: requiredString(value, 'contractVersion'),
    serviceProtocolIdentity
  };
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

function requiredString(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== 'string' || fieldValue.length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return fieldValue;
}
