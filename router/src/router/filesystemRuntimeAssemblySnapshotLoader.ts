import { readFile, realpath } from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';

import type { RuntimeAssemblyRef } from '../protocol/assemblyActivationProtocol.js';
import { parseStrictJson } from '../protocol/strictJson.js';
import { sha256Hex, stableStringify } from '../manifest/identity.js';
import {
  decodeRouterSnapshot,
  type LoadedRuntimeAssembly,
  type RuntimeAssemblyActorMethod,
  type RuntimeAssemblySnapshotLoader
} from './runtimeAssemblySnapshot.js';

const MAX_RECORD_BYTES = 64 * 1024 * 1024;
const ASSEMBLY_IDENTITY = /^skiff-runtime-assembly-v1:sha256:([0-9a-f]{64})$/;
const SERVICE_PROTOCOL_IDENTITY = /^skiff-service-protocol-v3:sha256:([0-9a-f]{64})$/;
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
    const decoded = decodeRouterSnapshot({ assembly, serviceContracts }, ref).assembly;
    const actorMethods = await this.loadActorMethods(assemblyObject);
    return actorMethods.length === 0 ? decoded : { ...decoded, actorMethods };
  }

  private async loadActorMethods(
    assembly: Record<string, unknown>
  ): Promise<RuntimeAssemblyActorMethod[]> {
    const plan = record(assembly.packageLinkPlan, 'RuntimeAssembly.packageLinkPlan');
    if (!Array.isArray(plan.codeSlots)) return [];
    const methods: RuntimeAssemblyActorMethod[] = [];
    for (const [codeSlot, rawSlot] of plan.codeSlots.entries()) {
      const slot = record(rawSlot, `RuntimeAssembly.packageLinkPlan.codeSlots[${codeSlot}]`);
      const implementation = record(slot.package, 'PackageCodeSlot.package');
      const packageId = requiredString(implementation, 'packageId');
      const packageVersion = requiredString(implementation, 'packageVersion');
      const packageBuildId = requiredString(implementation, 'packageBuildId');
      const buildHash = identityHash(
        packageBuildId,
        'skiff-package-build-v4:sha256:',
        'packageBuildId'
      );
      const packageRecord = record(
        await this.readRecord(
          `records/package-artifacts/${coordinate(packageId)}/${packageVersion}/${buildHash}/package.json`,
          `PackageArtifact ${packageId}@${packageVersion}`
        ),
        'PackageArtifact'
      );
      if (!Array.isArray(packageRecord.files)) continue;
      for (const [fileIndex, rawFile] of packageRecord.files.entries()) {
        const fileRef = record(rawFile, `PackageArtifact.files[${fileIndex}]`);
        const fileIdentity = requiredString(fileRef, 'fileIrIdentity');
        const fileHash = identityHash(
          fileIdentity,
          'skiff-file-ir-v5:sha256:',
          'fileIrIdentity'
        );
        const file = record(
          await this.readRecord(
            `records/package-artifacts/${coordinate(packageId)}/${packageVersion}/${buildHash}/file-ir/${fileHash}.json`,
            `FileIr ${fileIdentity}`
          ),
          'FileIr'
        );
        if (!Array.isArray(file.actorDeclarations)) continue;
        for (const rawActor of file.actorDeclarations) {
          const actor = record(rawActor, 'ActorDeclaration');
          const abi = record(actor.abi, 'ActorDeclaration.abi');
          const actorSymbol = requiredString(abi, 'actorName');
          const actorAbiIdentity = requiredString(actor, 'actorAbiIdentity');
          const actorImplementationIdentity = requiredString(
            actor,
            'actorImplementationIdentity'
          );
          const implementations = record(
            actor.methodImplementations,
            'ActorDeclaration.methodImplementations'
          );
          for (const methodIdentity of Object.keys(implementations)) {
            methods.push({
              declarationOwner: {
                unit: { kind: 'package', value: codeSlot },
                file: { kind: 'loadedFileIndex', value: fileIndex },
                actorSymbol,
              },
              actorAbiIdentity,
              actorImplementationIdentity,
              methodIdentity,
            });
          }
        }
      }
    }
    return methods;
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
      return parseStrictJson(bytes);
    } catch (error) {
      throw new Error(`${label} record is not strict JSON`, { cause: error });
    }
  }
}

function coordinate(value: string): string {
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function identityHash(value: string, prefix: string, label: string): string {
  if (!value.startsWith(prefix) || !/^[0-9a-f]{64}$/.test(value.slice(prefix.length))) {
    throw new Error(`${label} is invalid`);
  }
  return value.slice(prefix.length);
}

function computeRuntimeAssemblyIdentity(value: Record<string, unknown>): string {
  const projection = canonicalRuntimeAssemblyIdentityValue({
    schema: 'skiff-runtime-assembly-identity-v1',
    roots: value.roots,
    resolvedDeployments: value.resolvedDeployments,
    resolvedContracts: value.resolvedContracts,
    resolvedPackages: value.resolvedPackages,
    packageLinkPlan: value.packageLinkPlan,
    serviceBindingTemplates: value.serviceBindingTemplates,
    activationTemplates: value.activationTemplates,
    globalIngress: value.globalIngress
  });
  return `skiff-runtime-assembly-v1:sha256:${sha256Hex(stableStringify(projection))}`;
}

function canonicalRuntimeAssemblyIdentityValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value
      .map(canonicalRuntimeAssemblyIdentityValue)
      .sort((left, right) => {
        const leftJson = stableStringify(left);
        const rightJson = stableStringify(right);
        return leftJson < rightJson ? -1 : leftJson > rightJson ? 1 : 0;
      });
  }
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([key]) =>
          !['packageVersion', 'contractVersion', 'exactVersion'].includes(key))
        .map(([key, item]) => [key, canonicalRuntimeAssemblyIdentityValue(item)])
    );
  }
  return value;
}

function computeServiceProtocolIdentity(value: Record<string, unknown>): string {
  const projection = {
    schema: 'skiff-service-protocol-identity-v3',
    serviceId: value.serviceId,
    operations: value.operations,
    packageTypeRequirements: value.packageTypeRequirements
  };
  return `skiff-service-protocol-v3:sha256:${sha256Hex(stableStringify(projection))}`;
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
