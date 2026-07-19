export const RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT =
  'runtime/host/src/host/request_entry.rs';

export const REQUIRED_RUNTIME_ARTIFACT_BOUNDARY_SUBJECT_IDS = Object.freeze([
  'typed-runtime-assembly-loader',
  'shared-package-linked-image',
  'runtime-assembly-linker',
  'whole-assembly-host',
  'terminal-runtime-consumers',
]);

export const REQUIRED_RUNTIME_ARTIFACT_BOUNDARY_OWNED_ROOTS = Object.freeze([
  Object.freeze({
    subjectId: 'whole-assembly-host',
    ownedRoot: RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT,
  }),
]);

export const RUNTIME_ARTIFACT_BOUNDARY_SUBJECTS = Object.freeze([
  subject({
    id: 'typed-runtime-assembly-loader',
    kind: 'canonical',
    ownedRoots: [
      'runtime/loader/src/runtime_assembly.rs',
      'runtime/loader/src/runtime_assembly',
    ],
  }),
  subject({
    id: 'shared-package-linked-image',
    kind: 'canonical',
    ownedRoots: [
      'runtime/linked-program/src/shared_image.rs',
      'runtime/linked-program/src/shared_image',
    ],
  }),
  subject({
    id: 'runtime-assembly-linker',
    kind: 'canonical',
    ownedRoots: [
      'runtime/linker/src/assembly.rs',
      'runtime/linker/src/assembly',
    ],
  }),
  subject({
    id: 'whole-assembly-host',
    kind: 'canonical',
    ownedRoots: [
      'runtime/host/src/loader/assembly_admission.rs',
      RUNTIME_REQUEST_ENTRY_BOUNDARY_ROOT,
    ],
    discoveryRoots: ['runtime/host/src'],
  }),
  subject({
    id: 'terminal-runtime-consumers',
    kind: 'consumer',
    discoveryRoots: [
      'runtime/activation/src',
      'runtime/eval/src',
      'runtime/package-test/src',
      'runtime/request/src',
      'runtime/linked-type-plan/src',
    ],
  }),
]);

function subject({
  id,
  kind,
  ownedRoots = [],
  discoveryRoots = ownedRoots,
  allowMissingOwnedRoots = false,
}) {
  return Object.freeze({
    id,
    kind,
    ownedRoots: Object.freeze(ownedRoots),
    discoveryRoots: Object.freeze(discoveryRoots),
    allowMissingOwnedRoots,
  });
}
