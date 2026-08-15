import {
  createBytecodeVmEvidenceRoot,
  openBytecodeVmEvidenceRoot,
} from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE6_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-6-directory-identity-r1-v1';
export const PHASE6_DIRECTORY_IDENTITY_FILE =
  'phase-6-r1-v1-directory-identities.json';

const options = Object.freeze({
  schemaVersion: PHASE6_DIRECTORY_IDENTITY_SCHEMA,
  identityFile: PHASE6_DIRECTORY_IDENTITY_FILE,
});

export function createPhase6EvidenceRoot(outputDir) {
  return createBytecodeVmEvidenceRoot(outputDir, options);
}

export function openPhase6EvidenceRoot(outputDir, expectedIdentities) {
  return openBytecodeVmEvidenceRoot(outputDir, expectedIdentities, options);
}
