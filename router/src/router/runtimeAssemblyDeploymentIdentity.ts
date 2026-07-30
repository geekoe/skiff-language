import { createHash } from 'node:crypto';

import { stableStringify } from '../manifest/identity.js';

export function deriveCurrentRuntimeAssemblyGatewayEntryIdentity(
  protocolSurface: unknown
): string {
  return `skiff-gateway-entry-v2:sha256:${createHash('sha256')
    .update(stableStringify({
      schema: 'skiff-gateway-entry-identity-v2',
      surface: protocolSurface
    }))
    .digest('hex')}`;
}

export function deriveCurrentRuntimeAssemblyServiceDeploymentIdentity(
  input: unknown
): string {
  const value = exactObject(input, 'ServiceDeployment identity input');
  const projection = {
    schema: 'skiff-deployment-artifact-identity-v4',
    contract: withoutHumanVersionLabels(value.contract),
    deploymentRevision: value.deploymentRevision,
    implementation: withoutHumanVersionLabels(value.implementation),
    operationBindings: sortedRecords(
      value.operationBindings,
      (left, right) =>
        compareStrings(
          recordString(left, 'contractOperationId'),
          recordString(right, 'contractOperationId')
        )
    ),
    packageBindings: sortedRecords(
      withoutHumanVersionLabels(value.packageBindings),
      compareCanonicalJson
    ),
    serviceSelectors: sortedRecords(
      withoutHumanVersionLabels(value.serviceSelectors),
      compareCanonicalJson
    ),
    gatewayEntries: value.gatewayEntries,
    ingress: sortedRecords(value.ingress, compareIngressBindings),
    resourceBindings: sortedRecords(
      value.resourceBindings,
      (left, right) =>
        compareStrings(
          recordString(left, 'requirementKey'),
          recordString(right, 'requirementKey')
        )
    ),
    runtimeCapabilityBindings: sortedRecords(
      value.runtimeCapabilityBindings,
      (left, right) =>
        compareStrings(
          recordString(left, 'capability'),
          recordString(right, 'capability')
        ) ||
        compareStrings(
          recordString(left, 'version'),
          recordString(right, 'version')
        )
    )
  };
  return `skiff-deployment-artifact-v4:sha256:${createHash('sha256')
    .update(stableStringify(projection))
    .digest('hex')}`;
}

function withoutHumanVersionLabels(input: unknown): unknown {
  if (Array.isArray(input)) {
    return input.map(withoutHumanVersionLabels);
  }
  if (input !== null && typeof input === 'object') {
    const result: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(
      input as Record<string, unknown>
    )) {
      if (
        key !== 'packageVersion' &&
        key !== 'contractVersion' &&
        key !== 'exactVersion'
      ) {
        result[key] = withoutHumanVersionLabels(value);
      }
    }
    return result;
  }
  return input;
}

function sortedRecords(
  input: unknown,
  compare: (left: unknown, right: unknown) => number
): unknown[] {
  if (!Array.isArray(input)) {
    throw new Error('ServiceDeployment identity collection must be an array');
  }
  return structuredClone(input).sort(compare);
}

function compareIngressBindings(left: unknown, right: unknown): number {
  const leftSelector = exactObject(
    exactObject(left, 'ingress binding').selector,
    'ingress selector'
  );
  const rightSelector = exactObject(
    exactObject(right, 'ingress binding').selector,
    'ingress selector'
  );
  const leftProtocol = recordString(leftSelector, 'protocol');
  const rightProtocol = recordString(rightSelector, 'protocol');
  const protocolOrder =
    ingressProtocolRank(leftProtocol) - ingressProtocolRank(rightProtocol);
  if (protocolOrder !== 0) return protocolOrder;
  const methodOrder = compareOptionalStrings(
    leftSelector.method,
    rightSelector.method
  );
  if (methodOrder !== 0) return methodOrder;
  return compareStrings(
    recordString(leftSelector, 'path'),
    recordString(rightSelector, 'path')
  );
}

function ingressProtocolRank(value: string): number {
  if (value === 'http') return 0;
  if (value === 'webSocket') return 1;
  throw new Error('ServiceDeployment identity ingress protocol is invalid');
}

function compareOptionalStrings(left: unknown, right: unknown): number {
  if (left === null) return right === null ? 0 : -1;
  if (right === null) return 1;
  if (typeof left !== 'string' || typeof right !== 'string') {
    throw new Error('ServiceDeployment identity ingress method is invalid');
  }
  return compareStrings(left, right);
}

function compareCanonicalJson(left: unknown, right: unknown): number {
  return compareStrings(stableStringify(left), stableStringify(right));
}

function compareStrings(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function recordString(input: unknown, field: string): string {
  const value = exactObject(input, 'ServiceDeployment identity record')[field];
  if (typeof value !== 'string') {
    throw new Error(`ServiceDeployment identity ${field} must be a string`);
  }
  return value;
}

function exactObject(input: unknown, label: string): Record<string, unknown> {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error(`${label} must be an object`);
  }
  return input as Record<string, unknown>;
}
