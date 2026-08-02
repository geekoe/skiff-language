import { readFile, realpath } from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';

import type { RuntimeAssemblyRef } from '../protocol/assemblyActivationProtocol.js';
import { parseStrictJson } from '../protocol/strictJson.js';
import { joinRuntimeAssemblyDeployments } from './runtimeAssemblyDeploymentSnapshot.js';
import {
  ACTOR_ROUTING_PROJECTION_RECORD_PATH,
  decodeActorRoutingProjectionRecord,
  MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES,
} from './actorRoutingProjection.js';
import {
  decodeRuntimeAssemblyRecord,
  type LoadedRuntimeAssembly,
  type RuntimeAssemblyActorMethod,
  type RuntimeAssemblyDeploymentRef,
  type RuntimeAssemblySnapshotLoader
} from './runtimeAssemblySnapshot.js';

const MAX_RECORD_BYTES = 64 * 1024 * 1024;
const ASSEMBLY_IDENTITY = /^skiff-runtime-assembly-v3:sha256:([0-9a-f]{64})$/;

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
    const recordSurface = decodeRuntimeAssemblyRecord(assembly, ref);
    const serviceDeployments = await Promise.all(
      recordSurface.resolvedDeployments.map((deployment, index) =>
        this.loadServiceDeployment(deployment, index)
      )
    );
    const decoded = joinRuntimeAssemblyDeployments(recordSurface, serviceDeployments);
    const actorMethods = await this.loadActorMethods();
    return actorMethods.length === 0 ? decoded : { ...decoded, actorMethods };
  }

  private async loadServiceDeployment(
    reference: RuntimeAssemblyDeploymentRef,
    index: number
  ): Promise<unknown> {
    const service = coordinate(reference.serviceId, 'serviceId');
    const contractVersion = safeSegment(
      reference.contractVersion,
      'contractVersion'
    );
    const revision = safeSegment(
      reference.deploymentRevision,
      'deploymentRevision'
    );
    const identity = identityHash(
      reference.deploymentArtifactIdentity,
      'skiff-deployment-artifact-v4:sha256:',
      'deploymentArtifactIdentity'
    );
    return await this.readRecord(
      `records/service-deployments/${service}/${contractVersion}/${revision}/${identity}.json`,
      `ServiceDeployment resolvedDeployments[${index}]`
    );
  }

  /**
   * Loads the actor method catalog strictly from the canonical actor routing
   * projection record (A0 §2). PackageArtifact / File IR / source / payload are
   * never read for actor catalog construction.
   */
  private async loadActorMethods(): Promise<RuntimeAssemblyActorMethod[]> {
    const bytes = await this.readRecordBytes(
      ACTOR_ROUTING_PROJECTION_RECORD_PATH,
      'actor routing projection',
      MAX_ACTOR_ROUTING_PROJECTION_RECORD_BYTES
    );
    const projection = decodeActorRoutingProjectionRecord(bytes);
    return projection.methods.map((method) => ({
      actor: {
        serviceId: method.actor.serviceId,
        actorAbiIdentity: method.actor.actorAbiIdentity,
      },
      actorImplementationIdentity: method.actorImplementationIdentity,
      methodIdentity: method.methodIdentity,
      deployment: method.deployment,
      package: method.package,
    }));
  }

  private async readRecord(
    relativePath: string,
    label: string,
    maxBytes = MAX_RECORD_BYTES
  ): Promise<unknown> {
    const bytes = await this.readRecordBytes(relativePath, label, maxBytes);
    try {
      return parseStrictJson(bytes);
    } catch (error) {
      throw new Error(`${label} record is not strict JSON`, { cause: error });
    }
  }

  private async readRecordBytes(
    relativePath: string,
    label: string,
    maxBytes: number
  ): Promise<Uint8Array> {
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
    if (bytes.byteLength === 0 || bytes.byteLength > maxBytes) {
      throw new Error(`${label} record has an invalid bounded size`);
    }
    return bytes;
  }
}

function coordinate(value: string, label: string): string {
  if (
    value.length === 0 ||
    value.length > 200 ||
    value !== value.trim() ||
    value.includes('~') ||
    value.includes('//') ||
    value.startsWith('/') ||
    value.endsWith('/') ||
    !/^[a-z0-9_.\/-]+$/.test(value)
  ) {
    throw new Error(`${label} is not a canonical artifact coordinate`);
  }
  return value.replaceAll('.', '~d').replaceAll('/', '~s');
}

function safeSegment(value: string, label: string): string {
  if (
    value.length === 0 ||
    value.length > 200 ||
    value !== value.trim() ||
    value === '.' ||
    value === '..' ||
    !/^[A-Za-z0-9_.-]+$/.test(value)
  ) {
    throw new Error(`${label} is not a canonical artifact segment`);
  }
  return value;
}

function identityHash(value: string, prefix: string, label: string): string {
  if (!value.startsWith(prefix) || !/^[0-9a-f]{64}$/.test(value.slice(prefix.length))) {
    throw new Error(`${label} is invalid`);
  }
  return value.slice(prefix.length);
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
