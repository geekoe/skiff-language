export const CHECKER_CLASSIFICATIONS = Object.freeze({
  DEFAULT: 'default verify',
  RUST_QUALITY: 'rust-quality',
  SELF_TEST: 'self-test',
  LIVE_MANUAL: 'live/manual',
  KNOWN_RED: 'known-red legacy',
});

export const CHECKER_REGISTRY = Object.freeze([
  checker('scripts/check-rust-file-lines.mjs', CHECKER_CLASSIFICATIONS.RUST_QUALITY, {
    invocations: [invocation('rust-quality:file-lines', 'rust-quality')],
  }),
  checker('scripts/check-command-execution-policy.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:command-execution-policy', 'checks')],
  }),
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
  checker('scripts/check-javascript-syntax.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('javascript:syntax', 'scripts-syntax')],
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
    invocations: [
      invocation('implementation:tooling:dev-sync-fixture', 'scripts-dev-sync'),
    ],
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
  checker('scripts/check-skiff-source-layout.mjs', CHECKER_CLASSIFICATIONS.DEFAULT, {
    invocations: [invocation('checks:skiff-source-layout', 'checks')],
  }),
]);

export async function checkerTasks(root, selector, { kind } = {}) {
  return CHECKER_REGISTRY.flatMap((entry) =>
    entry.invocations
      .filter((candidate) => candidate.selector === selector)
      .map((candidate) => ({
        id: candidate.id,
        kind: kind ?? entry.classification,
        command: 'node',
        args: [entry.path, ...candidate.args],
        cwd: root,
      })),
  );
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
