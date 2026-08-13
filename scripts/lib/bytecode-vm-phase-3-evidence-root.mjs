import {
  createBytecodeVmEvidenceRoot,
  openBytecodeVmEvidenceRoot,
} from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE3_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-3-directory-identity-v1';
export const PHASE3_DIRECTORY_IDENTITY_FILE = 'phase-3-directory-identities.json';

const options = Object.freeze({
  schemaVersion: PHASE3_DIRECTORY_IDENTITY_SCHEMA,
  identityFile: PHASE3_DIRECTORY_IDENTITY_FILE,
});

export function createPhase3EvidenceRoot(outputDir) {
  return createBytecodeVmEvidenceRoot(outputDir, options);
}

export function openPhase3EvidenceRoot(outputDir, expectedIdentities) {
  return openBytecodeVmEvidenceRoot(outputDir, expectedIdentities, options);
}
