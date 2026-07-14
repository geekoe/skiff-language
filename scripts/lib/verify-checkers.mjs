import { discoverCheckerScripts } from './verify-discovery.mjs';

export const CHECKER_CLASSIFICATIONS = Object.freeze({
  DEFAULT: 'default verify',
  SELF_TEST: 'self-test',
  LIVE_MANUAL: 'live/manual',
  KNOWN_RED: 'known-red legacy',
});

export const CHECKER_REGISTRY = Object.freeze([
  checker('scripts/check-compiler-boundaries.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:compiler-boundaries', 'compiler-boundaries')],
  }),
  checker('scripts/check-compiler-crate-dag.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [
      invocation('checks:compiler-crate-dag:self-test', 'checks', ['--self-test']),
      invocation('checks:compiler-crate-dag', 'checks'),
    ],
  }),
  checker('scripts/check-crate-public-api.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [
      invocation('checks:crate-public-api:self-test', 'checks', ['--self-test']),
      invocation('checks:crate-public-api:all-configured', 'checks', ['--all-configured']),
    ],
  }),
  checker('scripts/check-db-encrypted-storage-live.mjs', CHECKER_CLASSIFICATIONS.LIVE_MANUAL, {
    reason: 'Starts an isolated managed Mongo/runtime/keyring live environment.',
    invocations: [
      invocation('live:db-encrypted-storage', 'db-encrypted-storage-live'),
    ],
  }),
  checker('scripts/check-local-instance.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:local-instance', 'checks')],
  }),
  checker('scripts/check-loop-risk-health.mjs', CHECKER_CLASSIFICATIONS.LIVE_MANUAL, {
    reason: 'Requires a running router health endpoint and runtime identity.',
  }),
  checker(
    'scripts/check-artifact-identity-single-source.mjs',
    CHECKER_CLASSIFICATIONS.DEFAULT,
    {
      invocations: [
        invocation('checks:artifact-identity:self-test', 'checks', ['--self-test']),
        invocation('checks:artifact-identity', 'checks'),
      ],
    },
  ),
  checker('scripts/check-package-store-discovery.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:package-store-discovery', 'checks')],
  }),
  checker('scripts/check-publication-resource-archive.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:publication-resource-archive', 'checks')],
  }),
  checker('scripts/check-runtime-crate-dag.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [
      invocation('checks:runtime-crate-dag:self-test', 'checks', ['--self-test']),
      invocation('checks:runtime-crate-dag', 'checks'),
    ],
  }),
  checker('scripts/check-runtime-eval-error-boundary.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:runtime-eval-error-boundary', 'checks')],
  }),
  checker('scripts/check-skiff-source-layout.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:skiff-source-layout', 'checks')],
  }),
]);

export async function checkerPhases(root, selector) {
  await assertCheckerRegistryComplete(root);
  return CHECKER_REGISTRY.flatMap((entry) =>
    entry.invocations
      .filter((candidate) => candidate.selector === selector)
      .map((candidate) => ({
        id: candidate.id,
        kind: entry.classification,
        command: 'node',
        args: [entry.path, ...candidate.args],
        cwd: root,
      })),
  );
}

export async function assertCheckerRegistryComplete(root) {
  const discovered = await discoverCheckerScripts(root);
  const registered = CHECKER_REGISTRY.map((entry) => entry.path).sort();
  const duplicatePaths = registered.filter((path, index) => registered.indexOf(path) !== index);
  if (duplicatePaths.length > 0) {
    throw new Error(`duplicate checker registry path(s): ${[...new Set(duplicatePaths)].join(', ')}`);
  }

  const missing = discovered.filter((path) => !registered.includes(path));
  const stale = registered.filter((path) => !discovered.includes(path));
  if (missing.length > 0 || stale.length > 0) {
    throw new Error([
      missing.length > 0 ? `unclassified checker(s): ${missing.join(', ')}` : '',
      stale.length > 0 ? `missing registered checker(s): ${stale.join(', ')}` : '',
    ].filter(Boolean).join('; '));
  }
}

function checker(path, classification, { reason = null, invocations = [] } = {}) {
  if (!Object.values(CHECKER_CLASSIFICATIONS).includes(classification)) {
    throw new Error(`invalid checker classification for ${path}: ${classification}`);
  }
  if (
    [CHECKER_CLASSIFICATIONS.KNOWN_RED, CHECKER_CLASSIFICATIONS.LIVE_MANUAL].includes(
      classification,
    ) && !reason
  ) {
    throw new Error(`${classification} checker ${path} requires a reason`);
  }
  return Object.freeze({
    path,
    classification,
    reason,
    invocations: Object.freeze(invocations),
  });
}

function invocation(id, selector, args = []) {
  return Object.freeze({ id, selector, args: Object.freeze(args) });
}
