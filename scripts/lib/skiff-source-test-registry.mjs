import { isAbsolute, relative, resolve, sep } from 'node:path';

export const canonicalSkiffSourceTestRegistry = Object.freeze([
  Object.freeze({
    id: 'std',
    root: 'test-services/std',
  }),
  Object.freeze({
    id: 'alias-return-catch-once',
    root: 'test-runner/fixtures/alias-return-catch-once-tests',
    subjectRoot: 'test-runner/fixtures/alias-return-catch-once',
  }),
]);

export function createCanonicalSkiffSourceTestPlan({
  skiffRoot,
  registry = canonicalSkiffSourceTestRegistry,
}) {
  if (!Array.isArray(registry) || registry.length === 0) {
    throw new Error('canonical Skiff source test registry must contain at least one entry');
  }

  const resolvedSkiffRoot = resolve(skiffRoot);
  const ids = new Set();
  const roots = new Set();
  return registry.map((entry) => {
    const id = requiredRegistryText(entry?.id, 'id');
    const root = requiredRegistryText(entry?.root, `entry ${id} root`);
    const subjectRoot = entry?.subjectRoot === undefined
      ? undefined
      : requiredRegistryText(entry.subjectRoot, `entry ${id} subjectRoot`);
    if (!/^[a-z][a-z0-9-]*$/.test(id)) {
      throw new Error(`canonical Skiff source test id must be kebab-case, found ${id}`);
    }
    if (ids.has(id)) {
      throw new Error(`duplicate canonical Skiff source test id ${id}`);
    }
    if (isAbsolute(root)) {
      throw new Error(`canonical Skiff source test root must be repository-relative: ${root}`);
    }

    const absoluteRoot = resolve(resolvedSkiffRoot, root);
    const repositoryRelativeRoot = relative(resolvedSkiffRoot, absoluteRoot);
    if (repositoryRelativeRoot.length === 0) {
      throw new Error(`canonical Skiff source test root must not be the repository root: ${root}`);
    }
    if (repositoryRelativeRoot === '..' || repositoryRelativeRoot.startsWith(`..${sep}`)) {
      throw new Error(`canonical Skiff source test root escapes the repository: ${root}`);
    }
    if (roots.has(repositoryRelativeRoot)) {
      throw new Error(`duplicate canonical Skiff source test root ${root}`);
    }
    ids.add(id);
    roots.add(repositoryRelativeRoot);
    const subject = subjectRoot === undefined
      ? undefined
      : resolveRepositoryPath(resolvedSkiffRoot, subjectRoot, `entry ${id} subjectRoot`);
    return Object.freeze({
      id,
      root: repositoryRelativeRoot,
      absoluteRoot,
      ...(subject === undefined
        ? {}
        : {
            subjectRoot: subject.relativeRoot,
            absoluteSubjectRoot: subject.absoluteRoot,
          }),
    });
  });
}

function resolveRepositoryPath(skiffRoot, path, label) {
  if (isAbsolute(path)) {
    throw new Error(`canonical Skiff source test ${label} must be repository-relative: ${path}`);
  }
  const absoluteRoot = resolve(skiffRoot, path);
  const relativeRoot = relative(skiffRoot, absoluteRoot);
  if (
    relativeRoot.length === 0
    || relativeRoot === '..'
    || relativeRoot.startsWith(`..${sep}`)
  ) {
    throw new Error(`canonical Skiff source test ${label} escapes the repository: ${path}`);
  }
  return { absoluteRoot, relativeRoot };
}

function requiredRegistryText(value, label) {
  if (typeof value !== 'string' || value.trim() !== value || value.length === 0) {
    throw new Error(`canonical Skiff source test ${label} must be a non-empty trimmed string`);
  }
  return value;
}
