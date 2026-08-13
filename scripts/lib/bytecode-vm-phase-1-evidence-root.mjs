import {
  createBytecodeVmEvidenceRoot,
  openBytecodeVmEvidenceRoot,
} from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE1_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-1-directory-identity-v1';
export const PHASE1_DIRECTORY_IDENTITY_FILE = 'phase-1-directory-identities.json';

const options = Object.freeze({
  schemaVersion: PHASE1_DIRECTORY_IDENTITY_SCHEMA,
  identityFile: PHASE1_DIRECTORY_IDENTITY_FILE,
});

export function createPhase1EvidenceRoot(outputDir) {
  return createBytecodeVmEvidenceRoot(outputDir, options);
}

export function openPhase1EvidenceRoot(outputDir, expectedIdentities) {
  return openBytecodeVmEvidenceRoot(outputDir, expectedIdentities, options);
}
