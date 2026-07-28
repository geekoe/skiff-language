import {
  deriveCurrentRuntimeAssemblyGatewayEntryIdentity,
  deriveCurrentRuntimeAssemblyServiceDeploymentIdentity
} from './runtimeAssemblyDeploymentIdentity.js';
import {
  decodeRuntimeAssemblyGatewayEntryKey,
  decodeRuntimeAssemblyIngressSelector,
  runtimeAssemblyIngressKey,
  type DecodedRuntimeAssemblyRecord,
  type LoadedRuntimeAssembly,
  type RuntimeAssemblyContractRef,
  type RuntimeAssemblyDeploymentRef,
  type RuntimeAssemblyIngressBinding,
  type RuntimeAssemblyIngressSelector
} from './runtimeAssemblySnapshot.js';
import {
  decodeRuntimeAssemblyWebSocketGatewayEntry,
  isRuntimeAssemblyWebSocketProtocolKind,
  methodBindingFromDecoded,
  RuntimeAssemblyWebSocketMethodTable,
  type DecodedRuntimeAssemblyWebSocketEntry,
  type DecodedRuntimeAssemblyWebSocketMethodEntry,
  type DecodedRuntimeAssemblyWebSocketPhysicalEntry
} from './runtimeAssemblyWebSocketSnapshot.js';

export { deriveWebSocketEntryId } from './runtimeAssemblyWebSocketSnapshot.js';
export {
  deriveCurrentRuntimeAssemblyServiceDeploymentIdentity
} from './runtimeAssemblyDeploymentIdentity.js';

const DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN =
  /^skiff-deployment-artifact-v4:sha256:[0-9a-f]{64}$/;
const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v2:sha256:[0-9a-f]{64}$/;
const SERVICE_PROTOCOL_IDENTITY_PATTERN =
  /^skiff-service-protocol-v5:sha256:[0-9a-f]{64}$/;
const PACKAGE_BUILD_IDENTITY_PATTERN =
  /^skiff-package-build-v10:sha256:[0-9a-f]{64}$/;
const PACKAGE_LOCAL_ABI_IDENTITY_PATTERN =
  /^skiff-package-local-abi-v7:sha256:[0-9a-f]{64}$/;

interface DecodedServiceDeployment {
  ref: RuntimeAssemblyDeploymentRef;
  gatewayEntries: ReadonlyMap<string, DecodedDeploymentGatewayEntry>;
  ingress: readonly DecodedDeploymentIngressBinding[];
  timeoutMs?: number;
}

interface DecodedHttpDeploymentGatewayEntry {
  kind: 'http';
  gatewayEntryIdentity: string;
  adapterKind: 'rawHttp' | 'typedJson';
  operationMode: 'unary' | 'serverStream';
  handler: string;
}

type DecodedDeploymentGatewayEntry =
  | DecodedHttpDeploymentGatewayEntry
  | DecodedRuntimeAssemblyWebSocketEntry;

interface DecodedDeploymentIngressBinding {
  selector: RuntimeAssemblyIngressSelector;
  gatewayEntryKey: string;
}

interface ExpectedRuntimeAssemblyIngress {
  selector: RuntimeAssemblyIngressSelector;
  deployment: RuntimeAssemblyDeploymentRef;
  gatewayEntryKey: string;
  gatewayEntryIdentity: string;
  attachBinding?: RuntimeAssemblyIngressBinding;
}

export function joinRuntimeAssemblyDeployments(
  record: DecodedRuntimeAssemblyRecord,
  deploymentInputs: readonly unknown[]
): LoadedRuntimeAssembly {
  if (deploymentInputs.length !== record.resolvedDeployments.length) {
    throw new Error(
      'RouterSnapshot.serviceDeployments must exactly match RuntimeAssembly.resolvedDeployments'
    );
  }
  const contractByCoordinate = new Map(
    record.resolvedContracts.map((contract) => [
      contractCoordinate(contract.serviceId, contract.contractVersion),
      contract
    ])
  );
  const expectedBySelector = new Map<string, ExpectedRuntimeAssemblyIngress>();
  for (const [index, reference] of record.resolvedDeployments.entries()) {
    const expectedContract = contractByCoordinate.get(
      contractCoordinate(reference.serviceId, reference.contractVersion)
    );
    if (expectedContract === undefined) {
      throw new Error(
        `RuntimeAssembly deployment ${reference.serviceId}@${reference.contractVersion} has no exact resolved contract`
      );
    }
    const deployment = decodeServiceDeployment(
      deploymentInputs[index],
      reference,
      expectedContract,
      `RouterSnapshot.serviceDeployments[${index}]`
    );
    for (const expected of buildDeploymentIngressExpectations(deployment)) {
      const selectorKey = scopedSelectorKey(
        expected.deployment,
        expected.selector
      );
      if (expectedBySelector.has(selectorKey)) {
        throw new Error(
          `ServiceDeployment ingress contains duplicate selector ${selectorKey}`
        );
      }
      expectedBySelector.set(selectorKey, expected);
    }
  }

  const gatewayIngress: RuntimeAssemblyIngressBinding[] = [];
  for (const declared of record.gatewayIngress) {
    const selectorKey = scopedSelectorKey(
      declared.deployment,
      declared.selector
    );
    const expected = expectedBySelector.get(selectorKey);
    if (expected === undefined) {
      throw new Error(
        `RuntimeAssembly gatewayIngress contains extra selector ${selectorKey}`
      );
    }
    if (
      !selectorEquals(declared.selector, expected.selector) ||
      !deploymentRefEquals(declared.deployment, expected.deployment) ||
      declared.gatewayEntryKey !== expected.gatewayEntryKey ||
      declared.gatewayEntryIdentity !== expected.gatewayEntryIdentity
    ) {
      throw new Error(
        `RuntimeAssembly gatewayIngress does not exactly match ServiceDeployment selector ${selectorKey}`
      );
    }
    expectedBySelector.delete(selectorKey);
    if (expected.attachBinding !== undefined) {
      gatewayIngress.push(expected.attachBinding);
    }
  }
  if (expectedBySelector.size > 0) {
    const [missing] = expectedBySelector.keys();
    throw new Error(
      `RuntimeAssembly gatewayIngress is missing ServiceDeployment selector ${missing}`
    );
  }
  return {
    schemaVersion: record.schemaVersion,
    assemblyIdentity: record.assemblyIdentity,
    resolvedDeployments: record.resolvedDeployments,
    resolvedContracts: record.resolvedContracts,
    gatewayIngress
  };
}

function buildDeploymentIngressExpectations(
  deployment: DecodedServiceDeployment
): readonly ExpectedRuntimeAssemblyIngress[] {
  const expected: ExpectedRuntimeAssemblyIngress[] = [];
  const physical: Array<{
    binding: DecodedDeploymentIngressBinding;
    entry: DecodedRuntimeAssemblyWebSocketPhysicalEntry;
  }> = [];
  const methods: Array<{
    binding: DecodedDeploymentIngressBinding;
    entry: DecodedRuntimeAssemblyWebSocketMethodEntry;
  }> = [];
  const referenced = new Map<string, number>();
  const methodNames = new Set<string>();

  for (const binding of deployment.ingress) {
    const entry = deployment.gatewayEntries.get(binding.gatewayEntryKey);
    if (entry === undefined) {
      throw new Error(
        `ServiceDeployment ingress references missing gateway entry ${binding.gatewayEntryKey}`
      );
    }
    referenced.set(
      binding.gatewayEntryKey,
      (referenced.get(binding.gatewayEntryKey) ?? 0) + 1
    );
    if (entry.kind === 'http') {
      if (binding.selector.protocol !== 'http') {
        throw new Error(
          `ServiceDeployment ingress selector protocol does not match HTTP gateway entry ${binding.gatewayEntryKey}`
        );
      }
      const attachBinding: RuntimeAssemblyIngressBinding = {
        selector: binding.selector,
        deployment: deployment.ref,
        gatewayEntryKey: binding.gatewayEntryKey,
        gatewayEntryIdentity: entry.gatewayEntryIdentity,
        adapterKind: entry.adapterKind,
        operationMode: entry.operationMode,
        ...(deployment.timeoutMs === undefined
          ? {}
          : { timeoutMs: deployment.timeoutMs })
      };
      expected.push(expectedIngress(binding, deployment.ref, entry, attachBinding));
      continue;
    }
    if (binding.selector.protocol !== 'webSocket') {
      throw new Error(
        `ServiceDeployment ingress selector protocol does not match WebSocket gateway entry ${binding.gatewayEntryKey}`
      );
    }
    if (entry.kind === 'websocketConnect') {
      if (binding.selector.method !== null) {
        throw new Error(
          `physical WebSocket gateway entry ${binding.gatewayEntryKey} requires method null`
        );
      }
      physical.push({ binding, entry });
      continue;
    }
    if (binding.selector.method === null) {
      throw new Error(
        `WebSocket JSON-RPC gateway entry ${binding.gatewayEntryKey} requires a method`
      );
    }
    if (methodNames.has(binding.selector.method)) {
      throw new Error(
        `ServiceDeployment contains duplicate WebSocket JSON-RPC method ${JSON.stringify(binding.selector.method)}`
      );
    }
    methodNames.add(binding.selector.method);
    methods.push({ binding, entry });
  }

  for (const key of deployment.gatewayEntries.keys()) {
    if (!referenced.has(key)) {
      throw new Error(
        `ServiceDeployment gateway entry ${key} is orphaned from ingress`
      );
    }
  }
  for (const method of methods) {
    if (referenced.get(method.binding.gatewayEntryKey) !== 1) {
      throw new Error(
        `WebSocket JSON-RPC gateway entry ${method.binding.gatewayEntryKey} must have exactly one selector`
      );
    }
  }
  if (physical.length > 1) {
    throw new Error(
      'ServiceDeployment contains ambiguous physical WebSocket ingress'
    );
  }
  if (methods.length > 0 && physical.length === 0) {
    throw new Error(
      'WebSocket JSON-RPC methods require a physical WebSocket ingress'
    );
  }
  if (physical.length === 0) {
    return expected;
  }
  const physicalBinding = physical[0]!;
  if (referenced.get(physicalBinding.binding.gatewayEntryKey) !== 1) {
    throw new Error(
      'physical WebSocket gateway entry must have exactly one selector'
    );
  }
  const physicalSelector = physicalBinding.binding.selector;
  if (
    physicalSelector.protocol !== 'webSocket' ||
    physicalSelector.method !== null
  ) {
    throw new Error('physical WebSocket selector is invalid');
  }
  const methodBindings = methods.map(({ binding, entry }) => {
    if (
      binding.selector.protocol !== 'webSocket' ||
      binding.selector.path !== physicalSelector.path
    ) {
      throw new Error(
        `WebSocket JSON-RPC gateway entry ${binding.gatewayEntryKey} is orphaned from its physical path`
      );
    }
    if (!physicalBinding.entry.rpcProfiles.includes(entry.profile)) {
      throw new Error(
        `WebSocket JSON-RPC gateway entry ${binding.gatewayEntryKey} uses an unsupported physical profile`
      );
    }
    return methodBindingFromDecoded({
      methodSelector: binding.selector,
      entry,
      deployment: deployment.ref,
      gatewayEntryKey: binding.gatewayEntryKey,
      websocketEntryId: physicalBinding.entry.websocketEntryId,
      ...(deployment.timeoutMs === undefined
        ? {}
        : { timeoutMs: deployment.timeoutMs })
    });
  });
  const methodTable = new RuntimeAssemblyWebSocketMethodTable(methodBindings);
  const attachBinding: RuntimeAssemblyIngressBinding = {
    selector: {
      protocol: 'webSocket',
      method: null,
      path: physicalSelector.path
    },
    deployment: deployment.ref,
    gatewayEntryKey: physicalBinding.binding.gatewayEntryKey,
    gatewayEntryIdentity: physicalBinding.entry.gatewayEntryIdentity,
    adapterKind: 'websocketConnect',
    operationMode: 'unary',
    ...(physicalBinding.entry.handler === undefined
      ? {}
      : { handler: physicalBinding.entry.handler }),
    websocketEntryId: physicalBinding.entry.websocketEntryId,
    websocketRpcProfiles: Object.freeze([
      ...physicalBinding.entry.rpcProfiles
    ]),
    websocketMethods: methodTable,
    ...(deployment.timeoutMs === undefined
      ? {}
      : { timeoutMs: deployment.timeoutMs })
  };
  expected.push(
    expectedIngress(
      physicalBinding.binding,
      deployment.ref,
      physicalBinding.entry,
      attachBinding
    )
  );
  for (const method of methods) {
    expected.push(
      expectedIngress(method.binding, deployment.ref, method.entry)
    );
  }
  return expected;
}

function expectedIngress(
  binding: DecodedDeploymentIngressBinding,
  deployment: RuntimeAssemblyDeploymentRef,
  entry: DecodedDeploymentGatewayEntry,
  attachBinding?: RuntimeAssemblyIngressBinding
): ExpectedRuntimeAssemblyIngress {
  return {
    selector: binding.selector,
    deployment,
    gatewayEntryKey: binding.gatewayEntryKey,
    gatewayEntryIdentity: entry.gatewayEntryIdentity,
    ...(attachBinding === undefined ? {} : { attachBinding })
  };
}

function decodeServiceDeployment(
  input: unknown,
  expected: RuntimeAssemblyDeploymentRef,
  expectedContract: RuntimeAssemblyContractRef,
  label: string
): DecodedServiceDeployment {
  const value = exactObject(input, label);
  exactFields(value, [
    'schemaVersion',
    'contract',
    'deploymentRevision',
    'deploymentArtifactIdentity',
    'implementation',
    'operationBindings',
    'packageBindings',
    'serviceSelectors',
    'gatewayEntries',
    'ingress',
    'configLiterals',
    'secretRefs',
    'stateBindings',
    'resourceBindings',
    'runtimeCapabilityBindings',
    'policy',
    'diagnosticText'
  ], label);
  if (value.schemaVersion !== 'skiff-service-deployment-v3') {
    throw new Error(`${label}.schemaVersion must be skiff-service-deployment-v3`);
  }
  const contract = decodeContractRef(value.contract, `${label}.contract`);
  const deploymentRevision = requiredString(value, 'deploymentRevision');
  const deploymentArtifactIdentity = requiredString(
    value,
    'deploymentArtifactIdentity'
  );
  if (
    contract.serviceId !== expected.serviceId ||
    contract.contractVersion !== expected.contractVersion ||
    contract.serviceProtocolIdentity !==
      expectedContract.serviceProtocolIdentity ||
    deploymentRevision !== expected.deploymentRevision ||
    deploymentArtifactIdentity !== expected.deploymentArtifactIdentity
  ) {
    throw new Error(`${label} does not match its exact ServiceDeployment reference`);
  }
  if (!DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN.test(deploymentArtifactIdentity)) {
    throw new Error(`${label}.deploymentArtifactIdentity is invalid`);
  }
  const implementationPackageId = decodeImplementationPackageId(
    value.implementation,
    `${label}.implementation`
  );
  const gatewayEntriesValue = exactObject(
    value.gatewayEntries,
    `${label}.gatewayEntries`
  );
  const gatewayEntries = new Map<string, DecodedDeploymentGatewayEntry>();
  for (const [key, entry] of Object.entries(gatewayEntriesValue)) {
    const canonicalKey = decodeRuntimeAssemblyGatewayEntryKey(
      key,
      `${label}.gatewayEntries`
    );
    gatewayEntries.set(
      canonicalKey,
      decodeDeploymentGatewayEntry(
        entry,
        `${label}.gatewayEntries.${canonicalKey}`,
        contract.serviceId,
        implementationPackageId,
        canonicalKey
      )
    );
  }
  if (!Array.isArray(value.ingress)) {
    throw new Error(`${label}.ingress must be an array`);
  }
  const ingress = value.ingress.map((entry, index) =>
    decodeDeploymentIngressBinding(entry, `${label}.ingress[${index}]`)
  );
  assertUniqueSelectors(ingress, `${label}.ingress`);
  const timeoutMs = decodeDeploymentPolicy(value.policy, `${label}.policy`);
  const computedDeploymentIdentity =
    deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(value);
  if (deploymentArtifactIdentity !== computedDeploymentIdentity) {
    throw new Error(
      `${label}.deploymentArtifactIdentity does not match its current preimage`
    );
  }
  return {
    ref: expected,
    gatewayEntries,
    ingress,
    ...(timeoutMs === undefined ? {} : { timeoutMs })
  };
}

function decodeDeploymentGatewayEntry(
  input: unknown,
  label: string,
  serviceId: string,
  implementationPackageId: string,
  gatewayEntryKey: string
): DecodedDeploymentGatewayEntry {
  if (isRuntimeAssemblyWebSocketProtocolKind(input)) {
    return decodeRuntimeAssemblyWebSocketGatewayEntry({
      value: input,
      label,
      serviceId,
      implementationPackageId,
      gatewayEntryKey
    });
  }
  const value = exactObject(input, label);
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
  const gatewayEntryIdentity = requiredString(value, 'gatewayEntryIdentity');
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
  const errorProjection = exactObject(
    protocolSurface.externalErrorProjection,
    `${label}.protocolSurface.externalErrorProjection`
  );
  exactFields(
    errorProjection,
    ['kind', 'version'],
    `${label}.protocolSurface.externalErrorProjection`
  );
  if (errorProjection.kind !== 'fixed' || errorProjection.version !== 'v1') {
    throw new Error(`${label}.protocolSurface external error projection is invalid`);
  }
  const protocol = exactObject(
    protocolSurface.protocol,
    `${label}.protocolSurface.protocol`
  );
  exactFields(protocol, ['kind', 'surface'], `${label}.protocolSurface.protocol`);
  const surface = exactObject(
    protocol.surface,
    `${label}.protocolSurface.protocol.surface`
  );
  if (protocol.kind !== 'http') {
    throw new Error(`${label}.protocolSurface protocol kind is invalid`);
  }
  const handler = requiredString(value, 'handler');
  optionalNullableString(value.pre, `${label}.pre`);
  optionalNullableString(value.guard, `${label}.guard`);
  exactFields(
    surface,
    [
      'adapterKind',
      'dispatchMode',
      'externalSources',
      'requestBodySchema',
      'responseSchema',
      'streamItemSchema'
    ],
    `${label}.protocolSurface.protocol.surface`
  );
  if (surface.adapterKind !== 'rawHttp' && surface.adapterKind !== 'typedJson') {
    throw new Error(`${label}.protocolSurface adapterKind is invalid`);
  }
  if (surface.dispatchMode !== 'unary' && surface.dispatchMode !== 'serverStream') {
    throw new Error(`${label}.protocolSurface dispatchMode is invalid`);
  }
  if (surface.adapterKind === 'typedJson' && surface.dispatchMode === 'serverStream') {
    throw new Error(`${label} typedJson gateway entry cannot be serverStream`);
  }
  if (!Array.isArray(surface.externalSources)) {
    throw new Error(`${label}.protocolSurface externalSources must be an array`);
  }
  for (const [index, sourceInput] of surface.externalSources.entries()) {
    decodeExternalSource(
      sourceInput,
      `${label}.protocolSurface.protocol.surface.externalSources[${index}]`
    );
  }
  if (
    (surface.dispatchMode === 'unary' && surface.streamItemSchema !== null) ||
    (surface.dispatchMode === 'serverStream' && surface.streamItemSchema === null)
  ) {
    throw new Error(`${label}.protocolSurface stream schema does not match dispatchMode`);
  }
  if (
    surface.adapterKind === 'rawHttp' &&
    (surface.requestBodySchema !== null || surface.responseSchema !== null)
  ) {
    throw new Error(`${label}.protocolSurface rawHttp must not carry typed schemas`);
  }
  if (
    surface.adapterKind === 'typedJson' &&
    (surface.requestBodySchema === null || surface.responseSchema === null)
  ) {
    throw new Error(`${label}.protocolSurface typedJson requires request/response schemas`);
  }

  const adapterPlan = exactObject(value.adapterPlan, `${label}.adapterPlan`);
  exactFields(adapterPlan, ['kind', 'args'], `${label}.adapterPlan`);
  if (adapterPlan.kind !== surface.adapterKind) {
    throw new Error(`${label}.adapterPlan.kind does not match protocol adapterKind`);
  }
  if (!Array.isArray(adapterPlan.args)) {
    throw new Error(`${label}.adapterPlan.args must be an array`);
  }
  for (const [index, argumentInput] of adapterPlan.args.entries()) {
    const argument = exactObject(
      argumentInput,
      `${label}.adapterPlan.args[${index}]`
    );
    exactFields(argument, ['param', 'source'], `${label}.adapterPlan.args[${index}]`);
    requiredString(argument, 'param');
    decodeExternalSource(
      argument.source,
      `${label}.adapterPlan.args[${index}].source`
    );
  }
  if (
    gatewayEntryIdentity !==
    deriveCurrentRuntimeAssemblyGatewayEntryIdentity(protocolSurface)
  ) {
    throw new Error(
      `${label}.gatewayEntryIdentity does not match its current surface`
    );
  }
  return {
    kind: 'http',
    gatewayEntryIdentity,
    adapterKind: surface.adapterKind,
    operationMode: surface.dispatchMode,
    handler
  };
}

function decodeExternalSource(input: unknown, label: string): void {
  const value = exactObject(input, label);
  exactFields(value, ['kind'], label);
  if (
    value.kind !== 'http.request' &&
    value.kind !== 'http.body' &&
    value.kind !== 'http.context'
  ) {
    throw new Error(`${label}.kind is invalid`);
  }
}

function decodeDeploymentIngressBinding(
  input: unknown,
  label: string
): DecodedDeploymentIngressBinding {
  const value = exactObject(input, label);
  exactFields(value, ['selector', 'gatewayEntryKey'], label);
  return {
    selector: decodeRuntimeAssemblyIngressSelector(
      value.selector,
      `${label}.selector`
    ),
    gatewayEntryKey: decodeRuntimeAssemblyGatewayEntryKey(
      value.gatewayEntryKey,
      label
    )
  };
}

function decodeDeploymentPolicy(input: unknown, label: string): number | undefined {
  const value = exactObject(input, label);
  exactFieldsWithOptional(
    value,
    ['resources', 'activation', 'principal'],
    ['timeoutMs'],
    label
  );
  requiredString(value, 'principal');
  const resources = exactObject(value.resources, `${label}.resources`);
  exactFields(resources, ['cpuMillis', 'memoryBytes'], `${label}.resources`);
  positiveSafeInteger(resources.cpuMillis, `${label}.resources.cpuMillis`);
  positiveSafeInteger(resources.memoryBytes, `${label}.resources.memoryBytes`);
  const activation = exactObject(value.activation, `${label}.activation`);
  exactFields(
    activation,
    ['maxConcurrency', 'idleTimeoutMs'],
    `${label}.activation`
  );
  positiveSafeInteger(
    activation.maxConcurrency,
    `${label}.activation.maxConcurrency`
  );
  if (activation.idleTimeoutMs !== null) {
    positiveSafeInteger(
      activation.idleTimeoutMs,
      `${label}.activation.idleTimeoutMs`
    );
  }
  if (!Object.hasOwn(value, 'timeoutMs')) {
    return undefined;
  }
  return positiveSafeInteger(value.timeoutMs, `${label}.timeoutMs`);
}

function decodeImplementationPackageId(input: unknown, label: string): string {
  const value = exactObject(input, label);
  exactFields(
    value,
    [
      'packageId',
      'packageVersion',
      'packageBuildId',
      'packageLocalAbiIdentity'
    ],
    label
  );
  const packageId = requiredString(value, 'packageId');
  requiredString(value, 'packageVersion');
  const packageBuildId = requiredString(value, 'packageBuildId');
  if (!PACKAGE_BUILD_IDENTITY_PATTERN.test(packageBuildId)) {
    throw new Error(`${label}.packageBuildId is invalid`);
  }
  const packageLocalAbiIdentity = requiredString(
    value,
    'packageLocalAbiIdentity'
  );
  if (!PACKAGE_LOCAL_ABI_IDENTITY_PATTERN.test(packageLocalAbiIdentity)) {
    throw new Error(`${label}.packageLocalAbiIdentity is invalid`);
  }
  return packageId;
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

function contractCoordinate(serviceId: string, contractVersion: string): string {
  return `${serviceId}\u0000${contractVersion}`;
}

function deploymentRefEquals(
  left: RuntimeAssemblyDeploymentRef,
  right: RuntimeAssemblyDeploymentRef
): boolean {
  return deploymentRefKey(left) === deploymentRefKey(right);
}

function selectorEquals(
  left: RuntimeAssemblyIngressSelector,
  right: RuntimeAssemblyIngressSelector
): boolean {
  return (
    left.protocol === right.protocol &&
    left.method === right.method &&
    left.path === right.path
  );
}

function scopedSelectorKey(
  deployment: RuntimeAssemblyDeploymentRef,
  selector: RuntimeAssemblyIngressSelector
): string {
  return `${contractCoordinate(deployment.serviceId, deployment.contractVersion)}\u0000${runtimeAssemblyIngressKey(selector)}`;
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

function exactFieldsWithOptional(
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[],
  label: string
): void {
  const actual = Object.keys(value);
  const allowed = new Set([...required, ...optional]);
  if (
    required.some((field) => !Object.hasOwn(value, field)) ||
    actual.some((field) => !allowed.has(field))
  ) {
    throw new Error(
      `${label} fields must contain ${[...required].sort().join(',')} and only optional ${[...optional].sort().join(',')}`
    );
  }
}

function requiredString(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== 'string' || fieldValue.length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return fieldValue;
}

function optionalNullableString(value: unknown, label: string): void {
  if (value !== null && (typeof value !== 'string' || value.length === 0)) {
    throw new Error(`${label} must be null or a non-empty string`);
  }
}

function positiveSafeInteger(value: unknown, label: string): number {
  if (!Number.isSafeInteger(value) || (value as number) <= 0) {
    throw new Error(`${label} must be a positive safe integer`);
  }
  return value as number;
}
