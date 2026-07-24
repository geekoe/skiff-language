import { readFile, realpath } from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';

import type { RuntimeAssemblyRef } from '../protocol/assemblyActivationProtocol.js';
import { parseStrictActivationJson } from '../protocol/strictActivationJson.js';
import { sha256Hex, stableStringify } from '../manifest/identity.js';
import {
  decodeRouterSnapshot,
  type LoadedRuntimeAssembly,
  type RuntimeAssemblySnapshotLoader
} from './runtimeAssemblySnapshot.js';

const MAX_RECORD_BYTES = 64 * 1024 * 1024;
const ASSEMBLY_IDENTITY = /^skiff-runtime-assembly-v1:sha256:([0-9a-f]{64})$/;
const SERVICE_PROTOCOL_IDENTITY = /^skiff-service-protocol-v2:sha256:([0-9a-f]{64})$/;
const SERVICE_ID = /^[a-z0-9_.-]+(?:\/[a-z0-9_.-]+)*$/;
const VERSION = /^[A-Za-z0-9_.-]{1,200}$/;

export class FilesystemRuntimeAssemblySnapshotLoader
implements RuntimeAssemblySnapshotLoader {
  private readonly artifactsPath: string;

  constructor(artifactsPath: string) {
    if (!isAbsolute(artifactsPath)) {
      throw new Error('filesystem RuntimeAssembly loader requires an absolute artifactsPath');
    }
    this.artifactsPath = resolve(artifactsPath);
  }

  async load(ref: RuntimeAssemblyRef): Promise<LoadedRuntimeAssembly> {
    const match = ASSEMBLY_IDENTITY.exec(ref.assemblyIdentity);
    if (match === null) {
      throw new Error('RuntimeAssembly reference identity is invalid');
    }
    const assembly = await this.readRecord(
      `records/runtime-assemblies/${match[1]}.json`,
      'RuntimeAssembly'
    );
    const assemblyObject = record(assembly, 'RuntimeAssembly');
    if (assemblyObject.assemblyIdentity !== ref.assemblyIdentity) {
      throw new Error('RuntimeAssembly record identity does not match its canonical path');
    }
    const computedAssemblyIdentity = computeRuntimeAssemblyIdentity(assemblyObject);
    if (computedAssemblyIdentity !== ref.assemblyIdentity) {
      throw new Error('RuntimeAssembly record content does not match its declared identity');
    }
    if (!Array.isArray(assemblyObject.resolvedContracts)) {
      throw new Error('RuntimeAssembly.resolvedContracts must be an array');
    }
    const serviceContracts = await Promise.all(
      assemblyObject.resolvedContracts.map(async (input, index) => {
        const contract = record(input, `RuntimeAssembly.resolvedContracts[${index}]`);
        const serviceId = requiredString(contract, 'serviceId');
        const contractVersion = requiredString(contract, 'contractVersion');
        const protocol = requiredString(contract, 'serviceProtocolIdentity');
        const protocolMatch = SERVICE_PROTOCOL_IDENTITY.exec(protocol);
        if (
          !SERVICE_ID.test(serviceId) ||
          serviceId.includes('..') ||
          !VERSION.test(contractVersion) ||
          contractVersion === '.' ||
          contractVersion === '..' ||
          protocolMatch === null
        ) {
          throw new Error(
            `RuntimeAssembly.resolvedContracts[${index}] is not a canonical contract reference`
          );
        }
        const encodedService = serviceId.replaceAll('.', '~d').replaceAll('/', '~s');
        const loaded = await this.readRecord(
          `records/service-contracts/${encodedService}/${contractVersion}/${protocolMatch[1]}.json`,
          `ServiceContract ${serviceId}@${contractVersion}`
        );
        const loadedObject = record(loaded, 'ServiceContract');
        if (
          loadedObject.serviceId !== serviceId ||
          loadedObject.contractVersion !== contractVersion ||
          loadedObject.serviceProtocolIdentity !== protocol
        ) {
          throw new Error(
            `ServiceContract ${serviceId}@${contractVersion} identity does not match its canonical path`
          );
        }
        const computedProtocolIdentity = computeServiceProtocolIdentity(loadedObject);
        if (computedProtocolIdentity !== protocol) {
          throw new Error(
            `ServiceContract ${serviceId}@${contractVersion} content does not match its declared identity`
          );
        }
        return loaded;
      })
    );
    return decodeRouterSnapshot({ assembly, serviceContracts }, ref).assembly;
  }

  private async readRecord(relativePath: string, label: string): Promise<unknown> {
    const root = await realpath(this.artifactsPath);
    const candidate = resolve(root, relativePath);
    assertContained(root, candidate, label);
    let canonical: string;
    try {
      canonical = await realpath(candidate);
    } catch (error) {
      throw new Error(`${label} record is unavailable at ${relativePath}`, { cause: error });
    }
    assertContained(root, canonical, label);
    const bytes = await readFile(canonical);
    if (bytes.byteLength === 0 || bytes.byteLength > MAX_RECORD_BYTES) {
      throw new Error(`${label} record has an invalid bounded size`);
    }
    try {
      return parseStrictActivationJson(bytes);
    } catch (error) {
      throw new Error(`${label} record is not strict JSON`, { cause: error });
    }
  }
}

function computeRuntimeAssemblyIdentity(value: Record<string, unknown>): string {
  const projection = {
    schema: 'skiff-runtime-assembly-identity-v1',
    roots: value.roots,
    resolvedDeployments: value.resolvedDeployments,
    resolvedContracts: value.resolvedContracts,
    resolvedPackages: value.resolvedPackages,
    packageLinkPlan: value.packageLinkPlan,
    serviceBindingTemplates: value.serviceBindingTemplates,
    activationTemplates: value.activationTemplates,
    globalIngress: value.globalIngress
  };
  return `skiff-runtime-assembly-v1:sha256:${sha256Hex(stableStringify(projection))}`;
}

function computeServiceProtocolIdentity(value: Record<string, unknown>): string {
  const projection = {
    schema: 'skiff-service-protocol-identity-v2',
    serviceId: value.serviceId,
    contractVersion: value.contractVersion,
    operations: value.operations,
    boundarySchema: value.boundarySchema
  };
  return `skiff-service-protocol-v2:sha256:${sha256Hex(stableStringify(projection))}`;
}

function assertContained(root: string, candidate: string, label: string): void {
  const path = relative(root, candidate);
  if (path === '..' || path.startsWith(`..${sep}`) || isAbsolute(path)) {
    throw new Error(`${label} record escapes artifactsPath`);
  }
}

function record(input: unknown, label: string): Record<string, unknown> {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error(`${label} must be an object`);
  }
  return input as Record<string, unknown>;
}

function requiredString(input: Record<string, unknown>, field: string): string {
  const value = input[field];
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  return value;
}
