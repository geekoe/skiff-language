const rustImplementationSubjects = [
  {
    selector: 'foundation',
    leafSelector: 'foundation-rust-tests',
    phaseId: 'implementation:foundation:rust',
    packages: [
      rustPackage('canonical-json', 'skiff-canonical-json'),
      rustPackage('artifact-model', 'skiff-artifact-model'),
      rustPackage('artifact-identity', 'skiff-artifact-identity'),
      rustPackage('syntax', 'skiff-syntax'),
    ],
  },
  {
    selector: 'compiler',
    leafSelector: 'compiler-rust-tests',
    phaseId: 'implementation:compiler:rust',
    packages: [
      rustPackage('compiler/core', 'skiff-compiler-core'),
      rustPackage('compiler/publication-abi', 'skiff-compiler-publication-abi'),
      rustPackage('compiler/input-model', 'skiff-compiler-input-model'),
      rustPackage('compiler/input', 'skiff-compiler-input'),
      rustPackage('compiler/source', 'skiff-compiler-source'),
      rustPackage('compiler/lowering', 'skiff-compiler-lowering'),
      rustPackage('compiler/compiled', 'skiff-compiler-compiled'),
      rustPackage('compiler/projection-input', 'skiff-compiler-projection-input'),
      rustPackage('compiler/projection', 'skiff-compiler-projection'),
      rustPackage('compiler/emission', 'skiff-compiler-emission'),
      rustPackage('compiler', 'skiff-compiler'),
    ],
  },
  {
    selector: 'runtime',
    leafSelector: 'runtime-rust-tests',
    phaseId: 'implementation:runtime:rust',
    packages: [
      rustPackage('runtime', 'runtime'),
      rustPackage('runtime/activation', 'skiff-runtime-activation'),
      rustPackage('runtime/boundary', 'skiff-runtime-boundary'),
      rustPackage('runtime/capability-context', 'skiff-runtime-capability-context'),
      rustPackage('runtime/eval', 'skiff-runtime-eval'),
      rustPackage('runtime/host', 'skiff-runtime-host'),
      rustPackage('runtime/linked-type-plan', 'skiff-runtime-linked-type-plan'),
      rustPackage('runtime/linked-program', 'skiff-runtime-linked-program'),
      rustPackage('runtime/linker', 'skiff-runtime-linker'),
      rustPackage('runtime/loader', 'skiff-runtime-loader'),
      rustPackage('runtime/model', 'skiff-runtime-model'),
      rustPackage('runtime/native', 'skiff-runtime-native'),
      rustPackage('runtime/native-contract', 'skiff-runtime-native-contract'),
      rustPackage('runtime/package-test', 'skiff-runtime-package-test'),
      rustPackage('runtime/request-contract', 'skiff-runtime-request-contract'),
      rustPackage('runtime/service-db', 'skiff-runtime-service-db'),
      rustPackage('runtime/request', 'skiff-runtime-request'),
      rustPackage('runtime/transport', 'skiff-runtime-transport'),
    ],
  },
  {
    selector: 'test-runner',
    leafSelector: 'test-runner-rust-tests',
    phaseId: 'implementation:test-runner:rust',
    packages: [
      rustPackage('test-runner', 'skiff-test-runner'),
    ],
  },
];

export const RUST_IMPLEMENTATION_SUBJECTS = deepFreeze(rustImplementationSubjects);

assertRustSubjectRegistryIntegrity(RUST_IMPLEMENTATION_SUBJECTS);

export const RUST_IMPLEMENTATION_SUBJECT_SELECTORS = Object.freeze(
  RUST_IMPLEMENTATION_SUBJECTS.map(({ selector }) => selector),
);

export function rustSubjectTestArgs(subject) {
  return [
    'test',
    '--no-fail-fast',
    ...subject.packages.flatMap(({ packageName }) => ['--package', packageName]),
  ];
}

export function assertRustWorkspaceOwnership(
  workspaceMembers,
  subjects = RUST_IMPLEMENTATION_SUBJECTS,
) {
  if (
    !Array.isArray(workspaceMembers)
    || !workspaceMembers.every(isNonEmptyString)
    || new Set(workspaceMembers).size !== workspaceMembers.length
  ) {
    throw new Error('Cargo workspace members must be a unique non-empty string array');
  }
  assertRustSubjectRegistryIntegrity(subjects);

  const ownership = new Map();
  for (const subject of subjects) {
    for (const pkg of subject.packages) {
      ownership.set(pkg.workspaceMember, subject.selector);
    }
  }

  const workspace = new Set(workspaceMembers);
  const missing = workspaceMembers.filter((member) => !ownership.has(member));
  const unexpected = [...ownership.keys()].filter((member) => !workspace.has(member));
  if (missing.length > 0 || unexpected.length > 0) {
    throw new Error([
      missing.length > 0
        ? `unowned Rust workspace member(s): ${missing.join(', ')}`
        : '',
      unexpected.length > 0
        ? `Rust subject member(s) absent from workspace: ${unexpected.join(', ')}`
        : '',
    ].filter(Boolean).join('; '));
  }

  return ownership;
}

function assertRustSubjectRegistryIntegrity(subjects) {
  if (!Array.isArray(subjects) || subjects.length === 0) {
    throw new Error('Rust implementation subject registry must not be empty');
  }
  assertUnique(subjects.map(({ selector }) => selector), 'Rust subject selectors');
  assertUnique(subjects.map(({ leafSelector }) => leafSelector), 'Rust subject leaves');
  assertUnique(subjects.map(({ phaseId }) => phaseId), 'Rust subject phase ids');

  const workspaceMembers = [];
  const packageNames = [];
  for (const subject of subjects) {
    if (!Array.isArray(subject.packages) || subject.packages.length === 0) {
      throw new Error(`Rust subject ${subject.selector} must own at least one package`);
    }
    for (const pkg of subject.packages) {
      if (!isNonEmptyString(pkg.workspaceMember) || !isNonEmptyString(pkg.packageName)) {
        throw new Error(`Rust subject ${subject.selector} has an invalid package entry`);
      }
      workspaceMembers.push(pkg.workspaceMember);
      packageNames.push(pkg.packageName);
    }
  }
  assertUnique(workspaceMembers, 'Rust workspace member ownership');
  assertUnique(packageNames, 'Rust package names');
}

function assertUnique(values, source) {
  if (
    !values.every(isNonEmptyString)
    || new Set(values).size !== values.length
  ) {
    throw new Error(`${source} must be unique non-empty strings`);
  }
}

function rustPackage(workspaceMember, packageName) {
  return { workspaceMember, packageName };
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
