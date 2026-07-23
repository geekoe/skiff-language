import type {
  EnvironmentActivationState,
  RuntimeAssemblyRef
} from '../protocol/assemblyActivationProtocol.js';

const CONTRACT_OPERATION_IDENTITY_PATTERN =
  /^skiff-contract-operation-v1:sha256:[0-9a-f]{64}$/;
const DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN =
  /^skiff-deployment-artifact-v1:sha256:[0-9a-f]{64}$/;
const SERVICE_PROTOCOL_IDENTITY_PATTERN =
  /^skiff-service-protocol-v2:sha256:[0-9a-f]{64}$/;

export type RuntimeAssemblyIngressProtocol = 'http' | 'webSocket';

export interface RuntimeAssemblyIngressSelector {
  protocol: RuntimeAssemblyIngressProtocol;
  host: string;
  method: string | null;
  path: string;
}

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
  contract: RuntimeAssemblyContractRef;
  contractOperationId: string;
  operationMode: 'unary' | 'serverStream';
}

export interface LoadedRuntimeAssembly {
  schemaVersion: string;
  assemblyIdentity: string;
  resolvedDeployments?: readonly RuntimeAssemblyDeploymentRef[];
  resolvedContracts?: readonly RuntimeAssemblyContractRef[];
  globalIngress: readonly RuntimeAssemblyIngressBinding[];
}

export interface RuntimeAssemblySnapshotLoader {
  load(ref: RuntimeAssemblyRef): Promise<LoadedRuntimeAssembly>;
}

export class MemoryRuntimeAssemblySnapshotLoader implements RuntimeAssemblySnapshotLoader {
  private readonly byIdentity = new Map<string, LoadedRuntimeAssembly>();

  constructor(assemblies: readonly LoadedRuntimeAssembly[]) {
    for (const assembly of assemblies) {
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
        throw new Error(`RuntimeAssembly contains duplicate global ingress ${key}`);
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
    ingress: new RuntimeAssemblyIngressIndex(assembly.globalIngress)
  };
}

export function runtimeAssemblyIngressKey(
  selector: RuntimeAssemblyIngressSelector
): string {
  const protocol = selector.protocol;
  const host = canonicalIngressHost(selector.host);
  const method = selector.method === null ? '' : selector.method.toUpperCase();
  if (protocol === 'http' && method.length === 0) {
    throw new Error('HTTP RuntimeAssembly ingress requires a method');
  }
  if (protocol === 'webSocket' && method.length > 0) {
    throw new Error('WebSocket RuntimeAssembly ingress must not declare a method');
  }
  if (!selector.path.startsWith('/') || selector.path.includes('?') || selector.path.includes('#')) {
    throw new Error('RuntimeAssembly ingress path must be an absolute URL path');
  }
  return `${protocol}\u0000${host}\u0000${method}\u0000${selector.path}`;
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

export function decodeRouterSnapshot(
  input: unknown,
  expectedAssembly: RuntimeAssemblyRef
): { assembly: LoadedRuntimeAssembly } {
  const value = exactObject(input, 'RouterSnapshot');
  exactFields(value, ['assembly', 'serviceContracts'], 'RouterSnapshot');
  if (!Array.isArray(value.serviceContracts)) {
    throw new Error('RouterSnapshot.serviceContracts must be an array');
  }
  const operationModes = decodeContractOperationModes(value.serviceContracts);
  const assembly = decodeRuntimeAssemblyIngressSurface(value.assembly, operationModes);
  if (assembly.assemblyIdentity !== expectedAssembly.assemblyIdentity) {
    throw new Error('RouterSnapshot assembly does not match the exact requested reference');
  }
  return { assembly };
}

function decodeRuntimeAssemblyIngressSurface(
  input: unknown,
  operationModes: ReadonlyMap<string, 'unary' | 'serverStream'>
): LoadedRuntimeAssembly {
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
    'globalIngress'
  ], 'RuntimeAssembly');
  if (value.schemaVersion !== 'skiff-runtime-assembly-v1') {
    throw new Error('RuntimeAssembly schemaVersion must be skiff-runtime-assembly-v1');
  }
  const assemblyIdentity = requiredString(value, 'assemblyIdentity');
  if (!/^skiff-runtime-assembly-v1:sha256:[0-9a-f]{64}$/.test(assemblyIdentity)) {
    throw new Error('RuntimeAssembly assemblyIdentity is invalid');
  }
  if (
    !Array.isArray(value.resolvedDeployments) ||
    !Array.isArray(value.resolvedContracts) ||
    !Array.isArray(value.globalIngress)
  ) {
    throw new Error(
      'RuntimeAssembly resolvedDeployments, resolvedContracts and globalIngress must be arrays'
    );
  }
  return {
    schemaVersion: value.schemaVersion,
    assemblyIdentity,
    resolvedDeployments: value.resolvedDeployments.map((entry, index) =>
      decodeDeploymentRef(
        entry,
        `RuntimeAssembly.resolvedDeployments[${index}]`
      )
    ),
    resolvedContracts: value.resolvedContracts.map((entry, index) =>
      decodeContractRef(entry, `RuntimeAssembly.resolvedContracts[${index}]`)
    ),
    globalIngress: value.globalIngress.map((entry, index) =>
      decodeIngressBinding(
        entry,
        `RuntimeAssembly.globalIngress[${index}]`,
        operationModes
      )
    )
  };
}

function decodeIngressBinding(
  input: unknown,
  label: string,
  operationModes: ReadonlyMap<string, 'unary' | 'serverStream'>
): RuntimeAssemblyIngressBinding {
  const value = exactObject(input, label);
  exactFields(
    value,
    ['selector', 'deployment', 'contract', 'contractOperationId'],
    label
  );
  const selectorValue = exactObject(value.selector, `${label}.selector`);
  exactFields(selectorValue, ['protocol', 'host', 'method', 'path'], `${label}.selector`);
  if (selectorValue.protocol !== 'http' && selectorValue.protocol !== 'webSocket') {
    throw new Error(`${label}.selector.protocol is invalid`);
  }
  if (selectorValue.method !== null && typeof selectorValue.method !== 'string') {
    throw new Error(`${label}.selector.method must be a string or null`);
  }
  const selector: RuntimeAssemblyIngressSelector = {
    protocol: selectorValue.protocol,
    host: requiredString(selectorValue, 'host'),
    method: selectorValue.method,
    path: requiredString(selectorValue, 'path')
  };
  runtimeAssemblyIngressKey(selector);
  const deployment = decodeDeploymentRef(value.deployment, `${label}.deployment`);
  const contract = decodeContractRef(value.contract, `${label}.contract`);
  if (
    deployment.serviceId !== contract.serviceId ||
    deployment.contractVersion !== contract.contractVersion
  ) {
    throw new Error(`${label} deployment and contract coordinates must match`);
  }
  const contractOperationId = requiredString(value, 'contractOperationId');
  if (!CONTRACT_OPERATION_IDENTITY_PATTERN.test(contractOperationId)) {
    throw new Error(`${label}.contractOperationId is invalid`);
  }
  const operationMode = operationModes.get(contractOperationKey(contract, contractOperationId));
  if (operationMode === undefined) {
    throw new Error(`${label}.contractOperationId is absent from the exact ServiceContract`);
  }
  return {
    selector,
    deployment,
    contract,
    contractOperationId,
    operationMode
  };
}

function decodeContractOperationModes(
  contracts: readonly unknown[]
): ReadonlyMap<string, 'unary' | 'serverStream'> {
  const modes = new Map<string, 'unary' | 'serverStream'>();
  const contractCoordinates = new Set<string>();
  for (const [index, input] of contracts.entries()) {
    const label = `RouterSnapshot.serviceContracts[${index}]`;
    const contract = exactObject(input, label);
    exactFields(contract, [
      'schemaVersion',
      'serviceId',
      'contractVersion',
      'serviceProtocolIdentity',
      'operations',
      'boundarySchema',
      'diagnosticText'
    ], label);
    if (contract.schemaVersion !== 'skiff-service-contract-v2') {
      throw new Error(`${label}.schemaVersion must be skiff-service-contract-v2`);
    }
    const ref: RuntimeAssemblyContractRef = {
      serviceId: requiredString(contract, 'serviceId'),
      contractVersion: requiredString(contract, 'contractVersion'),
      serviceProtocolIdentity: requiredString(contract, 'serviceProtocolIdentity')
    };
    if (!SERVICE_PROTOCOL_IDENTITY_PATTERN.test(ref.serviceProtocolIdentity)) {
      throw new Error(`${label}.serviceProtocolIdentity is invalid`);
    }
    const coordinate = contractCoordinateKey(ref);
    if (contractCoordinates.has(coordinate)) {
      throw new Error(`${label} duplicates an exact ServiceContract coordinate`);
    }
    contractCoordinates.add(coordinate);
    const operations = exactObject(contract.operations, `${label}.operations`);
    for (const [operationId, descriptorInput] of Object.entries(operations)) {
      if (!CONTRACT_OPERATION_IDENTITY_PATTERN.test(operationId)) {
        throw new Error(`${label}.operations contains an invalid operation identity`);
      }
      const descriptor = exactObject(
        descriptorInput,
        `${label}.operations.${operationId}`
      );
      exactFields(descriptor, ['operationId', 'stableKey', 'contract'], `${label}.operations`);
      if (descriptor.operationId !== operationId) {
        throw new Error(`${label}.operations descriptor identity mismatch`);
      }
      const operationContract = exactObject(
        descriptor.contract,
        `${label}.operations.${operationId}.contract`
      );
      const stream = exactObject(
        operationContract.stream,
        `${label}.operations.${operationId}.contract.stream`
      );
      const kind = requiredString(stream, 'kind');
      if (kind !== 'unary' && kind !== 'serverStream') {
        throw new Error(
          `${label}.operations.${operationId} is not available for Router ingress`
        );
      }
      modes.set(contractOperationKey(ref, operationId), kind);
    }
  }
  return modes;
}

function contractOperationKey(
  contract: RuntimeAssemblyContractRef,
  operationId: string
): string {
  return `${contractCoordinateKey(contract)}\u0000${operationId}`;
}

function contractCoordinateKey(contract: RuntimeAssemblyContractRef): string {
  return [
    contract.serviceId,
    contract.contractVersion,
    contract.serviceProtocolIdentity
  ].join('\u0000');
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
