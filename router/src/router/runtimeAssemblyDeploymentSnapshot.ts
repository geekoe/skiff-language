import { createHash } from 'node:crypto';

import { stableStringify } from '../manifest/identity.js';
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

const DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN =
  /^skiff-deployment-artifact-v2:sha256:[0-9a-f]{64}$/;
const GATEWAY_ENTRY_IDENTITY_PATTERN =
  /^skiff-gateway-entry-v1:sha256:[0-9a-f]{64}$/;
const SERVICE_PROTOCOL_IDENTITY_PATTERN =
  /^skiff-service-protocol-v5:sha256:[0-9a-f]{64}$/;
const WEBSOCKET_ENTRY_IDENTITY_PREFIX =
  'skiff-websocket-entry-v1:sha256:';
const WEBSOCKET_GATEWAY_ENTRY_KEY = 'websocket';

interface DecodedServiceDeployment {
  ref: RuntimeAssemblyDeploymentRef;
  gatewayEntries: ReadonlyMap<string, DecodedDeploymentGatewayEntry>;
  ingress: readonly DecodedDeploymentIngressBinding[];
  timeoutMs?: number;
}

interface DecodedDeploymentGatewayEntry {
  gatewayEntryIdentity: string;
  adapterKind: 'rawHttp' | 'typedJson' | 'websocketConnect';
  operationMode: 'unary' | 'serverStream';
  handler?: string;
  websocketEntryId?: string;
}

interface DecodedDeploymentIngressBinding {
  selector: RuntimeAssemblyIngressSelector;
  gatewayEntryKey: string;
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
  const expectedBySelector = new Map<string, RuntimeAssemblyIngressBinding>();
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
    for (const binding of deployment.ingress) {
      const selectorKey = runtimeAssemblyIngressKey(binding.selector);
      if (expectedBySelector.has(selectorKey)) {
        throw new Error(
          `ServiceDeployment ingress contains duplicate selector ${selectorKey}`
        );
      }
      const entry = deployment.gatewayEntries.get(binding.gatewayEntryKey);
      if (entry === undefined) {
        throw new Error(
          `ServiceDeployment ingress references missing gateway entry ${binding.gatewayEntryKey}`
        );
      }
      expectedBySelector.set(selectorKey, {
        selector: binding.selector,
        deployment: deployment.ref,
        gatewayEntryKey: binding.gatewayEntryKey,
        gatewayEntryIdentity: entry.gatewayEntryIdentity,
        adapterKind: entry.adapterKind,
        operationMode: entry.operationMode,
        ...(entry.adapterKind !== 'websocketConnect' ||
        entry.handler === undefined
          ? {}
          : { handler: entry.handler }),
        ...(entry.websocketEntryId === undefined
          ? {}
          : { websocketEntryId: entry.websocketEntryId }),
        ...(deployment.timeoutMs === undefined
          ? {}
          : { timeoutMs: deployment.timeoutMs })
      });
    }
  }

  const gatewayIngress = record.gatewayIngress.map((declared) => {
    const selectorKey = runtimeAssemblyIngressKey(declared.selector);
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
    return expected;
  });
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
  if (value.schemaVersion !== 'skiff-service-deployment-v2') {
    throw new Error(`${label}.schemaVersion must be skiff-service-deployment-v2`);
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
        canonicalKey
      )
    );
  }
  const websocketEntries = Array.from(gatewayEntries.entries()).filter(
    ([, entry]) => entry.adapterKind === 'websocketConnect'
  );
  if (websocketEntries.length > 1) {
    throw new Error(`${label}.gatewayEntries must contain at most one WebSocket entry`);
  }
  if (
    websocketEntries[0] !== undefined &&
    websocketEntries[0][0] !== WEBSOCKET_GATEWAY_ENTRY_KEY
  ) {
    throw new Error(
      `${label}.gatewayEntries WebSocket entry key must be ${WEBSOCKET_GATEWAY_ENTRY_KEY}`
    );
  }
  if (!Array.isArray(value.ingress)) {
    throw new Error(`${label}.ingress must be an array`);
  }
  const ingress = value.ingress.map((entry, index) =>
    decodeDeploymentIngressBinding(entry, `${label}.ingress[${index}]`)
  );
  assertUniqueSelectors(ingress, `${label}.ingress`);
  for (const binding of ingress) {
    const gatewayEntry = gatewayEntries.get(binding.gatewayEntryKey);
    if (gatewayEntry === undefined) {
      throw new Error(
        `${label}.ingress references missing gateway entry ${binding.gatewayEntryKey}`
      );
    }
    if (
      (binding.selector.protocol === 'webSocket') !==
      (gatewayEntry.adapterKind === 'websocketConnect')
    ) {
      throw new Error(
        `${label}.ingress selector protocol does not match gateway entry ${binding.gatewayEntryKey}`
      );
    }
  }
  const websocketIngress = ingress.filter(
    (binding) => binding.selector.protocol === 'webSocket'
  );
  if (
    websocketIngress.length !== websocketEntries.length ||
    (websocketIngress[0] !== undefined &&
      websocketIngress[0].gatewayEntryKey !== websocketEntries[0]?.[0])
  ) {
    throw new Error(
      `${label} WebSocket ingress must exactly join its sole compiler-owned gateway entry`
    );
  }
  const timeoutMs = decodeDeploymentPolicy(value.policy, `${label}.policy`);
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
  gatewayEntryKey: string
): DecodedDeploymentGatewayEntry {
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
  if (protocol.kind === 'websocketConnect') {
    return decodeWebSocketDeploymentGatewayEntry({
      value,
      surface,
      label,
      gatewayEntryIdentity,
      serviceId,
      gatewayEntryKey
    });
  }
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
  return {
    gatewayEntryIdentity,
    adapterKind: surface.adapterKind,
    operationMode: surface.dispatchMode,
    handler
  };
}

function decodeWebSocketDeploymentGatewayEntry(input: {
  value: Record<string, unknown>;
  surface: Record<string, unknown>;
  label: string;
  gatewayEntryIdentity: string;
  serviceId: string;
  gatewayEntryKey: string;
}): DecodedDeploymentGatewayEntry {
  const {
    value,
    surface,
    label,
    gatewayEntryIdentity,
    serviceId,
    gatewayEntryKey
  } = input;
  exactFields(
    surface,
    [
      'connectRequestShape',
      'connectResultShape',
      'connectionPolicyShape',
      'externalSources',
      'downlinkFrames'
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
  if (
    !Array.isArray(surface.externalSources) ||
    surface.externalSources.length !== 2
  ) {
    throw new Error(
      `${label}.protocolSurface WebSocket externalSources must be the exact fixed pair`
    );
  }
  const expectedSources = [
    'websocket.connectRequest',
    'websocket.connectionId'
  ];
  for (const [index, expectedKind] of expectedSources.entries()) {
    const source = exactObject(
      surface.externalSources[index],
      `${label}.protocolSurface.protocol.surface.externalSources[${index}]`
    );
    exactFields(
      source,
      ['kind'],
      `${label}.protocolSurface.protocol.surface.externalSources[${index}]`
    );
    if (source.kind !== expectedKind) {
      throw new Error(
        `${label}.protocolSurface WebSocket externalSources must be the exact fixed pair`
      );
    }
  }
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
  if (value.pre !== null || value.guard !== null) {
    throw new Error(`${label} WebSocket entry cannot declare pre or guard`);
  }
  const handler =
    value.handler === null
      ? undefined
      : requiredString(value, 'handler');

  const adapterPlan = exactObject(value.adapterPlan, `${label}.adapterPlan`);
  exactFields(adapterPlan, ['kind', 'args'], `${label}.adapterPlan`);
  if (adapterPlan.kind !== 'websocketConnect') {
    throw new Error(`${label}.adapterPlan.kind must be websocketConnect`);
  }
  if (!Array.isArray(adapterPlan.args)) {
    throw new Error(`${label}.adapterPlan.args must be an array`);
  }
  if (handler === undefined && adapterPlan.args.length !== 0) {
    throw new Error(`${label} handler-absent WebSocket entry must have no adapter args`);
  }
  const params = new Set<string>();
  for (const [index, argumentInput] of adapterPlan.args.entries()) {
    const argumentLabel = `${label}.adapterPlan.args[${index}]`;
    const argument = exactObject(argumentInput, argumentLabel);
    exactFields(argument, ['param', 'source'], argumentLabel);
    const param = requiredString(argument, 'param');
    if (params.has(param)) {
      throw new Error(`${label}.adapterPlan.args contains duplicate param ${param}`);
    }
    params.add(param);
    const source = exactObject(argument.source, `${argumentLabel}.source`);
    exactFields(source, ['kind'], `${argumentLabel}.source`);
    if (
      source.kind !== 'websocket.connectRequest' &&
      source.kind !== 'websocket.connectionId'
    ) {
      throw new Error(`${argumentLabel}.source.kind is invalid for websocketConnect`);
    }
  }

  return {
    gatewayEntryIdentity,
    adapterKind: 'websocketConnect',
    operationMode: 'unary',
    ...(handler === undefined ? {} : { handler }),
    websocketEntryId: deriveWebSocketEntryId(serviceId, gatewayEntryKey)
  };
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
    left.host === right.host &&
    left.method === right.method &&
    left.path === right.path
  );
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
