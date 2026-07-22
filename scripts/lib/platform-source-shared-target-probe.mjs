import { basename, dirname, join } from 'node:path';

import {
  PROBE_TARGETED_CRATES,
  createProbeLedger,
  validateProbeOptions,
} from './platform-source-probe-contract.mjs';
import { preflightPlatformSourceProbe } from './platform-source-probe-preflight.mjs';
import {
  commandFailure,
  commandText,
  createProbeDependencies,
  errorMessage,
  finalizeProbeDigest,
} from './platform-source-probe-support.mjs';

const PRELUDE_LABEL = 'PLATFORM_SOURCE_PRELUDE_IDENTITY';
const STD_LABEL = 'PLATFORM_SOURCE_STD_PACKAGE_BUILD_ID';
const HOST_FINAL_VALUE = 'provider-observed-helper-mutated';
const HOST_FINAL_VALUE_ASSERTION = `assert root.main.run() == "${HOST_FINAL_VALUE}"`;

export async function runPlatformSourceSharedTargetProbe(options, overrides = {}) {
  const deps = createProbeDependencies(overrides);
  const input = validateProbeOptions(options);
  const ledger = createProbeLedger(input);
  deps.ledger = ledger;
  try {
    const preflight = await preflightPlatformSourceProbe(input, deps, checked);
    ledger.capacity = preflight.capacity;
    ledger.combinedLedger = preflight.combinedLedger;
  } catch (error) {
    ledger.status = 'PREFLIGHT BLOCKED';
    ledger.primary = { status: 'PREFLIGHT BLOCKED', error: errorMessage(error) };
    return ledger;
  }

  const abortController = new AbortController();
  let interruptedBy;
  const handlers = new Map(['SIGINT', 'SIGTERM'].map((signal) => [signal, () => {
    interruptedBy ??= signal;
    abortController.abort(new Error(`platform source probe interrupted by ${signal}`));
  }]));
  for (const [signal, handler] of handlers) deps.signalTarget.on(signal, handler);

  let taskRoot;
  let aAdded = false;
  let bAdded = false;
  let primaryError;
  try {
    const prefix = join(
      dirname(input.integrationRoot),
      input.mode === 'combined' ? '.skiff-p5-i16.' : '.skiff-p5-g16.',
    );
    taskRoot = await deps.makeTempRoot(prefix);
    const sharedTarget = join(taskRoot, 'cargo-target');
    await deps.mkdir(sharedTarget, { recursive: true });
    ledger.paths = {
      integrationRoot: input.integrationRoot,
      aWorktree: input.aWorktree,
      bWorktree: input.bWorktree,
      taskRoot,
      sharedTarget,
    };
    const state = {
      input,
      deps,
      ledger,
      taskRoot,
      sharedTarget,
      signal: abortController.signal,
      markAAdded: () => { aAdded = true; },
      markBAdded: () => { bAdded = true; },
    };
    if (input.mode === 'combined') await runCombined(state);
    else await runFull(state);
    abortController.signal.throwIfAborted();
    ledger.primary = { status: 'PASS', error: null };
  } catch (error) {
    primaryError = error;
    ledger.primary = { status: 'FAIL', error: errorMessage(error) };
  }

  const cleanupErrors = [];
  for (const [path, added] of [[input.bWorktree, bAdded], [input.aWorktree, aAdded]]) {
    if (!added) continue;
    try {
      await checked(deps, 'git', [
        '-C', input.integrationRoot, 'worktree', 'remove', '--force', path,
      ], { cwd: input.integrationRoot });
    } catch (error) {
      cleanupErrors.push(error);
    }
  }
  if (taskRoot !== undefined) {
    try { await deps.remove(taskRoot); } catch (error) { cleanupErrors.push(error); }
  }
  for (const [signal, handler] of handlers) deps.signalTarget.off(signal, handler);
  ledger.cleanup = {
    aWorktreeAbsent: !await deps.exists(input.aWorktree),
    bWorktreeAbsent: !await deps.exists(input.bWorktree),
    taskRootAbsent: taskRoot === undefined || !await deps.exists(taskRoot),
    processGroupsAbsent: ledger.processes.every((entry) => entry.absent === true),
    portsAbsent: ledger.ports.every((entry) => entry.absent === true),
    interruptedBy: interruptedBy ?? null,
    errors: cleanupErrors.map(errorMessage),
  };
  for (const [label, absent] of [
    ['A worktree', ledger.cleanup.aWorktreeAbsent],
    ['B worktree', ledger.cleanup.bWorktreeAbsent],
    ['task root', ledger.cleanup.taskRootAbsent],
    ['owned process group', ledger.cleanup.processGroupsAbsent],
    ['observed port', ledger.cleanup.portsAbsent],
  ]) {
    if (!absent) ledger.cleanup.errors.push(`${label} cleanup proof is not ABSENT`);
  }
  const cleanupPassed = ledger.cleanup.errors.length === 0;
  ledger.status = primaryError === undefined && cleanupPassed ? 'PASS' : 'FAIL';
  ledger.firstError = primaryError === undefined
    ? (ledger.cleanup.errors[0] ?? null)
    : errorMessage(primaryError);
  let result = finalizeProbeDigest(ledger);
  if (input.mode === 'combined') {
    ledger.output = { combinedLedger: input.ledger, atomicWrite: 'PASS', error: null };
    result = finalizeProbeDigest(ledger);
    try {
      await deps.writeLedger(input.ledger, result);
    } catch (error) {
      const message = `failed to write combined ledger atomically: ${errorMessage(error)}`;
      ledger.status = 'FAIL';
      ledger.firstError ??= message;
      ledger.output = { combinedLedger: input.ledger, atomicWrite: 'FAIL', error: message };
      result = finalizeProbeDigest(ledger);
    }
  }
  return result;
}

async function runCombined(state) {
  const { input, deps, ledger, sharedTarget, signal } = state;
  await checked(deps, 'node', [
    '--test',
    join(input.integrationRoot, 'scripts/tests/platform-source-transport-combined.test.mjs'),
  ], { cwd: input.integrationRoot, env: targetEnv(sharedTarget), signal });
  await checked(deps, 'cargo', [
    'check', '--locked', '--manifest-path', join(input.integrationRoot, 'Cargo.toml'),
    '-p', 'skiff-compiler', '-p', 'skiff-test-runner', '--bins',
  ], { cwd: input.integrationRoot, env: targetEnv(sharedTarget), signal });

  await addWorktrees(state);
  const probes = [];
  const fresh = [];
  await buildOrigin(state, input.aWorktree, 'A-origin');
  probes.push(await identityProbe(state, input.aWorktree, input.aWorktree));
  const aBeforeB = await deps.snapshotArtifacts(sharedTarget);
  const aWithB = await identityProbe(state, input.bWorktree, input.bWorktree);
  probes.push(aWithB);
  fresh.push(assertFresh(
    aWithB.output,
    aBeforeB,
    await deps.snapshotArtifacts(sharedTarget),
    'A-origin/B-root',
  ));

  await buildOrigin(state, input.bWorktree, 'B-origin');
  probes.push(await identityProbe(state, input.bWorktree, input.bWorktree));
  const bBeforeA = await deps.snapshotArtifacts(sharedTarget);
  const bWithA = await identityProbe(state, input.aWorktree, input.aWorktree);
  probes.push(bWithA);
  fresh.push(assertFresh(
    bWithA.output,
    bBeforeA,
    await deps.snapshotArtifacts(sharedTarget),
    'B-origin/A-root',
  ));

  await buildOrigin(state, input.aWorktree, 'final-A-origin');
  const artifacts = await deps.snapshotArtifacts(sharedTarget);
  ledger.artifacts = artifacts;
  ledger.identityProbes = probes.map(({ output, ...probe }) => probe);
  ledger.fresh = fresh;
  ledger.structure = await inspectStructure(state, artifacts);
  const registry = await deps.loadRegistry(input.aWorktree, input.candidate);
  if (JSON.stringify(registry) !== JSON.stringify([{ id: 'std', root: 'std' }])) {
    throw new Error('canonical Skiff source test registry changed');
  }
  ledger.registry = registry;
  ledger.fullProbeRuns = 0;
  ledger.sourceSuite = null;
}

async function runFull(state) {
  const { input, deps, ledger, sharedTarget, signal, taskRoot } = state;
  await addWorktrees(state);
  await buildOrigin(state, input.aWorktree, 'A-origin-full', { buildCompilerBinary: false });
  const before = await deps.snapshotArtifacts(sharedTarget);
  const outcome = await checked(deps, 'cargo', runnerBuildArgs(input.bWorktree), {
    cwd: input.bWorktree,
    env: targetEnv(sharedTarget),
    signal,
  });
  const after = await deps.snapshotArtifacts(sharedTarget);
  ledger.fresh = [assertFresh(
    commandText(outcome),
    before,
    after,
    'full A-origin/B-root',
    { requireIdentity: false },
  )];
  ledger.artifacts = after;
  const hostAssertionPath = join(
    input.bWorktree,
    'test-runner',
    'fixtures',
    'package-service-host',
    'consumer',
    'main.test.skiff',
  );
  const hostSource = await deps.readText(hostAssertionPath);
  const exactAssertions = hostSource
    .split(/\r?\n/)
    .filter((line) => line.trim() === HOST_FINAL_VALUE_ASSERTION);
  if (exactAssertions.length !== 1) {
    throw new Error(`Host fixture must assert ${HOST_FINAL_VALUE} exactly once`);
  }
  const gate = await checked(deps, 'node', [
    join(input.bWorktree, 'scripts/run-skiff-tests.mjs'),
  ], {
    cwd: taskRoot,
    env: targetEnv(sharedTarget),
    signal,
    observePorts: true,
  });
  const output = commandText(gate);
  if (ledger.ports.length === 0 || !ledger.processes.some((entry) => entry.pid === gate.pid)) {
    throw new Error('full gate omitted owned process/port cleanup evidence');
  }
  const counts = [...output.matchAll(/test result: ok\. (\d+) passed; 0 failed/g)]
    .map((match) => Number(match[1]));
  if (counts.length !== 2 || counts[0] !== 11 || counts[1] !== 1) {
    throw new Error(`full gate must report exact std 11/11 and Host 1/1, got ${counts.join('/')}`);
  }
  ledger.sourceSuite = {
    std: { passed: 11, total: 11 },
    host: { passed: 1, total: 1 },
    finalValue: HOST_FINAL_VALUE,
    finalValueEvidence: {
      assertionPath: hostAssertionPath,
      assertion: HOST_FINAL_VALUE_ASSERTION,
    },
  };
  ledger.fullProbeRuns = 1;
}

async function addWorktrees(state) {
  const { input, deps, signal } = state;
  await addWorktree(state, input.aWorktree, state.markAAdded, signal);
  await addWorktree(state, input.bWorktree, state.markBAdded, signal);
}

async function addWorktree(state, path, markAdded, signal) {
  const { input, deps } = state;
  try {
    await checked(deps, 'git', [
      '-C', input.integrationRoot, 'worktree', 'add', '--detach', path, input.candidate,
    ], { cwd: input.integrationRoot, signal });
    markAdded();
  } catch (error) {
    if (await deps.exists(path)) markAdded();
    throw error;
  }
}

async function buildOrigin(state, root, label, { buildCompilerBinary = true } = {}) {
  const { deps, ledger, sharedTarget, signal } = state;
  await checked(deps, 'cargo', [
    'clean', '--manifest-path', join(root, 'Cargo.toml'), '--target-dir', sharedTarget,
    ...PROBE_TARGETED_CRATES.flatMap((name) => ['-p', name]),
  ], { cwd: root, env: targetEnv(sharedTarget), signal });
  await checked(deps, 'cargo', runnerBuildArgs(root), {
    cwd: root, env: targetEnv(sharedTarget), signal,
  });
  if (buildCompilerBinary) {
    await checked(deps, 'cargo', [
      'build', '--locked', '--manifest-path', join(root, 'compiler', 'Cargo.toml'),
      '--bin', 'skiff-compiler', '-vv',
    ], { cwd: root, env: targetEnv(sharedTarget), signal });
  }
  ledger.rounds.push({ label, origin: root });
}

function runnerBuildArgs(root) {
  return [
    'build', '--locked', '--manifest-path', join(root, 'test-runner', 'Cargo.toml'),
    '--bin', 'skiff-test-runner',
    '--bin', 'skiff-package-service-smoke-fixture',
    '-vv',
  ];
}

async function identityProbe(state, manifestRoot, platformRoot) {
  const { deps, input, sharedTarget, signal } = state;
  const outcome = await checked(deps, 'cargo', [
    'test', '--locked', '--manifest-path', join(manifestRoot, 'test-runner', 'Cargo.toml'),
    '--test', 'package_service_contract_deployment',
    'platform_source_identity_probe', '-vv', '--', '--ignored', '--exact', '--nocapture',
  ], {
    cwd: manifestRoot,
    env: {
      ...targetEnv(sharedTarget),
      SKIFF_TEST_PLATFORM_SOURCE_ROOT: platformRoot,
    },
    signal,
  });
  const output = commandText(outcome);
  const prelude = labeledValue(output, PRELUDE_LABEL);
  const std = labeledValue(output, STD_LABEL);
  if (prelude !== input.expectedPreludeIdentity || std !== input.expectedStdPackageBuildId) {
    throw new Error(`identity probe mismatch for ${manifestRoot} using ${platformRoot}`);
  }
  return { manifestRoot, platformRoot, preludeIdentity: prelude, stdPackageBuildId: std, output };
}

async function inspectStructure(state, artifacts) {
  const { deps, signal } = state;
  const production = artifacts.filter((entry) => entry.structureSubject === true);
  const depInfo = artifacts.filter((entry) => entry.depInfo === true).map((entry) => entry.path);
  const missingProduction = missingArtifactSubjects(production.map((entry) => entry.path), [
    ['compiler input rlib', /^libskiff_compiler_input-[^.]+\.rlib$/],
    ['compiler source rlib', /^libskiff_compiler_source-[^.]+\.rlib$/],
    ['compiler binary', /^skiff-compiler$/],
    ['runner binary', /^skiff-test-runner$/],
    ['smoke binary', /^skiff-package-service-smoke-fixture$/],
  ]);
  const missingDepInfo = missingArtifactSubjects(depInfo, [
    ['compiler input dep-info', /^skiff_compiler_input(?:-[^.]+)?\.d$/],
    ['compiler source dep-info', /^skiff_compiler_source(?:-[^.]+)?\.d$/],
    ['compiler binary dep-info', /^skiff[-_]compiler(?:-[^.]+)?\.d$/],
    ['runner binary dep-info', /^skiff[-_]test[-_]runner(?:-[^.]+)?\.d$/],
    ['smoke binary dep-info', /^skiff[-_]package[-_]service[-_]smoke[-_]fixture(?:-[^.]+)?\.d$/],
  ]);
  if (missingProduction.length > 0 || missingDepInfo.length > 0) {
    throw new Error([
      ...missingProduction,
      ...missingDepInfo,
    ].join(', '));
  }
  const forbidden = /compiler[/\\]input[/\\.]+(?:std|prelude)|compiler[/\\]source[/\\.]+(?:std|prelude)/;
  for (const artifact of production) {
    const outcome = await checked(deps, 'strings', [artifact.path], { signal });
    if (forbidden.test(commandText(outcome))) {
      throw new Error(`production artifact embeds a compiler platform worktree path: ${artifact.path}`);
    }
  }
  const rg = await deps.runCommand('rg', ['# env-dep:CARGO_MANIFEST_DIR=', ...depInfo], { signal });
  recordOwnedResources(deps.ledger, rg);
  if (rg.code === 0) throw new Error('production dep-info retained CARGO_MANIFEST_DIR');
  if (rg.code !== 1) throw commandFailure('rg', rg);
  return {
    stringsNoMatch: production.map((entry) => entry.path),
    depInfoNoMatch: depInfo,
  };
}

function missingArtifactSubjects(paths, expected) {
  const names = paths.map((path) => basename(path));
  return expected
    .filter(([, pattern]) => !names.some((name) => pattern.test(name)))
    .map(([label]) => `shared target omitted ${label}`);
}

async function checked(deps, command, args, options = {}) {
  const outcome = await deps.runCommand(command, args, options);
  recordOwnedResources(deps.ledger, outcome);
  if (outcome.code !== 0) throw commandFailure(command, outcome);
  return outcome;
}

function recordOwnedResources(ledger, outcome) {
  if (Number.isInteger(outcome.pid)) {
    ledger.processes.push({ pid: outcome.pid, absent: outcome.processGroupAbsent === true });
  }
  for (const port of outcome.observedPorts ?? []) {
    ledger.ports.push({ port, absent: outcome.portsAbsent === true });
  }
}

function assertFresh(output, before, after, label, { requireIdentity = true } = {}) {
  const freshCrates = PROBE_TARGETED_CRATES.filter(
    (name) => new RegExp(`Fresh\\s+${name}\\b`).test(output),
  );
  if (freshCrates.length !== PROBE_TARGETED_CRATES.length) {
    throw new Error(`${label} omitted Fresh crate evidence: ${freshCrates.join(', ')}`);
  }
  if (JSON.stringify(before) !== JSON.stringify(after)) {
    throw new Error(`${label} changed shared-target artifact hash or mtime`);
  }
  if (requireIdentity && !before.some((entry) => entry.identityTest === true)) {
    throw new Error(`${label} omitted the identity integration-test artifact`);
  }
  return {
    label,
    crates: freshCrates,
    identityTargetFresh: requireIdentity,
    artifacts: before,
  };
}

function targetEnv(sharedTarget) {
  const env = { ...process.env, CARGO_TARGET_DIR: sharedTarget };
  delete env.SKIFF_TEST_PLATFORM_SOURCE_ROOT;
  return env;
}

function labeledValue(output, label) {
  const values = [...output.matchAll(new RegExp(`^${label}=(.+)$`, 'gm'))]
    .map((match) => match[1]);
  if (values.length !== 1) throw new Error(`identity probe must print ${label} exactly once`);
  return values[0];
}
