// Release pointer table seed for live checks.
//
// The release pointer table `(profile, serviceId, version) -> buildId` is the
// router's only mutable deployment state (M1 typed pointer store). Live
// harnesses that author artifacts with the compiler `build` action (which
// does not publish release pointers) seed the table directly by writing the
// canonical `pointers/releases/<profile>/<service~enc>/<version>.json`
// document into the artifact root.

import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

import { encodeServiceSegment } from './release-command.mjs';

export const RELEASE_POINTER_SCHEMA_VERSION = 'skiff-release-pointer-v1';

export function releasePointerPath({ artifactRoot, profile, deployment }) {
  const serviceSegment = encodeServiceSegment(deployment.serviceId);
  const versionSegment = deployment.contractVersion;
  return join(
    artifactRoot,
    'pointers',
    'releases',
    profile,
    serviceSegment,
    `${versionSegment}.json`,
  );
}

export function releasePointerDocument({ profile, deployment, recordPath }) {
  return {
    schemaVersion: RELEASE_POINTER_SCHEMA_VERSION,
    profile,
    deployment: {
      serviceId: deployment.serviceId,
      contractVersion: deployment.contractVersion,
      deploymentRevision: deployment.deploymentRevision,
      deploymentArtifactIdentity: deployment.deploymentArtifactIdentity,
    },
    recordPath,
  };
}

export async function writeReleasePointerSeed({
  artifactRoot,
  profile,
  deployment,
  recordPath,
}) {
  if (
    typeof artifactRoot !== 'string'
    || artifactRoot.length === 0
    || typeof profile !== 'string'
    || profile.length === 0
  ) {
    throw new Error('release pointer seed requires an absolute artifact root and profile');
  }
  for (const field of [
    'serviceId',
    'contractVersion',
    'deploymentRevision',
    'deploymentArtifactIdentity',
  ]) {
    if (
      typeof deployment?.[field] !== 'string'
      || deployment[field].length === 0
      || deployment[field].trim() !== deployment[field]
    ) {
      throw new Error(`release pointer seed deployment.${field} must be a non-empty trimmed string`);
    }
  }
  const pointer = releasePointerDocument({ profile, deployment, recordPath });
  const pointerPath = releasePointerPath({ artifactRoot, profile, deployment });
  await mkdir(dirname(pointerPath), { recursive: true });
  await writeFile(pointerPath, `${JSON.stringify(pointer, null, 2)}\n`);
  return { pointer, pointerPath };
}
