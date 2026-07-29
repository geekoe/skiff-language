import assert from 'node:assert/strict';
import test from 'node:test';

import { CHECKER_REGISTRY } from '../lib/verify-checkers.mjs';
import { GRAPH_CASES, GRAPH_MATRIX_EXPECTED_IDS } from './helpers/crate-public-api-graph-cases.mjs';
import {
  GATE_POLICY,
  HELP_ORDER,
  REPO_ROOT,
  cargoKinds,
  expectedPassingOutput,
  passingRustdoc,
  runPublicApiCli,
} from './helpers/crate-public-api-harness.mjs';

const helpOutput = `Usage:
  node scripts/check-crate-public-api.mjs --crate <crate> [--allow-crate <crate> ...]
  node scripts/check-crate-public-api.mjs --all-configured
  node scripts/check-crate-public-api.mjs --self-test

Checks exported public API types with rustdoc JSON:
  cargo +nightly rustdoc -p <crate> --lib -- -Z unstable-options --output-format json
  RUSTC_BOOTSTRAP=1 cargo rustdoc -p <crate> --lib -- -Z unstable-options --output-format json

Default gated crates:
  skiff-deployment
  skiff-compiler-contract
  skiff-compiler
`;

const selfTestOutput =
  'Self-test passed: allowed fixture 0 violation(s), denied fixture 9 violation(s).\n';
const nightlyUnavailableWarning =
  'Nightly Rust toolchain is unavailable; falling back to current toolchain with RUSTC_BOOTSTRAP=1.\n';
const managedCrateCount = GATE_POLICY.length;

function fallbackOutput(crateName, extraAllowedCrates = []) {
  const allowed = [crateName, 'std', 'core', 'alloc', ...extraAllowedCrates];
  return [
    `Public API allow-list for ${crateName}: ${allowed.join(', ')}`,
    'Policy: no default allow-list exists; using self plus std/core/alloc',
    `Public API check passed for ${crateName}.`,
    '',
  ].join('\n');
}

function graphExpected(caseDefinition) {
  const stdout = [
    'Public API allow-list for matrix-crate: matrix-crate, std, core, alloc, allowed-dep',
    'Policy: no default allow-list exists; using self plus std/core/alloc',
  ];
  const stderr = [];
  if (caseDefinition.expectedViolations.length === 0) {
    stdout.push('Public API check passed for matrix-crate.');
  } else {
    stderr.push(
      `Public API check failed for matrix-crate: ${caseDefinition.expectedViolations.length} forbidden reference(s).`,
    );
    for (const violation of caseDefinition.expectedViolations) {
      stderr.push(
        `DENY ${violation.site} references ${violation.referencedPath} from forbidden crate ${violation.crateName}`,
      );
    }
  }
  return {
    code: caseDefinition.expectedViolations.length === 0 ? 0 : 1,
    stderr: stderr.length === 0 ? '' : `${stderr.join('\n')}\n`,
    stdout: `${stdout.join('\n')}\n`,
  };
}

test('help aliases and duplicate help preserve the exact help snapshot and help order', async () => {
  for (const args of [['--help'], ['-h'], ['--help', '-h', '--help']]) {
    const result = await runPublicApiCli(args);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 0, stderr: '', stdout: helpOutput },
    );
    assert.deepEqual(result.cargoLog, []);
  }
  assert.deepEqual(
    helpOutput.match(/^  skiff-(?:deployment|compiler(?:-.+)?)$/gm).map((line) => line.trim()),
    HELP_ORDER,
  );
  assert.deepEqual(HELP_ORDER, GATE_POLICY.map(({ name }) => name));
});

test('self-test aliases and duplicate self-test preserve exact output without Cargo', async () => {
  for (const args of [['--self-test'], ['--test'], ['--self-test', '--test', '--self-test']]) {
    const result = await runPublicApiCli(args);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 0, stderr: '', stdout: selfTestOutput },
    );
    assert.deepEqual(result.cargoLog, []);
  }
});

test('parser errors preserve exact exit/stdout/stderr and never reach Cargo', async () => {
  const cases = [
    { args: [], error: 'missing crate name; run with --help for usage' },
    { args: ['--unknown'], error: 'unknown option: --unknown' },
    { args: ['--'], error: 'unknown option: --' },
    { args: ['one', 'two'], error: 'unexpected extra crate name: two' },
    { args: ['--crate'], error: '--crate requires a crate name' },
    { args: ['--crate='], error: '--crate requires a crate name' },
    { args: ['--crate', 'one', '--crate', 'two'], error: 'crate name was specified more than once: two' },
    { args: ['one', '--crate=two'], error: 'crate name was specified more than once: two' },
    { args: ['--all-configured', '--all-configured'], error: '--all-configured may be specified only once' },
    { args: ['--all-configured', 'one'], error: '--all-configured cannot be combined with an explicit crate' },
    { args: ['--all-configured', '--allow', 'extra'], error: '--all-configured cannot be combined with --allow-crate/--allow-list' },
    { args: ['--all-configured', '--allow-crate='], error: '--all-configured cannot be combined with --allow-crate/--allow-list' },
    { args: ['one', '--allow'], error: '--allow requires a crate name' },
    { args: ['one', '--allow', ''], error: '--allow requires a crate name' },
    { args: ['one', '--allow-crate', '--help'], error: '--allow-crate requires a crate name' },
    { args: ['one', '--allow-list'], error: '--allow-list requires a comma-separated crate list' },
    { args: ['one', '--allow-list', ''], error: '--allow-list requires a comma-separated crate list' },
  ];
  for (const { args, error } of cases) {
    const result = await runPublicApiCli(args);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 1, stderr: `${error}\n`, stdout: '' },
      args.join(' '),
    );
    assert.deepEqual(result.cargoLog, [], args.join(' '));
  }
});

test('help/self-test parse all arguments first, then preserve help-before-self-test priority', async () => {
  const successfulCases = [
    { args: ['--help', '--self-test'], stdout: helpOutput },
    { args: ['--self-test', '--help'], stdout: helpOutput },
    { args: ['fixture-crate', '--allow', 'extra', '--help'], stdout: helpOutput },
    { args: ['fixture-crate', '--allow', 'extra', '--self-test'], stdout: selfTestOutput },
  ];
  for (const { args, stdout } of successfulCases) {
    const result = await runPublicApiCli(args);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 0, stderr: '', stdout },
    );
    assert.deepEqual(result.cargoLog, []);
  }

  for (const args of [
    ['--help', '--unknown'],
    ['--self-test', '--unknown'],
  ]) {
    const result = await runPublicApiCli(args);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 1, stderr: 'unknown option: --unknown\n', stdout: '' },
    );
    assert.deepEqual(result.cargoLog, []);
  }
  for (const { args, error } of [
    {
      args: ['--help', '--all-configured', 'fixture-crate'],
      error: '--all-configured cannot be combined with an explicit crate',
    },
    {
      args: ['--self-test', '--all-configured', '--allow=extra'],
      error: '--all-configured cannot be combined with --allow-crate/--allow-list',
    },
  ]) {
    const result = await runPublicApiCli(args);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 1, stderr: `${error}\n`, stdout: '' },
    );
    assert.deepEqual(result.cargoLog, []);
  }
});

test('allow split/inline forms preserve empty, whitespace, comma, and first-spelling behavior', async () => {
  const base = {
    packageNames: ['fixture-crate'],
    defaultRustdoc: passingRustdoc('fixture-crate'),
  };
  const cases = [
    {
      args: ['fixture-crate', '--allow-crate', 'alpha-beta', '--allow', 'alpha_beta'],
      extras: ['alpha-beta'],
    },
    {
      args: ['fixture-crate', '--allow-crate=alpha-beta', '--allow=alpha_beta'],
      extras: ['alpha-beta'],
    },
    {
      args: ['fixture-crate', '--allow-crate=', '--allow='],
      extras: [''],
    },
    {
      args: ['fixture-crate', '--allow', '   '],
      extras: ['   '],
    },
    {
      args: ['fixture-crate', '--allow-list', ' alpha-beta, ,alpha_beta,beta '],
      extras: ['alpha-beta', 'beta'],
    },
    {
      args: ['fixture-crate', '--allow-list=alpha-beta,,alpha_beta,beta'],
      extras: ['alpha-beta', 'beta'],
    },
    { args: ['fixture-crate', '--allow-list='], extras: [] },
    { args: ['fixture-crate', '--allow-list', '   '], extras: [] },
  ];
  for (const { args, extras } of cases) {
    const result = await runPublicApiCli(args, base);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      { code: 0, stderr: '', stdout: fallbackOutput('fixture-crate', extras) },
      args.join(' '),
    );
    assert.deepEqual(cargoKinds(result), ['metadata', 'probe', 'rustdoc']);
  }
});

test('single dash and -x are positional crate names, not options', async () => {
  for (const crateName of ['-', '-x']) {
    const result = await runPublicApiCli([crateName], { packageNames: [] });
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      {
        code: 0,
        stderr: '',
        stdout: `SKIP public API check for ${crateName}: package is not present in this workspace yet.\n`,
      },
    );
    assert.deepEqual(cargoKinds(result), ['metadata']);
  }
});

test('explicit missing package performs metadata once, prints SKIP, and avoids probe/rustdoc', async () => {
  const result = await runPublicApiCli(['missing-crate'], { packageNames: [] });
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    {
      code: 0,
      stderr: '',
      stdout: 'SKIP public API check for missing-crate: package is not present in this workspace yet.\n',
    },
  );
  assert.deepEqual(cargoKinds(result), ['metadata']);
});

test('explicit unmanaged package uses self plus std/core/alloc fallback policy', async () => {
  const result = await runPublicApiCli(['fixture-crate'], {
    packageNames: ['fixture-crate'],
    defaultRustdoc: passingRustdoc('fixture-crate'),
  });
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    { code: 0, stderr: '', stdout: fallbackOutput('fixture-crate') },
  );
  assert.deepEqual(cargoKinds(result), ['metadata', 'probe', 'rustdoc']);
});

test('--crate selects the managed deployment policy and runs one rustdoc target', async () => {
  const deploymentPolicy = GATE_POLICY.find(({ name }) => name === 'skiff-deployment');
  const result = await runPublicApiCli(['--crate', deploymentPolicy.name], {
    packageNames: [deploymentPolicy.name],
    defaultRustdoc: passingRustdoc(deploymentPolicy.name),
  });
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    { code: 0, stderr: '', stdout: expectedPassingOutput([deploymentPolicy]) },
  );
  assert.deepEqual(cargoKinds(result), ['metadata', 'probe', 'rustdoc']);
});

test('all-configured fails closed after one metadata call when any managed crate is missing', async () => {
  const missing = GATE_POLICY.at(-1).name;
  const packageNames = GATE_POLICY.map(({ name }) => name).filter((name) => name !== missing);
  const result = await runPublicApiCli(['--all-configured'], { packageNames });
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    {
      code: 1,
      stderr: `configured public API crate(s) missing from workspace: ${missing}\n`,
      stdout: '',
    },
  );
  assert.deepEqual(cargoKinds(result), ['metadata']);
});

test('nightly-available all-configured does metadata/probe once and rustdoc serially in gate order', async () => {
  const result = await runPublicApiCli(['--all-configured']);
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    { code: 0, stderr: '', stdout: expectedPassingOutput() },
  );
  assert.deepEqual(cargoKinds(result), [
    'metadata',
    'probe',
    ...Array(managedCrateCount).fill('rustdoc'),
  ]);
  const rustdocCalls = result.cargoLog.filter(({ kind }) => kind === 'rustdoc');
  assert.deepEqual(
    rustdocCalls.map(({ args }) => args[args.indexOf('-p') + 1]),
    GATE_POLICY.map(({ name }) => name),
  );
  assert.ok(rustdocCalls.every(({ args, rustcBootstrap }) =>
    args[0] === '+nightly' && rustcBootstrap === null));
  assert.deepEqual(result.cargoLog[0], {
    args: ['metadata', '--format-version', '1', '--no-deps'],
    cwd: REPO_ROOT,
    kind: 'metadata',
    rustcBootstrap: null,
  });
  assert.deepEqual(result.cargoLog[1], {
    args: ['+nightly', '--version'],
    cwd: REPO_ROOT,
    kind: 'probe',
    rustcBootstrap: null,
  });
  assert.deepEqual(rustdocCalls[0].args, [
    '+nightly',
    'rustdoc',
    '-p',
    GATE_POLICY[0].name,
    '--lib',
    '--',
    '-Z',
    'unstable-options',
    '--output-format',
    'json',
  ]);
});

test('empty inline allow-list does not count as an all-configured override', async () => {
  const result = await runPublicApiCli(['--all-configured', '--allow-list=']);
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    { code: 0, stderr: '', stdout: expectedPassingOutput() },
  );
  assert.deepEqual(cargoKinds(result), [
    'metadata',
    'probe',
    ...Array(managedCrateCount).fill('rustdoc'),
  ]);
});

test('nightly-unavailable all-configured emits one warning per terminal owner', async () => {
  const result = await runPublicApiCli(['--all-configured'], { nightlyAvailable: false });
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    {
      code: 0,
      stderr: nightlyUnavailableWarning.repeat(managedCrateCount),
      stdout: expectedPassingOutput(),
    },
  );
  assert.deepEqual(cargoKinds(result), [
    'metadata',
    'probe',
    ...Array(managedCrateCount).fill('rustdoc'),
  ]);
  const rustdocCalls = result.cargoLog.filter(({ kind }) => kind === 'rustdoc');
  assert.deepEqual(
    rustdocCalls.map(({ args }) => args[args.indexOf('-p') + 1]),
    GATE_POLICY.map(({ name }) => name),
  );
  assert.ok(rustdocCalls.every(({ args, rustcBootstrap }) =>
    args[0] === 'rustdoc' && rustcBootstrap === '1'));
});

test('nightly rustdoc failure falls back to bootstrap and warns before result rendering', async () => {
  const crateName = 'fixture-crate';
  const result = await runPublicApiCli([crateName], {
    packageNames: [crateName],
    defaultRustdoc: passingRustdoc(crateName),
    rustdocFailures: {
      [crateName]: { nightly: { code: 31, stderr: 'nightly failed\n' } },
    },
  });
  assert.deepEqual(
    { code: result.code, stderr: result.stderr, stdout: result.stdout },
    {
      code: 0,
      stderr: `Built rustdoc JSON for ${crateName} with RUSTC_BOOTSTRAP=1 cargo rustdoc.\n`,
      stdout: fallbackOutput(crateName),
    },
  );
  assert.deepEqual(cargoKinds(result), ['metadata', 'probe', 'rustdoc', 'rustdoc']);
  const rustdocCalls = result.cargoLog.slice(2);
  assert.equal(rustdocCalls[0].args[0], '+nightly');
  assert.equal(rustdocCalls[1].args[0], 'rustdoc');
  assert.equal(rustdocCalls[1].rustcBootstrap, '1');
});

test('policy violations remain sorted, continue later crates, and produce final exit 1', async () => {
  const first = GATE_POLICY[0].name;
  const denied = GRAPH_CASES.find(({ id }) => id === 'stable-violation-sort').rustdoc;
  const rustdocs = Object.fromEntries(GATE_POLICY.map(({ name }) => [name, passingRustdoc(name)]));
  rustdocs[first] = denied;
  const result = await runPublicApiCli(['--all-configured'], { rustdocs });
  assert.equal(result.code, 1);
  assert.deepEqual(cargoKinds(result), [
    'metadata',
    'probe',
    ...Array(managedCrateCount).fill('rustdoc'),
  ]);
  assert.match(result.stdout, new RegExp(`^Public API allow-list for ${first}:`));
  assert.match(result.stdout, new RegExp(`Public API check passed for ${GATE_POLICY.at(-1).name}\\.\\n$`));
  assert.equal(
    (result.stdout.match(/Public API check passed/g) ?? []).length,
    managedCrateCount - 1,
  );
  const expectedFailure = (crateName) => [
    `Public API check failed for ${crateName}: 2 forbidden reference(s).`,
    'DENY matrix_crate::call signature input alpha references forbidden_dep::Denied from forbidden crate forbidden_dep',
    'DENY matrix_crate::call signature input zeta references forbidden_dep::Denied from forbidden crate forbidden_dep',
  ].join('\n');
  assert.equal(result.stderr, `${expectedFailure(first)}\n`);
});

test('metadata operational failures stop immediately', async () => {
  const failed = await runPublicApiCli(['--all-configured'], {
    metadataFailure: { code: 17, stderr: 'metadata exploded\n' },
  });
  assert.deepEqual(
    { code: failed.code, stderr: failed.stderr, stdout: failed.stdout },
    { code: 1, stderr: 'cargo metadata --format-version 1 --no-deps exited with 17\n', stdout: '' },
  );
  assert.deepEqual(cargoKinds(failed), ['metadata']);

  const invalid = await runPublicApiCli(['--all-configured'], { invalidMetadata: true });
  assert.equal(invalid.code, 1);
  assert.equal(invalid.stdout, '');
  assert.match(invalid.stderr, /^failed to parse cargo metadata JSON: /);
  assert.deepEqual(cargoKinds(invalid), ['metadata']);
});

test('Nth-crate rustdoc operational failure preserves prior streaming output and stops the session', async () => {
  const failingIndex = 1;
  const failingCrate = GATE_POLICY[failingIndex].name;
  const result = await runPublicApiCli(['--all-configured'], {
    rustdocFailures: {
      [failingCrate]: {
        bootstrap: { code: 42, stderr: 'bootstrap exploded\n' },
        nightly: { code: 41, stderr: 'nightly exploded\n' },
      },
    },
  });
  assert.equal(result.code, 1);
  assert.equal(result.stdout, expectedPassingOutput(GATE_POLICY.slice(0, failingIndex)));
  assert.match(result.stderr, new RegExp(`^failed to build rustdoc JSON for ${failingCrate}\\.`));
  assert.match(result.stderr, /cargo \+nightly rustdoc failed:/);
  assert.match(result.stderr, /RUSTC_BOOTSTRAP=1 cargo rustdoc failed:/);
  assert.deepEqual(
    cargoKinds(result),
    ['metadata', 'probe', ...Array(failingIndex).fill('rustdoc'), 'rustdoc', 'rustdoc'],
  );
  assert.equal(result.stderr.includes('Public API check failed'), false);
});

test('Nth-crate missing/invalid rustdoc JSON stops after preserving prior output', async () => {
  for (const mode of ['omitRustdoc', 'invalidRustdoc']) {
    const failingIndex = 1;
    const failingCrate = GATE_POLICY[failingIndex].name;
    const result = await runPublicApiCli(['--all-configured'], { [mode]: [failingCrate] });
    assert.equal(result.code, 1, mode);
    assert.equal(result.stdout, expectedPassingOutput(GATE_POLICY.slice(0, failingIndex)), mode);
    if (mode === 'omitRustdoc') {
      assert.match(result.stderr, /rustdoc JSON was not produced at .+\.json\n$/);
    } else {
      assert.match(result.stderr, /Unexpected token|Expected property name|JSON/);
    }
    assert.deepEqual(
      cargoKinds(result),
      ['metadata', 'probe', ...Array(failingIndex + 1).fill('rustdoc')],
      mode,
    );
    assert.equal(result.stderr.includes('Public API check failed'), false, mode);
  }
});

test('verify registry retains exactly one self-test and one all-configured invocation', () => {
  const entries = CHECKER_REGISTRY.filter(
    ({ path }) => path === 'scripts/check-crate-public-api.mjs',
  );
  assert.equal(entries.length, 1);
  assert.deepEqual(entries[0].invocations, [
    {
      args: ['--self-test'],
      id: 'checks:crate-public-api:self-test',
      selector: 'checks',
    },
    {
      args: ['--all-configured'],
      id: 'checks:crate-public-api:all-configured',
      selector: 'checks',
    },
  ]);
});

test('graph matrix declares every required branch exactly once', () => {
  assert.deepEqual(GRAPH_CASES.map(({ id }) => id), GRAPH_MATRIX_EXPECTED_IDS);
  assert.equal(new Set(GRAPH_MATRIX_EXPECTED_IDS).size, GRAPH_MATRIX_EXPECTED_IDS.length);
});

for (const caseDefinition of GRAPH_CASES) {
  test(`legacy CLI graph matrix: ${caseDefinition.id}`, async () => {
    const result = await runPublicApiCli(
      ['matrix-crate', '--allow-crate', 'allowed-dep'],
      {
        packageNames: ['matrix-crate'],
        rustdocs: { 'matrix-crate': caseDefinition.rustdoc },
      },
    );
    const expected = graphExpected(caseDefinition);
    assert.deepEqual(
      { code: result.code, stderr: result.stderr, stdout: result.stdout },
      expected,
      caseDefinition.id,
    );
    assert.deepEqual(cargoKinds(result), ['metadata', 'probe', 'rustdoc']);
  });
}
