// Actor parity artifact authoring (plan §7/§8/§9).
//
// Reuses the `router-live:actor` fixture path (`actor_live_fixture.mjs`):
// real compiler package/assembly/config-snapshot over the
// actor-full-chain-acceptance service source. The canonical actor-routing
// projection record is synthesized separately (test-side A1 producer) and
// the same artifact root is copied into each side's independent artifact
// root so both implementations load byte-identical records.

import { join } from 'node:path';

import {
  authorActorLiveArtifact,
  ACTOR_LIVE_ENTRYPOINTS,
  loadActorLiveDeploymentRecord,
  writeActorLiveServiceSource,
} from '../actor_live_fixture.mjs';
import {
  ACTOR_PARITY_ENVIRONMENT,
  ACTOR_PARITY_SERVICE_SOURCE_FIXTURE,
} from './actor_parity_constants.mjs';

export async function authorActorParityArtifact({
  skiffRoot,
  sourceRoot,
  artifactRoot,
  environment = ACTOR_PARITY_ENVIRONMENT,
}) {
  await writeActorLiveServiceSource(
    sourceRoot,
    join(skiffRoot, ACTOR_PARITY_SERVICE_SOURCE_FIXTURE),
  );
  const authored = await authorActorLiveArtifact({
    skiffRoot,
    sourceRoot,
    artifactRoot,
    environment,
  });
  const deploymentRecord = await loadActorLiveDeploymentRecord(artifactRoot);
  return {
    assemblyIdentity: authored.assemblyIdentity,
    configSnapshotId: authored.configSnapshotId,
    deployment: deploymentRecord.deployment,
    gatewayEntries: deploymentRecord.gatewayEntries,
  };
}

export function actorParityEntrypoints(gatewayEntries) {
  return Object.fromEntries(
    Object.entries(ACTOR_LIVE_ENTRYPOINTS).map(([key, entry]) => {
      const gatewayEntryIdentity = gatewayEntries[key];
      if (typeof gatewayEntryIdentity !== 'string') {
        throw new Error(
          `actor parity artifact is missing gateway entry identity for ${key}`,
        );
      }
      return [
        key,
        {
          path: entry.path,
          method: 'POST',
          gatewayEntryIdentity,
        },
      ];
    }),
  );
}
