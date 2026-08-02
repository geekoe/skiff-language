// Actor-routing projection derivation for differential fixtures.
//
// At the Batch 10 baseline the compiler publish path does not yet emit
// `records/actor-routing/current.json` (A1-compiler node wires that in);
// the A2 TS Router consumes the projection strictly, so differential actor
// scenarios need a real projection that matches the compiled artifact. This
// module derives the frozen projection (schema v1) from the compiler's own
// immutable records: the service's package artifact record (package refs +
// actor ABI + public method identities) and its File IR record (actor
// implementation identities). Services without actor declarations return
// null and the harness keeps the legal empty projection. The derived record
// is written in canonical JSON (sorted keys, no whitespace), matching the
// A3 strict reader contract.

import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';

export const ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION =
  'skiff-actor-routing-projection-v1';

export async function deriveActorRoutingProjection({
  artifactRoot,
  deployment,
}) {
  // The harness passes either the compiler ServiceDeploymentRef (flat) or
  // the published deployment record ({ contract: {...} }); normalize both.
  const serviceId = deployment?.contract?.serviceId ?? deployment?.serviceId;
  if (typeof serviceId !== 'string' || serviceId.length === 0) {
    throw new Error('actor routing projection requires a deployment contract serviceId');
  }
  const ownPackage = await findOwnPackageRecord(artifactRoot, serviceId);
  if (ownPackage === null) {
    throw new Error(`actor routing projection: no package artifact for ${serviceId}`);
  }
  const packageRecord = ownPackage.record;
  const packageRef = {
    packageBuildId: packageRecord.packageBuildId,
    packageId: packageRecord.packageId,
    packageLocalAbiIdentity: packageRecord.packageLocalAbi?.localAbiIdentity,
    packageVersion: packageRecord.packageVersion,
  };
  const deploymentRef = {
    contractVersion: deployment.contract?.contractVersion ?? deployment.contractVersion,
    deploymentArtifactIdentity: deployment.deploymentArtifactIdentity,
    deploymentRevision: deployment.deploymentRevision,
    serviceId,
  };
  const actorTypes = packageRecord.implementationLinks?.types ?? {};
  const methods = [];
  for (const [symbol, typeEntry] of Object.entries(actorTypes)) {
    const actor = typeEntry?.actor;
    const publicMethods = actor?.abi?.publicMethods;
    if (!Array.isArray(publicMethods) || publicMethods.length === 0) {
      continue;
    }
    const fileIrIdentity = typeEntry?.file?.fileIrIdentity;
    const declaration = await findActorDeclaration({
      packageDirectory: ownPackage.directory,
      fileIrIdentity,
      actorAbiIdentity: actor.actorAbiIdentity,
      symbol,
    });
    for (const method of publicMethods) {
      if (typeof method.methodIdentity !== 'string' || typeof method.name !== 'string') {
        throw new Error(`actor routing projection: ${symbol} has an incomplete public method`);
      }
      methods.push({
        actor: {
          serviceId,
          actorAbiIdentity: actor.actorAbiIdentity,
        },
        actorImplementationIdentity: declaration.actorImplementationIdentity,
        methodIdentity: method.methodIdentity,
        deployment: deploymentRef,
        package: packageRef,
      });
    }
  }
  if (methods.length === 0) {
    return null;
  }
  methods.sort(compareProjectionMethods);
  return {
    schemaVersion: ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    methods,
  };
}

export function canonicalProjectionJson(projection) {
  const methods = projection.methods.map((method) => ({
    actor: {
      actorAbiIdentity: method.actor.actorAbiIdentity,
      serviceId: method.actor.serviceId,
    },
    actorImplementationIdentity: method.actorImplementationIdentity,
    deployment: {
      contractVersion: method.deployment.contractVersion,
      deploymentArtifactIdentity: method.deployment.deploymentArtifactIdentity,
      deploymentRevision: method.deployment.deploymentRevision,
      serviceId: method.deployment.serviceId,
    },
    methodIdentity: method.methodIdentity,
    package: {
      packageBuildId: method.package.packageBuildId,
      packageId: method.package.packageId,
      packageLocalAbiIdentity: method.package.packageLocalAbiIdentity,
      packageVersion: method.package.packageVersion,
    },
  }));
  return JSON.stringify({
    methods,
    schemaVersion: projection.schemaVersion,
  });
}

function compareProjectionMethods(left, right) {
  const leftKey = [
    left.actor.serviceId,
    left.actor.actorAbiIdentity,
    left.actorImplementationIdentity,
    left.methodIdentity,
    left.deployment.contractVersion,
    left.deployment.deploymentArtifactIdentity,
    left.deployment.deploymentRevision,
    left.package.packageBuildId,
    left.package.packageId,
    left.package.packageLocalAbiIdentity,
    left.package.packageVersion,
  ].join('\u0000');
  const rightKey = [
    right.actor.serviceId,
    right.actor.actorAbiIdentity,
    right.actorImplementationIdentity,
    right.methodIdentity,
    right.deployment.contractVersion,
    right.deployment.deploymentArtifactIdentity,
    right.deployment.deploymentRevision,
    right.package.packageBuildId,
    right.package.packageId,
    right.package.packageLocalAbiIdentity,
    right.package.packageVersion,
  ].join('\u0000');
  return leftKey < rightKey ? -1 : leftKey > rightKey ? 1 : 0;
}

async function findOwnPackageRecord(artifactRoot, serviceId) {
  const packageArtifactsRoot = join(artifactRoot, 'records', 'package-artifacts');
  const serviceDirs = await readdir(packageArtifactsRoot);
  for (const serviceDir of serviceDirs) {
    const versionsRoot = join(packageArtifactsRoot, serviceDir);
    let versions;
    try {
      versions = await readdir(versionsRoot);
    } catch {
      continue;
    }
    for (const version of versions) {
      const buildsRoot = join(versionsRoot, version);
      let builds;
      try {
        builds = await readdir(buildsRoot);
      } catch {
        continue;
      }
      for (const build of builds) {
        const packagePath = join(buildsRoot, build, 'package.json');
        let record;
        try {
          record = JSON.parse(await readFile(packagePath, 'utf8'));
        } catch {
          continue;
        }
        if (record?.packageId === serviceId) {
          return { record, directory: join(buildsRoot, build) };
        }
      }
    }
  }
  return null;
}

async function findActorDeclaration({
  packageDirectory,
  fileIrIdentity,
  actorAbiIdentity,
  symbol,
}) {
  if (typeof fileIrIdentity !== 'string' || fileIrIdentity.length === 0) {
    throw new Error(`actor routing projection: ${symbol} has no File IR identity`);
  }
  const fileIr = JSON.parse(
    await readFile(
      join(packageDirectory, 'file-ir', `${fileIrIdentity.split(':').pop()}.json`),
      'utf8',
    ),
  );
  const declaration = (fileIr.actorDeclarations ?? []).find(
    (candidate) => candidate.actorAbiIdentity === actorAbiIdentity,
  );
  if (declaration === undefined) {
    throw new Error(
      `actor routing projection: File IR has no actor declaration for ${symbol} (${actorAbiIdentity})`,
    );
  }
  return declaration;
}
