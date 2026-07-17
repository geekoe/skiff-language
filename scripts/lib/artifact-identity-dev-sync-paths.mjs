import { stat } from 'node:fs/promises';
import { join } from 'node:path';

import { assertArtifactReferencesMatchValidated } from './artifact-identity-validation.mjs';

/**
 * The only dev-sync filesystem boundary for Service/Package Unit references.
 * Exact matching must complete before any reference path reaches stat/join.
 */
export async function assertValidatedArtifactClosureFiles({
  root,
  references,
  validated,
  label,
  statPath,
}) {
  const trusted = assertArtifactReferencesMatchValidated(references, validated, label);
  await assertArtifactFile(
    root,
    trusted.serviceAssembly.assemblyPath,
    `${label} references missing service assembly`,
    statPath,
  );
  await assertArtifactFile(
    root,
    trusted.serviceUnit.unitPath,
    `${label} references missing service unit`,
    statPath,
  );
  for (const packageUnit of trusted.packageUnits) {
    await assertArtifactFile(
      root,
      packageUnit.unitPath,
      `${label} references missing package unit`,
      statPath,
    );
  }
}

async function assertArtifactFile(root, artifactPath, missingLabel, statPath) {
  const path = join(root, artifactPath);
  const fileStat = statPath ?? stat;
  let info;
  try {
    info = await fileStat(path);
  } catch (error) {
    if (error?.code !== 'ENOENT') {
      throw error;
    }
  }
  if (!info?.isFile()) {
    throw new Error(`${missingLabel} ${artifactPath}`);
  }
}
