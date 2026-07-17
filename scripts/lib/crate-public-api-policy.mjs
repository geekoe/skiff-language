const standardCrates = Object.freeze(['std', 'core', 'alloc']);
const approvedExternalValueCrates = Object.freeze(['serde', 'serde_json']);

const managedRecords = deepFreeze([
  record('skiff-compiler-contract', 0, [
    'skiff-compiler-contract',
    'skiff-artifact-model',
    'skiff-artifact-identity',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'contract public API exposes only self/artifact-model/artifact-identity/std and approved value crates'),
  record('skiff-compiler-publication-abi', 1, [
    'skiff-compiler-publication-abi',
    'skiff-artifact-model',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'publication-abi public API exposes only self/artifact-model/std and approved value crates'),
  record('skiff-compiler-input-model', 2, [
    'skiff-compiler-input-model',
    'skiff-compiler-core',
    'skiff-artifact-model',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'input-model public API excludes skiff-syntax/parser/AST unless explicitly allowed later'),
  record('skiff-compiler-input', 3, [
    'skiff-compiler-input',
    'skiff-compiler-core',
    'skiff-compiler-input-model',
    'skiff-artifact-model',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'input public API allows only self/core/input-model/artifact-model/std and approved value crates'),
  record('skiff-compiler-projection-input', 8, [
    'skiff-compiler-projection-input',
    'skiff-compiler-core',
    'skiff-artifact-model',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'projection-input public API allows only self/core/artifact-model/std and approved value crates'),
  record('skiff-compiler-source', 4, [
    'skiff-compiler-source',
    'skiff-compiler-core',
    'skiff-compiler-input-model',
    'skiff-artifact-model',
    'skiff-syntax',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'source public API allows only self/core/input-model/artifact-model/syntax/std and approved value crates'),
  record('skiff-compiler-lowering', 5, [
    'skiff-compiler-lowering',
    'skiff-compiler-core',
    'skiff-compiler-source',
    'skiff-artifact-model',
    'skiff-syntax',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'lowering public API allows only self/core/source/artifact-model/syntax/std and approved value crates'),
  record('skiff-compiler-compiled', 6, [
    'skiff-compiler-compiled',
    'skiff-compiler-core',
    'skiff-compiler-source',
    'skiff-compiler-lowering',
    'skiff-compiler-projection-input',
    'skiff-artifact-model',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'compiled public API allows only self/core/source/lowering/projection-input/artifact-model/std and approved value crates'),
  record('skiff-compiler-projection', 7, [
    'skiff-compiler-projection',
    'skiff-compiler-core',
    'skiff-compiler-projection-input',
    'skiff-compiler-publication-abi',
    'skiff-artifact-model',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'projection public API allows only self/core/projection-input/publication-abi/artifact-model/std and approved value crates'),
]);

const recordsByName = new Map(managedRecords.map((managed) => [managed.name, managed]));

export const MANAGED_CRATE_NAMES = Object.freeze(managedRecords.map(({ name }) => name));

export const MANAGED_CRATE_HELP_NAMES = Object.freeze(
  [...managedRecords]
    .sort((left, right) => left.helpOrder - right.helpOrder)
    .map(({ name }) => name),
);

export function managedCrateConfig(crateName) {
  const managed = recordsByName.get(crateName);
  return managed === undefined
    ? undefined
    : Object.freeze({ allowedCrates: managed.allowedCrates, note: managed.note });
}

export function publicApiConfigForCrate(crateName, extraAllowedCrates = []) {
  const managed = recordsByName.get(crateName);
  const base = managed ?? {
    allowedCrates: [crateName, ...standardCrates],
    note: 'no default allow-list exists; using self plus std/core/alloc',
  };
  return deepFreeze({
    allowedCrates: uniqueCrates([...base.allowedCrates, ...extraAllowedCrates]),
    note: base.note,
  });
}

export function normalizeCrateName(crateName) {
  return crateName.replaceAll('-', '_');
}

export function uniqueCrates(crates) {
  const seen = new Set();
  const unique = [];
  for (const crateName of crates) {
    const normalized = normalizeCrateName(crateName);
    if (seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    unique.push(crateName);
  }
  return Object.freeze(unique);
}

function record(name, helpOrder, allowedCrates, note) {
  return { name, helpOrder, allowedCrates, note };
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
