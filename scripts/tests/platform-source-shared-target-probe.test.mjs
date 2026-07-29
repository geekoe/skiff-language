import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { basename, dirname, join, relative } from 'node:path';
import test from 'node:test';

import {
  parseProbeArgs,
  runPlatformSourceSharedTargetProbe,
} from '../run-platform-source-shared-target-probe.mjs';
import {
  HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES,
  assertHostDiagnosticMatchesOutcome,
} from '../lib/platform-source-probe-diagnostic.mjs';
import { canonicalSkiffSourceTestRegistry } from '../lib/skiff-source-test-registry.mjs';
import { probeDigest } from '../lib/platform-source-probe-support.mjs';

const candidate = '1'.repeat(40);
const tree = '2'.repeat(40);
const lock = '3'.repeat(40);
const prelude = 'skiff-prelude-v1:sha256:prelude-golden';
const std = 'skiff-package-build-v4:sha256:std-golden';

test('combined and full modes remain disjoint command-double orchestrations', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = fixture.options('combined');
    const combinedDouble = fixture.commandDouble('combined');
    const combined = await runPlatformSourceSharedTargetProbe(
      combinedOptions,
      fixture.dependencies(combinedDouble),
    );
    assert.equal(combined.status, 'PASS', combined.firstError);
    assert.equal(combined.fullProbeRuns, 0);
    assert.equal(combined.sourceSuite, null);
    assert.equal(combined.schemaVersion, 'skiff-platform-source-shared-target-probe-v6');
    assert.equal(combined.artifactEvidence.length, 2);
    assert.equal(
      combined.artifactEvidence.every((entry) => (
        entry.comparator === 'strict-stable-artifact-v1'
        && entry.diff.changedCount === 0
      )),
      true,
    );
    assert.match(combined.probeNonce, /^[a-f0-9]{32}$/);
    assert.deepEqual(combined.ownership.worktrees.map((entry) => entry.label), ['A', 'B']);
    assert.equal(
      combined.ownership.worktrees.every((entry) => (
        entry.claimVerifiedBeforeRemoval && entry.pathAbsent && entry.registryAbsent
      )),
      true,
    );
    assert.equal(combined.ownership.taskRoot.markerVerifiedBeforeRemoval, true);
    assert.equal(combined.output.ownedTemporaryAbsent, true);
    assert.equal(combined.output.foreignDestinationPreserved, true);
    assert.equal(combined.identityProbes.length, 4);
    assert.deepEqual(combined.rounds.map((round) => round.label), [
      'A-origin', 'B-origin', 'final-A-origin',
    ]);
    assert.equal(
      combinedDouble.commands.filter(isMergeOnlyFixture).length,
      1,
    );
    assert.equal(combinedDouble.commands.filter(isRunSkiffTests).length, 0);
    assert.equal(combinedDouble.commands.filter(isRouterDependencyCommand).length, 0);
    assert.equal(combinedDouble.commands.filter(isIdentityProbe).length, 4);
    assert.equal(combinedDouble.commands.filter(isRunnerBuild).length, 3);
    assert.equal(combinedDouble.commands.filter(isCompilerBuild).length, 3);
    assert.equal(await absent(combined.paths.aWorktree), true);
    assert.equal(await absent(combined.paths.bWorktree), true);
    assert.equal(await absent(combined.paths.taskRoot), true);
    assert.deepEqual(
      JSON.parse(await readFile(combinedOptions.ledger, 'utf8')),
      combined,
    );

    const fullOptions = fixture.options('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const fullDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const full = await runPlatformSourceSharedTargetProbe(
      fullOptions,
      fixture.dependencies(fullDouble),
    );
    assert.equal(full.status, 'PASS', full.firstError);
    assert.equal(full.fullProbeRuns, 1);
    assert.deepEqual(full.sourceSuite, {
      std: { passed: 11, total: 11 },
      aliasReturnCatchOnce: { passed: 6, total: 6 },
      host: { passed: 4, total: 4 },
      finalValue: 'provider-observed-helper-mutated',
      finalValueEvidence: {
        passLine: 'PASS main.__test::provider observes helper mutation',
        assertionPath: join(
          fullOptions.bWorktree,
          'test-runner',
          'fixtures',
          'package-service-host',
          'consumer-tests',
          'main.test.skiff',
        ),
        assertion: 'assert subject/main.run() == "provider-observed-helper-mutated"',
      },
    });
    assert.equal(fullDouble.commands.filter(isMergeOnlyFixture).length, 0);
    assert.equal(fullDouble.commands.filter(isIdentityProbe).length, 0);
    assert.equal(fullDouble.commands.filter(isRunSkiffTests).length, 1);
    assert.equal(fullDouble.commands.filter(isRouterDependencyInstall).length, 1);
    assert.equal(fullDouble.commands.filter(isLocalRouterTsx).length, 1);
    assert.equal(fullDouble.commands.filter(isRunnerBuild).length, 2);
    assert.equal(fullDouble.commands.filter(isCompilerBuild).length, 0);
    assert.deepEqual(full.rounds.map((round) => round.label), ['A-origin-full']);
    assert.equal(full.artifactEvidence[0].comparator, 'full-root-materialization-v1');
    assert.equal(full.artifactEvidence[0].diff.rootMaterializations.length, 2);
    assert.equal(full.hostAttempt.status, 'PASS');
    assert.equal(full.hostAttempt.exactPassLineCount, 1);
    assert.equal(
      full.hostAttempt.expectedPassLine,
      'PASS <runtime-module-path>::provider observes helper mutation',
    );
    assert.equal(
      full.hostAttempt.observedPassLine,
      'PASS main.__test::provider observes helper mutation',
    );
    assert.equal(full.hostAttempt.passLines.length, 12);
    assert.match(full.hostAttempt.passLines[0], /^PASS <unexpected sha256:[a-f0-9]{64}>$/);
    assert.equal(
      full.hostAttempt.passLines[11],
      'PASS main.__test::provider observes helper mutation',
    );
    assert.match(full.hostAttempt.outputSha256, /^[a-f0-9]{64}$/);
    assert.equal(full.cleanup.processGroupsAbsent, true);
    assert.equal(full.cleanup.portsAbsent, true);
    assert.equal(full.nodeDependencies.status, 'PASS');
    const installIndex = fullDouble.commands.findIndex(isRouterDependencyInstall);
    const tsxIndex = fullDouble.commands.findIndex(isLocalRouterTsx);
    const hostIndex = fullDouble.commands.findIndex(isRunSkiffTests);
    const bBuildIndex = fullDouble.commands.findLastIndex(isRunnerBuild);
    const bWorktreeIndex = fullDouble.commands.findIndex(({ command, args }) => (
      command === 'git' && args.includes('add') && args.includes(fullOptions.bWorktree)
    ));
    assert.ok(bWorktreeIndex < installIndex);
    assert.ok(bBuildIndex < installIndex && installIndex < tsxIndex && tsxIndex < hostIndex);
    const install = fullDouble.commands[installIndex];
    assert.deepEqual(install.args, [
      '--dir', 'router', 'install', '--frozen-lockfile', '--offline',
    ]);
    assert.equal(install.options.cwd, fullOptions.bWorktree);
    assert.equal(fullDouble.commands[tsxIndex].options.cwd, fullOptions.bWorktree);
    assert.equal(
      fullDouble.commands[tsxIndex].command,
      join(fullOptions.bWorktree, 'router', 'node_modules', '.bin', 'tsx'),
    );
    const nodeModuleReferences = fullDouble.commands.flatMap(({ command, args }) => (
      [command, ...args].filter((value) => value.includes('node_modules'))
    ));
    assert.deepEqual(nodeModuleReferences, [
      join(fullOptions.bWorktree, 'router', 'node_modules', '.bin', 'tsx'),
    ]);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('Router dependencies fail closed before Host and preserve owned cleanup', async (t) => {
  const scenarios = [
    ['install-nonzero', 'install', 7, null],
    ['install-throw', 'install', null, null],
    ['tsx-signal', 'tsxExecutable', null, 'SIGTERM'],
  ];
  for (const [dependencyScenario, failedStep, code, signal] of scenarios) {
    await t.test(dependencyScenario, async () => {
      const fixture = await gateFixture();
      try {
        const combinedOptions = await createCombinedLedger(fixture);
        const commandDouble = fixture.commandDouble('full', {
          combinedLedger: combinedOptions.ledger,
          dependencyScenario,
        });
        const ledger = await runPlatformSourceSharedTargetProbe(
          fixture.options('full', { combinedLedger: combinedOptions.ledger }),
          fixture.dependencies(commandDouble),
        );
        assert.equal(ledger.status, 'FAIL');
        assert.equal(ledger.fullProbeRuns, 0);
        assert.equal(ledger.hostAttempt, null);
        assert.equal(ledger.nodeDependencies.status, 'FAIL');
        assert.equal(ledger.nodeDependencies.phase, 'router-dependencies');
        assert.equal(ledger.nodeDependencies[failedStep].code, code);
        assert.equal(ledger.nodeDependencies[failedStep].signal, signal);
        if (dependencyScenario === 'install-throw') {
          assert.equal(ledger.nodeDependencies[failedStep].stdoutSha256, null);
          assert.equal(ledger.nodeDependencies[failedStep].stderrSha256, null);
          assert.equal(ledger.nodeDependencies[failedStep].spawnError.code, 'ABORT_ERR');
        } else {
          assert.match(ledger.nodeDependencies[failedStep].stdoutSha256, /^[a-f0-9]{64}$/);
          assert.match(ledger.nodeDependencies[failedStep].stderrSha256, /^[a-f0-9]{64}$/);
        }
        assert.equal(
          JSON.stringify(ledger).includes('P5_F21C_DEPENDENCY_OUTPUT_SENTINEL'),
          false,
        );
        assert.match(ledger.firstError, /Gate Router dependency phase/);
        assert.equal(commandDouble.commands.filter(isRunSkiffTests).length, 0);
        assert.equal(commandDouble.commands.filter(isRouterDependencyInstall).length, 1);
        assert.equal(
          commandDouble.commands.filter(isLocalRouterTsx).length,
          dependencyScenario === 'tsx-signal' ? 1 : 0,
        );
        assert.equal(ledger.cleanup.aWorktreeAbsent, true);
        assert.equal(ledger.cleanup.bWorktreeAbsent, true);
        assert.equal(ledger.cleanup.taskRootAbsent, true);
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    });
  }
});

test('Router dependency primary failure remains first when worktree cleanup also fails', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = await createCombinedLedger(fixture);
    const commandDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
      dependencyScenario: 'install-nonzero',
      failRemove: true,
    });
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'FAIL');
    assert.match(ledger.firstError, /Router dependency phase install failed/);
    assert.equal(ledger.cleanup.errors.length > 0, true);
    assert.equal(
      commandDouble.commands.filter(({ command, args }) => (
        command === 'git' && args.includes('worktree') && args.includes('remove')
      )).length,
      2,
    );
    assert.equal(commandDouble.commands.filter(isRunSkiffTests).length, 0);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('candidate and combined-ledger mismatches block before build', async () => {
  const fixture = await gateFixture();
  try {
    const wrongCandidateDouble = fixture.commandDouble('combined');
    const wrongCandidate = await runPlatformSourceSharedTargetProbe(
      fixture.options('combined', { candidate: '4'.repeat(40) }),
      fixture.dependencies(wrongCandidateDouble),
    );
    assert.equal(wrongCandidate.status, 'PREFLIGHT BLOCKED');
    assert.match(wrongCandidate.primary.error, /candidate commit\/tree\/Cargo\.lock/);
    assert.equal(wrongCandidateDouble.commands.some(isBuildOrFixture), false);

    const combinedOptions = fixture.options('combined');
    const combinedDouble = fixture.commandDouble('combined');
    const combined = await runPlatformSourceSharedTargetProbe(
      combinedOptions,
      fixture.dependencies(combinedDouble),
    );
    assert.equal(combined.status, 'PASS');
    const tampered = JSON.parse(await readFile(combinedOptions.ledger, 'utf8'));
    tampered.candidate = '5'.repeat(40);
    await writeFile(combinedOptions.ledger, `${JSON.stringify(tampered)}\n`);

    const fullDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const full = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(fullDouble),
    );
    assert.equal(full.status, 'PREFLIGHT BLOCKED');
    assert.match(full.primary.error, /combined ledger digest mismatch/);
    assert.equal(fullDouble.commands.some(isBuildOrFixture), false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('pre-existing cleanup targets are refused and never removed', async () => {
  const fixture = await gateFixture();
  try {
    const options = fixture.options('combined');
    await mkdir(options.aWorktree);
    const commandDouble = fixture.commandDouble('combined');
    const ledger = await runPlatformSourceSharedTargetProbe(
      options,
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'PREFLIGHT BLOCKED');
    assert.match(ledger.primary.error, /worktree paths must be absent/);
    await access(options.aWorktree);
    assert.equal(commandDouble.commands.some(isBuildOrFixture), false);
    assert.equal(commandDouble.commands.some(({ args }) => args.includes('remove')), false);
    assert.equal(await absent(options.ledger), true);

    await rm(options.aWorktree, { recursive: true });
    await writeFile(options.ledger, 'preserve-existing-ledger\n');
    const ledgerDouble = fixture.commandDouble('combined');
    const ledgerRefusal = await runPlatformSourceSharedTargetProbe(
      options,
      fixture.dependencies(ledgerDouble),
    );
    assert.equal(ledgerRefusal.status, 'PREFLIGHT BLOCKED');
    assert.match(ledgerRefusal.primary.error, /ledger path must be absent/);
    assert.equal(await readFile(options.ledger, 'utf8'), 'preserve-existing-ledger\n');
    assert.equal(ledgerDouble.commands.some(isBuildOrFixture), false);
    assert.equal(ledgerDouble.commands.some(({ args }) => args.includes('remove')), false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('CLI parser requires an explicit mode and mode-specific ledger option', () => {
  const common = [
    '--integration-root', '/tmp/integration',
    '--candidate', candidate,
    '--expected-tree', tree,
    '--expected-lock-blob', lock,
    '--expected-prelude-identity', prelude,
    '--expected-std-package-build-id', std,
    '--a-worktree', '/tmp/a',
    '--b-worktree', '/tmp/b',
    '--json',
  ];
  assert.throws(() => parseProbeArgs(common), /--mode/);
  assert.throws(
    () => parseProbeArgs(['--mode', 'combined', ...common]),
    /--ledger requires an absolute path/,
  );
  assert.throws(
    () => parseProbeArgs(['--mode', 'full', ...common]),
    /--combined-ledger requires an absolute path/,
  );
});

test('primary failure remains first when owned worktree removal also fails', async () => {
  const fixture = await gateFixture();
  try {
    const commandDouble = fixture.commandDouble('combined', {
      failIdentity: true,
      failRemove: true,
    });
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('combined'),
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'FAIL');
    assert.match(ledger.firstError, /primary identity failure/);
    assert.equal(ledger.ownership.worktrees.length, 2);
    assert.equal(ledger.ownership.worktrees.every((entry) => entry.error !== null), true);
    assert.equal(
      commandDouble.commands.some(({ args }) => args.includes('--force')),
      false,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('combined comparator rejects even root-specific dep-info materialization', async () => {
  const fixture = await gateFixture();
  try {
    const commandDouble = fixture.commandDouble('combined');
    let snapshot = 0;
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('combined'),
      fixture.dependencies(commandDouble, {
        snapshotArtifacts: async (targetRoot) => {
          snapshot += 1;
          return fakeArtifacts(targetRoot, {
            materializationRoot: snapshot === 2 ? '/tmp/other-root' : '/tmp/combined-root',
          });
        },
      }),
    );
    assert.equal(ledger.status, 'FAIL');
    assert.equal(ledger.artifactEvidence.length, 1);
    assert.equal(ledger.artifactEvidence[0].comparator, 'strict-stable-artifact-v1');
    assert.equal(
      ledger.artifactEvidence[0].diff.firstDisallowed.classification,
      'root-specific-dep-info',
    );
    assert.equal(ledger.artifactEvidence[0].diff.firstDisallowed.allowed, false);
    assert.equal(ledger.artifactEvidence[0].diff.firstDisallowed.rootMaterialization, null);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('full artifact evidence blocks dependencies unless root materialization is exact', async (t) => {
  const scenarios = [
    ['binary-change', 'binary'],
    ['rlib-change', 'rlib'],
    ['hashed-dep-info-change', 'hashed-dep-info'],
    ['stable-mtime-change', 'binary'],
    ['illegal-root-materialization', 'root-specific-dep-info'],
    ['missing-fresh', 'missing-fresh'],
    ['fresh-conflict', 'conflicting-cargo-unit'],
  ];
  for (const [scenario, expected] of scenarios) {
    await t.test(scenario, async () => {
      const fixture = await gateFixture();
      try {
        const combinedOptions = await createCombinedLedger(fixture);
        const commandDouble = fixture.commandDouble('full', {
          combinedLedger: combinedOptions.ledger,
          artifactScenario: scenario,
        });
        const ledger = await runPlatformSourceSharedTargetProbe(
          fixture.options('full', { combinedLedger: combinedOptions.ledger }),
          fixture.dependencies(commandDouble),
        );
        assert.equal(ledger.status, 'FAIL');
        assert.equal(ledger.fullProbeRuns, 0);
        assert.equal(ledger.hostAttempt, null);
        assert.equal(commandDouble.commands.filter(isRouterDependencyCommand).length, 0);
        assert.equal(ledger.artifactEvidence.length, 1);
        const evidence = ledger.artifactEvidence[0];
        assert.equal(evidence.verdict, 'FAIL');
        if (expected === 'missing-fresh' || expected === 'conflicting-cargo-unit') {
          assert.equal(evidence.firstIssue.kind, expected);
        } else {
          assert.equal(evidence.firstIssue.kind, 'artifact-diff');
          assert.equal(evidence.firstIssue.classification, expected);
        }
        assert.ok(Array.isArray(evidence.before));
        assert.match(evidence.cargo.outputSha256, /^[a-f0-9]{64}$/);
        assert.ok(Array.isArray(evidence.after));
        assert.ok(Array.isArray(evidence.diff.entries));
        if (evidence.firstIssue.path !== null) {
          assert.equal(typeof evidence.firstIssue.before.sha256, 'string');
          assert.equal(typeof evidence.firstIssue.after.sha256, 'string');
          assert.equal(typeof evidence.firstIssue.before.mtimeMs, 'number');
          assert.equal(typeof evidence.firstIssue.after.size, 'number');
        }
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    });
  }
});

test('full Host attempt records nonzero, signal, and exact parse failures', async (t) => {
  const scenarios = [
    ['throw', 'command-throw'],
    ['nonzero', 'command-outcome'],
    ['signal', 'command-outcome'],
    ['malformed', 'result-counts'],
    ['extra-counts', 'result-counts'],
    ['wrong-pass', 'pass-line'],
    ['missing-pass', 'pass-line'],
    ['duplicate-pass', 'pass-line'],
    ['malformed-pass', 'pass-line-format'],
    ['oversized-pass', 'pass-line-format'],
  ];
  for (const [scenario, expectedIssue] of scenarios) {
    await t.test(scenario, async () => {
      const fixture = await gateFixture();
      try {
        const combinedOptions = await createCombinedLedger(fixture);
        const commandDouble = fixture.commandDouble('full', {
          combinedLedger: combinedOptions.ledger,
          hostScenario: scenario,
        });
        const ledger = await runPlatformSourceSharedTargetProbe(
          fixture.options('full', { combinedLedger: combinedOptions.ledger }),
          fixture.dependencies(commandDouble),
        );
        assert.equal(ledger.status, 'FAIL');
        assert.equal(ledger.fullProbeRuns, 1);
        assert.equal(ledger.sourceSuite, null);
        assert.equal(ledger.hostAttempt.status, 'FAIL');
        assert.equal(ledger.hostAttempt.firstIssue.kind, expectedIssue);
        if (scenario === 'throw') assert.equal(ledger.hostAttempt.outputSha256, null);
        else assert.match(ledger.hostAttempt.outputSha256, /^[a-f0-9]{64}$/);
        assert.equal(commandDouble.commands.filter(isRunSkiffTests).length, 1);
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    });
  }
});

test('bounded diagnostic retains phase while redacting paths, secrets, and HTTP bodies', async (t) => {
  const scenarios = [
    ['startup', 'startup', 'isolated-runtime'],
    ['std', 'std', 'std'],
    ['host-prepare', 'host-prepare', 'package-service-host'],
    ['host-runner', 'host-runner', 'package-service-host'],
    ['unknown', 'unknown', 'unknown'],
  ];
  for (const [scenario, expectedPhase, expectedSubject] of scenarios) {
    await t.test(scenario, async () => {
      const fixture = await gateFixture();
      try {
        const combinedOptions = await createCombinedLedger(fixture);
        const commandDouble = fixture.commandDouble('full', {
          combinedLedger: combinedOptions.ledger,
          hostScenario: `diagnostic-${scenario}`,
        });
        const ledger = await runPlatformSourceSharedTargetProbe(
          fixture.options('full', { combinedLedger: combinedOptions.ledger }),
          fixture.dependencies(commandDouble),
        );
        assert.equal(ledger.status, 'FAIL');
        assert.equal(ledger.hostAttempt.firstIssue.kind, 'command-outcome');
        assert.equal(ledger.hostAttempt.phase, expectedPhase);
        assert.equal(ledger.hostAttempt.subject, expectedSubject);
        assert.equal(
          ledger.hostAttempt.stdoutBytes,
          Buffer.byteLength(commandDouble.hostOutcome.stdout),
        );
        assert.equal(
          ledger.hostAttempt.stderrBytes,
          Buffer.byteLength(commandDouble.hostOutcome.stderr),
        );
        assert.equal(
          ledger.hostAttempt.stdoutSha256,
          createHash('sha256').update(commandDouble.hostOutcome.stdout).digest('hex'),
        );
        assert.equal(
          ledger.hostAttempt.stderrSha256,
          createHash('sha256').update(commandDouble.hostOutcome.stderr).digest('hex'),
        );
        assert.equal(
          ledger.hostAttempt.outputSha256,
          createHash('sha256').update(
            `${commandDouble.hostOutcome.stdout}\n${commandDouble.hostOutcome.stderr}`,
          ).digest('hex'),
        );
        const first = ledger.hostAttempt.diagnostics[0];
        assert.equal(first.kind, 'error');
        assert.equal(first.stream, 'stderr');
        assert.equal(first.truncated, true);
        assert.equal(ledger.hostAttempt.diagnostics.length, 3);
        assert.equal(ledger.hostAttempt.diagnosticOmittedCount, 1);
        assert.equal(
          first.originalLineSha256,
          createHash('sha256')
            .update(commandDouble.hostOutcome.stderr.split('\n')[0])
            .digest('hex'),
        );
        assert.ok(
          Buffer.byteLength(first.sanitizedExcerpt) <= HOST_DIAGNOSTIC_EXCERPT_MAX_BYTES,
        );
        assert.match(first.sanitizedExcerpt, /<PATH>/);
        assert.match(first.sanitizedExcerpt, /<REDACTED_SECRET>/);
        for (const path of commandDouble.diagnosticPaths) {
          assert.equal(first.sanitizedExcerpt.includes(path), false);
        }
        const serialized = JSON.stringify(ledger);
        assert.equal(serialized.includes('P5_F20A_SECRET_SENTINEL'), false);
        assert.equal(serialized.includes('P5_F20A_HTTP_BODY_SENTINEL'), false);
        assertHostDiagnosticMatchesOutcome(ledger.hostAttempt, commandDouble.hostOutcome);

        const tampered = structuredClone(ledger.hostAttempt);
        tampered.diagnostics[0].sanitizedExcerpt += 'tampered';
        assert.throws(
          () => assertHostDiagnosticMatchesOutcome(tampered, commandDouble.hostOutcome),
          /does not match the original command outcome/,
        );
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    });
  }
});

test('Host primary failure stays first when cleanup also fails', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = await createCombinedLedger(fixture);
    const commandDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
      hostScenario: 'nonzero',
      failRemove: true,
    });
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'FAIL');
    assert.equal(ledger.fullProbeRuns, 1);
    assert.equal(ledger.hostAttempt.firstIssue.kind, 'command-outcome');
    assert.match(ledger.firstError, /Host command failed/);
    assert.equal(ledger.cleanup.errors.length > 0, true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('unreachable expected assertion plus assert true is rejected before Host', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = await createCombinedLedger(fixture);
    const commandDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
      hostSource: [
        'test "provider observes helper mutation" {',
        '  if false {',
        '    assert subject/main.run() == "provider-observed-helper-mutated"',
        '  }',
        '  assert true',
        '}',
        '',
      ].join('\n'),
    });
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'FAIL');
    assert.equal(ledger.fullProbeRuns, 0);
    assert.equal(ledger.hostAttempt, null);
    assert.match(ledger.firstError, /one reachable assertion/);
    assert.equal(commandDouble.commands.filter(isRunSkiffTests).length, 0);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('legacy v5 combined ledger is explicitly invalid for full mode', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = await createCombinedLedger(fixture);
    const legacy = JSON.parse(await readFile(combinedOptions.ledger, 'utf8'));
    legacy.schemaVersion = 'skiff-platform-source-shared-target-probe-v5';
    await writeFile(combinedOptions.ledger, `${JSON.stringify(legacy)}\n`);
    const commandDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'PREFLIGHT BLOCKED');
    assert.match(ledger.primary.error, /harness schema/);
    assert.equal(commandDouble.commands.some(isBuildOrFixture), false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test('v6 validator recomputes strict artifact evidence instead of trusting verdict', async () => {
  const fixture = await gateFixture();
  try {
    const combinedOptions = await createCombinedLedger(fixture);
    const tampered = JSON.parse(await readFile(combinedOptions.ledger, 'utf8'));
    tampered.artifactEvidence[0].before[0].sha256 = 'f'.repeat(64);
    delete tampered.ledgerDigest;
    tampered.ledgerDigest = probeDigest(tampered);
    await writeFile(combinedOptions.ledger, `${JSON.stringify(tampered)}\n`);
    const commandDouble = fixture.commandDouble('full', {
      combinedLedger: combinedOptions.ledger,
    });
    const ledger = await runPlatformSourceSharedTargetProbe(
      fixture.options('full', { combinedLedger: combinedOptions.ledger }),
      fixture.dependencies(commandDouble),
    );
    assert.equal(ledger.status, 'PREFLIGHT BLOCKED');
    assert.match(ledger.primary.error, /identity, Fresh, or structure evidence is incomplete/);
    assert.equal(commandDouble.commands.some(isBuildOrFixture), false);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

async function createCombinedLedger(fixture) {
  const options = fixture.options('combined');
  const commandDouble = fixture.commandDouble('combined');
  const ledger = await runPlatformSourceSharedTargetProbe(
    options,
    fixture.dependencies(commandDouble),
  );
  assert.equal(ledger.status, 'PASS', ledger.firstError);
  return options;
}

async function gateFixture() {
  const root = await realpath(
    await mkdtemp(join(tmpdir(), 'skiff-platform-probe-double-')),
  );
  const integrationRoot = join(root, 'integration');
  await mkdir(integrationRoot);
  await writeFile(join(integrationRoot, 'Cargo.lock'), 'lock\n');
  return {
    root,
    options(mode, overrides = {}) {
      return {
        mode,
        integrationRoot,
        candidate,
        expectedTree: tree,
        expectedLockBlob: lock,
        expectedPreludeIdentity: prelude,
        expectedStdPackageBuildId: std,
        aWorktree: join(root, `${mode}-a`),
        bWorktree: join(root, `${mode}-b`),
        ...(mode === 'combined'
          ? { ledger: join(integrationRoot, '.p5-i16-combined-ledger.json') }
          : { combinedLedger: join(integrationRoot, '.p5-i16-combined-ledger.json') }),
        json: true,
        ...overrides,
      };
    },
    commandDouble(mode, options = {}) {
      return createCommandDouble({ integrationRoot, mode, ...options });
    },
    dependencies(commandDouble, overrides = {}) {
      return {
        signalTarget: new EventEmitter(),
        runCommand: commandDouble.run,
        availableBytes: async () => 16 * (1024 ** 3),
        allocatedBytes: async () => undefined,
        snapshotArtifacts: artifactSnapshotDouble(commandDouble),
        loadRegistry: async () => canonicalSkiffSourceTestRegistry,
        readText: async () => commandDouble.hostSource ?? [
          'test "provider observes helper mutation" {',
          '  assert subject/main.run() == "provider-observed-helper-mutated"',
          '}',
          '',
        ].join('\n'),
        assertPortsClosed: async () => {},
        assertExecutables: async () => {},
        ...overrides,
      };
    },
  };
}

function createCommandDouble({
  integrationRoot,
  mode,
  combinedLedger,
  failIdentity = false,
  failRemove = false,
  artifactScenario = 'legal-root-materialization',
  hostScenario = 'pass',
  dependencyScenario = 'pass',
  hostSource,
}) {
  const commands = [];
  const worktrees = new Map([[integrationRoot, { head: candidate, detached: false }]]);
  const sourceRoot = join(dirname(integrationRoot), `${mode}-a`);
  const targetRoot = join(dirname(integrationRoot), `${mode}-b`);
  const diagnosticPaths = [
    integrationRoot,
    sourceRoot,
    targetRoot,
    join(dirname(integrationRoot), '.skiff-p5-g16.diagnostic'),
    join(tmpdir(), 'skiff-f20a-temp'),
    join(homedir(), 'skiff-f20a-home'),
  ];
  let pid = 10_000;
  let hostOutcome = null;
  const run = async (command, args, options = {}) => {
    commands.push({ command, args: [...args], options });
    const outcome = {
      code: 0,
      signal: null,
      error: null,
      stdout: '',
      stderr: '',
      pid: pid += 1,
      processGroupAbsent: true,
      observedPorts: [],
      portsAbsent: true,
    };
    if (command === 'git' && args.includes('rev-parse')) {
      const expression = args.at(-1);
      outcome.stdout = `${expression === 'HEAD' ? candidate : expression === 'HEAD^{tree}' ? tree : lock}\n`;
    } else if (command === 'git' && args.includes('status')) {
      if (mode === 'full' && combinedLedger !== undefined) {
        outcome.stdout = `?? ${relative(integrationRoot, combinedLedger)}\n`;
      }
    } else if (command === 'git' && args.includes('worktree') && args.includes('list')) {
      outcome.stdout = [...worktrees].map(([path, entry]) => [
        `worktree ${path}`,
        `HEAD ${entry.head}`,
        entry.detached ? 'detached' : 'branch refs/heads/integration',
        '',
      ].join('\0')).join('\0');
    } else if (command === 'git' && args.includes('worktree') && args.includes('add')) {
      const path = args[args.indexOf('add') + 2];
      const adminPath = join(integrationRoot, `.git-worktree-${basename(path)}`);
      await mkdir(path, { recursive: true });
      await mkdir(adminPath);
      await writeFile(join(path, '.git'), `gitdir: ${adminPath}\n`);
      worktrees.set(path, { head: candidate, detached: true, adminPath });
    } else if (command === 'git' && args.includes('worktree') && args.includes('remove')) {
      const path = args[args.indexOf('remove') + 1];
      if (failRemove) {
        outcome.code = 8;
        outcome.stderr = 'owned remove failure';
      } else {
        await rm(path, { recursive: true, force: true });
        await rm(worktrees.get(path).adminPath, { recursive: true, force: true });
        worktrees.delete(path);
      }
    } else if (command === 'cargo' && args.includes('platform_source_identity_probe')) {
      outcome.stdout = [
        `PLATFORM_SOURCE_PRELUDE_IDENTITY=${prelude}`,
        `PLATFORM_SOURCE_STD_PACKAGE_BUILD_ID=${std}`,
      ].join('\n');
      outcome.stderr = freshOutput();
      if (failIdentity) {
        outcome.code = 7;
        outcome.stderr += '\nprimary identity failure';
      }
    } else if (command === 'cargo' && args[0] === 'build') {
      outcome.stderr = freshOutput({
        missing: artifactScenario === 'missing-fresh' ? 'skiff-compiler-source' : null,
        conflict: artifactScenario === 'fresh-conflict' ? 'skiff-compiler' : null,
      });
    } else if (command === 'pnpm' && args.includes('install')) {
      if (dependencyScenario === 'install-throw') {
        const error = new Error('P5_F21C_DEPENDENCY_OUTPUT_SENTINEL');
        error.code = 'ABORT_ERR';
        throw error;
      }
      if (dependencyScenario === 'install-nonzero') {
        outcome.code = 7;
        outcome.stderr = 'offline dependency cache is incomplete P5_F21C_DEPENDENCY_OUTPUT_SENTINEL';
      }
    } else if (command.endsWith(join('router', 'node_modules', '.bin', 'tsx'))) {
      outcome.stdout = 'tsx v4.21.0\n';
      if (dependencyScenario === 'tsx-signal') {
        outcome.code = null;
        outcome.signal = 'SIGTERM';
        outcome.stderr = 'tsx stopped P5_F21C_DEPENDENCY_OUTPUT_SENTINEL';
      }
    } else if (command === 'rg') {
      outcome.code = 1;
    } else if (command === 'node' && args.some((value) => value.endsWith('run-skiff-tests.mjs'))) {
      applyHostCommandScenario(outcome, hostScenario, diagnosticPaths);
      hostOutcome = { ...outcome };
    }
    return outcome;
  };
  return {
    commands,
    run,
    mode,
    artifactScenario,
    hostScenario,
    hostSource,
    sourceRoot,
    targetRoot,
    diagnosticPaths,
    get hostOutcome() { return hostOutcome; },
  };
}

function applyHostCommandScenario(outcome, scenario, diagnosticPaths) {
  if (scenario === 'throw') throw new Error('Host command threw after launch');
  if (scenario.startsWith('diagnostic-')) {
    const phase = scenario.slice('diagnostic-'.length);
    const phaseMarkers = {
      startup: `[skiff-test] isolated runtime workspace: ${diagnosticPaths[4]}`,
      std: `[skiff-tests] running std: ${join(diagnosticPaths[0], 'test-services', 'std')}`,
      'host-prepare': `[skiff-tests] preparing package-service-host: ${join(diagnosticPaths[2], 'test-runner', 'fixtures', 'package-service-host')}`,
      'host-runner': '[skiff-tests] running package-service-host: test-runner/fixtures/package-service-host/consumer-tests',
      unknown: '[skiff-tests] unclassified source-suite output',
    };
    outcome.code = 1;
    outcome.stdout = [
      phaseMarkers[phase],
      '[skiff-instance] stopping after startup failure',
    ].join('\n');
    outcome.stderr = [
      `[skiff-instance] supervisor failure: ${phase} paths=${diagnosticPaths.join(',')} secret=P5_F20A_SECRET_SENTINEL ${'x'.repeat(700)}`,
      `error: secondary ${phase} diagnostic`,
      'HTTP body: P5_F20A_HTTP_BODY_SENTINEL',
    ].join('\n');
    outcome.observedPorts = [46010, 46011, 46012];
    return;
  }
  const lines = [
    ...Array.from(
      { length: 11 },
      (_unused, index) => `PASS std.case_${index}.__test::unrelated std case ${index}`,
    ),
    'test result: ok. 11 passed; 0 failed',
    'test result: ok. 6 passed; 0 failed',
    'PASS main.__test::provider observes helper mutation',
    'test result: ok. 4 passed; 0 failed',
  ];
  const targetIndex = 13;
  if (scenario === 'nonzero') {
    outcome.code = 9;
    outcome.stderr = 'Host command failed after launch';
  } else if (scenario === 'signal') {
    outcome.code = null;
    outcome.signal = 'SIGTERM';
  } else if (scenario === 'malformed') {
    lines[lines.length - 1] = 'test result: malformed';
  } else if (scenario === 'extra-counts') {
    lines.push('test result: ok. 1 passed; 0 failed');
  } else if (scenario === 'wrong-pass') {
    lines[targetIndex] = 'PASS main.__test::wrong observation';
  } else if (scenario === 'missing-pass') {
    lines.splice(targetIndex, 1);
  } else if (scenario === 'duplicate-pass') {
    lines.splice(targetIndex, 0, lines[targetIndex]);
  } else if (scenario === 'malformed-pass') {
    lines[targetIndex] = 'PASS ::provider observes helper mutation';
  } else if (scenario === 'oversized-pass') {
    lines[targetIndex] = `PASS main.__test::${'x'.repeat(600)}`;
  }
  outcome.stdout = lines.join('\n');
  outcome.observedPorts = [46010, 46011, 46012];
}

function artifactSnapshotDouble(commandDouble) {
  let call = 0;
  return async (targetRoot) => {
    const phase = commandDouble.mode === 'full' && call > 0 ? 'after' : 'before';
    call += 1;
    return fakeArtifacts(targetRoot, {
      materializationRoot: phase === 'before'
        ? commandDouble.sourceRoot
        : commandDouble.targetRoot,
      scenario: phase === 'after' ? commandDouble.artifactScenario : 'before',
      includeCompilerDepInfo: commandDouble.mode === 'combined',
    });
  };
}

function fakeArtifacts(targetRoot, {
  materializationRoot = '/tmp/combined-root',
  scenario = 'before',
  includeCompilerDepInfo = true,
} = {}) {
  const debug = join(targetRoot, 'debug');
  const deps = join(debug, 'deps');
  const artifacts = [
    structure(join(debug, 'skiff-compiler')),
    structure(join(debug, 'skiff-test-runner')),
    structure(join(debug, 'skiff-package-service-smoke-fixture')),
    structure(join(deps, 'libskiff_compiler_input-a.rlib')),
    structure(join(deps, 'libskiff_compiler_source-b.rlib')),
    structure(join(deps, 'libskiff_compiler-c.rlib')),
    {
      path: join(deps, 'test_service_flow-d'),
      sha256: fakeSha('identity'),
      mtimeMs: 1,
      size: 1,
      classification: 'identity-test',
      depInfo: false,
      structureSubject: false,
      identityTest: true,
    },
    depInfo(join(deps, 'skiff_compiler_input-a.d')),
    depInfo(join(deps, 'skiff_compiler_source-b.d')),
    ...(includeCompilerDepInfo
      ? [depInfo(join(debug, 'skiff-compiler.d'), { materializationRoot })]
      : []),
    depInfo(join(debug, 'skiff-test-runner.d'), { materializationRoot }),
    depInfo(join(debug, 'skiff-package-service-smoke-fixture.d'), { materializationRoot }),
  ];
  if (scenario === 'binary-change') mutateArtifact(artifacts, 'skiff-test-runner', 'sha256');
  if (scenario === 'rlib-change') mutateArtifact(artifacts, 'libskiff_compiler_input-a.rlib', 'sha256');
  if (scenario === 'hashed-dep-info-change') {
    mutateArtifact(artifacts, 'skiff_compiler_input-a.d', 'sha256');
  }
  if (scenario === 'stable-mtime-change') mutateArtifact(artifacts, 'skiff-test-runner', 'mtimeMs');
  if (scenario === 'illegal-root-materialization') {
    const entry = artifacts.find((artifact) => basename(artifact.path) === 'skiff-test-runner.d');
    entry.materializationText += '\nforeign-change';
    entry.sha256 = fakeSha('illegal-root-materialization');
    entry.size = entry.materializationText.length;
  }
  return artifacts;
}

function structure(path) {
  return {
    path,
    sha256: fakeSha(basename(path)),
    mtimeMs: 1,
    size: 1,
    classification: basename(path).endsWith('.rlib') ? 'rlib' : 'binary',
    depInfo: false,
    structureSubject: true,
    identityTest: false,
  };
}

function depInfo(path, { materializationRoot } = {}) {
  const rootSpecific = materializationRoot !== undefined;
  const materializationText = rootSpecific
    ? `${path}: ${materializationRoot}/Cargo.toml ${materializationRoot}/src/main.rs`
    : undefined;
  return {
    path,
    sha256: fakeSha(materializationText ?? basename(path)),
    mtimeMs: rootSpecific ? materializationRoot.length : 1,
    size: materializationText?.length ?? 1,
    classification: rootSpecific ? 'root-specific-dep-info' : 'hashed-dep-info',
    depInfo: true,
    structureSubject: false,
    identityTest: false,
    ...(rootSpecific ? { materializationText } : {}),
  };
}

function mutateArtifact(artifacts, name, field) {
  const entry = artifacts.find((artifact) => basename(artifact.path) === name);
  entry[field] = field === 'mtimeMs' ? entry[field] + 1 : fakeSha(`${entry[field]}-changed`);
}

function fakeSha(value) {
  return createHash('sha256').update(value).digest('hex');
}

function freshOutput({ missing = null, conflict = null } = {}) {
  const lines = [
    'Fresh skiff-test-runner v0.1.0',
    'Fresh skiff-compiler v0.1.0',
    'Fresh skiff-compiler-input v0.1.0',
    'Fresh skiff-compiler-source v0.1.0',
  ].filter((line) => !line.includes(` ${missing} `));
  if (conflict !== null) lines.push(`Dirty ${conflict} v0.1.0`);
  return lines.join('\n');
}

function isMergeOnlyFixture({ command, args }) {
  return command === 'node'
    && args.some((value) => value.endsWith('platform-source-transport-combined.test.mjs'));
}

function isRunSkiffTests({ command, args }) {
  return command === 'node' && args.some((value) => value.endsWith('run-skiff-tests.mjs'));
}

function isRouterDependencyInstall({ command, args }) {
  return command === 'pnpm'
    && JSON.stringify(args) === JSON.stringify([
      '--dir', 'router', 'install', '--frozen-lockfile', '--offline',
    ]);
}

function isLocalRouterTsx({ command, args }) {
  return command.endsWith(join('router', 'node_modules', '.bin', 'tsx'))
    && JSON.stringify(args) === JSON.stringify(['--version']);
}

function isRouterDependencyCommand(entry) {
  return isRouterDependencyInstall(entry) || isLocalRouterTsx(entry);
}

function isIdentityProbe({ command, args }) {
  return command === 'cargo' && args.includes('platform_source_identity_probe');
}

function isRunnerBuild({ command, args }) {
  return command === 'cargo' && args[0] === 'build'
    && args.includes('skiff-test-runner')
    && args.includes('skiff-package-service-smoke-fixture');
}

function isCompilerBuild({ command, args }) {
  return command === 'cargo' && args[0] === 'build'
    && args.includes('skiff-compiler')
    && !args.includes('skiff-test-runner');
}

function isBuildOrFixture(entry) {
  return entry.command === 'cargo' || entry.command === 'node';
}

async function absent(path) {
  try {
    await access(path);
    return false;
  } catch (error) {
    if (error.code === 'ENOENT') return true;
    throw error;
  }
}
