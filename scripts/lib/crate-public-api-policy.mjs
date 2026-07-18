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
  record('skiff-compiler', 1, [
    'skiff-compiler',
    'skiff-compiler-contract',
    'skiff-compiler-input-model',
    'skiff-compiler-input',
    'skiff-compiler-source',
    'skiff-compiler-emission',
    'skiff-artifact-model',
    'skiff-syntax',
    ...standardCrates,
    ...approvedExternalValueCrates,
  ], 'package compiler public API exposes only terminal package/contract input-output types and approved value crates'),
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
