import {
  createBytecodeVmEvidenceRoot,
  openBytecodeVmEvidenceRoot,
} from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE2_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-2-directory-identity-v1';
export const PHASE2_DIRECTORY_IDENTITY_FILE = 'phase-2-directory-identities.json';

const options = Object.freeze({
  schemaVersion: PHASE2_DIRECTORY_IDENTITY_SCHEMA,
  identityFile: PHASE2_DIRECTORY_IDENTITY_FILE,
});

export function createPhase2EvidenceRoot(outputDir) {
  return createBytecodeVmEvidenceRoot(outputDir, options);
}

export function openPhase2EvidenceRoot(outputDir, expectedIdentities) {
  return openBytecodeVmEvidenceRoot(outputDir, expectedIdentities, options);
}
