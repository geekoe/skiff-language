import {
  createBytecodeVmEvidenceRoot,
  openBytecodeVmEvidenceRoot,
} from './bytecode-vm-phase-0-evidence-root.mjs';

export const PHASE5_DIRECTORY_IDENTITY_SCHEMA =
  'skiff-bytecode-vm-phase-5-directory-identity-r1-v3';
export const PHASE5_DIRECTORY_IDENTITY_FILE = 'phase-5-r1-v3-directory-identities.json';

const options = Object.freeze({
  schemaVersion: PHASE5_DIRECTORY_IDENTITY_SCHEMA,
  identityFile: PHASE5_DIRECTORY_IDENTITY_FILE,
});

export function createPhase5EvidenceRoot(outputDir) {
  return createBytecodeVmEvidenceRoot(outputDir, options);
}

export function openPhase5EvidenceRoot(outputDir, expectedIdentities) {
  return openBytecodeVmEvidenceRoot(outputDir, expectedIdentities, options);
}
