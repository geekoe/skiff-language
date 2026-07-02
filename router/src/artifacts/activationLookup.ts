import type { LoadedServiceAssemblyArtifact } from "./types.js";

export interface ActivationLookupRequest {
  serviceId: string;
  target: string;
  buildId: string;
}

interface ActivationLookupEntry {
  activationIdentity: string;
  buildId: string;
}

export class ActivationLookup {
  private readonly byKey = new Map<string, ActivationLookupEntry>();

  set(input: ActivationLookupRequest & { activationIdentity: string }): void {
    const entry: ActivationLookupEntry = {
      activationIdentity: input.activationIdentity,
      buildId: input.buildId,
    };
    const key = activationLookupKey(input);
    const existing = this.byKey.get(key);
    if (
      existing !== undefined &&
      existing.activationIdentity !== input.activationIdentity
    ) {
      throw new Error(
        `multiple router config activations found for service build operation ${input.serviceId}:${input.buildId}:${input.target}`,
      );
    }
    this.byKey.set(key, entry);
  }

  get(input: ActivationLookupRequest): string | undefined {
    return this.byKey.get(activationLookupKey(input))?.activationIdentity;
  }

  get size(): number {
    return this.byKey.size;
  }
}

export function activationLookupKey(input: ActivationLookupRequest): string;
export function activationLookupKey(
  serviceId: string,
  buildId: string,
  target: string,
): string;
export function activationLookupKey(
  inputOrServiceId: ActivationLookupRequest | string,
  buildId?: string,
  target?: string,
): string {
  if (typeof inputOrServiceId === "string") {
    if (buildId === undefined) {
      throw new Error("activationLookupKey buildId is required");
    }
    if (target === undefined) {
      throw new Error("activationLookupKey target is required");
    }
    return [inputOrServiceId, buildId, target].join("\0");
  }
  return [
    inputOrServiceId.serviceId,
    inputOrServiceId.buildId,
    inputOrServiceId.target,
  ].join("\0");
}

export function buildActivationLookup(
  artifacts: readonly LoadedServiceAssemblyArtifact[],
): ActivationLookup {
  const lookup = new ActivationLookup();
  for (const artifact of artifacts) {
    for (const activation of serviceConfigActivations(artifact)) {
      for (const target of activation.operationTargets) {
        lookup.set({
          serviceId: activation.serviceId,
          target,
          buildId: artifact.buildId,
          activationIdentity: activation.payload.activationIdentity,
        });
      }
    }
  }
  return lookup;
}

export function serviceConfigActivations(
  artifact: LoadedServiceAssemblyArtifact,
): readonly NonNullable<LoadedServiceAssemblyArtifact["activation"]>[] {
  if (artifact.activations !== undefined) {
    return artifact.activations;
  }
  return artifact.activation !== undefined ? [artifact.activation] : [];
}

export function validateServingManifestUniqueness(
  artifacts: readonly LoadedServiceAssemblyArtifact[],
): void {
  const servingByIdentity = new Map<string, string>();
  for (const artifact of artifacts) {
    const serviceId = artifact.manifestValue.service.id;
    const buildId = artifact.buildId;
    const protocolIdentity = artifact.manifestValue.service.protocolIdentity;
    for (const operation of artifact.manifestValue.operations) {
      const operationProtocolIdentity =
        operation.serviceProtocolIdentity ?? protocolIdentity;
      const key = [
        serviceId,
        buildId,
        operationProtocolIdentity,
        operation.operation,
      ].join("\0");
      const existingSource = servingByIdentity.get(key);
      if (existingSource !== undefined) {
        throw new Error(
          `multiple active service assemblies serve ${serviceId}:${operationProtocolIdentity}:${operation.operation}; ${existingSource} conflicts with ${artifact.sourcePath}`,
        );
      }
      servingByIdentity.set(key, artifact.sourcePath);
    }
  }
}
