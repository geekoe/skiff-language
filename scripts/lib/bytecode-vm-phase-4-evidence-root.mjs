import {
  createBytecodeVmEvidenceRoot,
  openBytecodeVmEvidenceRoot,
} from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE4_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-4-directory-identity-v1';
export const PHASE4_DIRECTORY_IDENTITY_FILE = 'phase-4-directory-identities.json';

const options = Object.freeze({
  schemaVersion: PHASE4_DIRECTORY_IDENTITY_SCHEMA,
  identityFile: PHASE4_DIRECTORY_IDENTITY_FILE,
});

export function createPhase4EvidenceRoot(outputDir) {
  return createBytecodeVmEvidenceRoot(outputDir, options);
}

export function openPhase4EvidenceRoot(outputDir, expectedIdentities) {
  return openBytecodeVmEvidenceRoot(outputDir, expectedIdentities, options);
}
