import { basename, dirname, join } from 'node:path';

import {
  PROBE_TARGETED_CRATES,
  createProbeLedger,
  validateProbeOptions,
} from './platform-source-probe-contract.mjs';
import {
  artifactSnapshotForLedger,
  assertArtifactEvidence,
  assertHostAttempt,
  beginHostAttempt,
  completeHostAttempt,
  createArtifactEvidence,
  failThrownHostAttempt,
  inspectHostFixture,
  snapshotProbeArtifacts,
} from './platform-source-probe-evidence.mjs';
import { preflightPlatformSourceProbe } from './platform-source-probe-preflight.mjs';
import {
  addOwnedWorktree,
  cleanupProbeOwnership,
  createProbeOwnership,
} from './platform-source-probe-ownership.mjs';
import {
  commandFailure,
  commandText,
  createProbeDependencies,
  errorMessage,
  finalizeProbeDigest,
  ledgerTemporaryPath,
} from './platform-source-probe-support.mjs';
import { canonicalSkiffSourceTestRegistry } from './skiff-source-test-registry.mjs';

const PRELUDE_LABEL = 'PLATFORM_SOURCE_PRELUDE_IDENTITY';
const STD_LABEL = 'PLATFORM_SOURCE_STD_PACKAGE_BUILD_ID';

export async function runPlatformSourceSharedTargetProbe(options, overrides = {}) {
  const deps = createProbeDependencies({
    snapshotArtifacts: snapshotProbeArtifacts,
    ...overrides,
  });
  const input = validateProbeOptions(options);
  const ledger = createProbeLedger(input, deps.createNonce());
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
  let ownership;
  let primaryError;
  try {
    const prefix = join(
      dirname(input.integrationRoot),
      input.mode === 'combined' ? '.skiff-p5-i16.' : '.skiff-p5-g16.',
    );
    taskRoot = await deps.makeTempRoot(prefix);
    ownership = await createProbeOwnership({ input, deps, ledger, taskRoot });
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
      ownership,
    };
    if (input.mode === 'combined') await runCombined(state);
    else await runFull(state);
    abortController.signal.throwIfAborted();
    ledger.primary = { status: 'PASS', error: null };
  } catch (error) {
    primaryError = error;
    ledger.primary = { status: 'FAIL', error: errorMessage(error) };
  }

  await finishProbeCleanup({
    input,
    deps,
    ledger,
    ownership,
    taskRoot,
    handlers,
    interruptedBy,
    primaryError,
  });
  return input.mode === 'combined'
    ? await installCombinedLedger(input, deps, ledger)
    : finalizeProbeDigest(ledger);
}

async function finishProbeCleanup({
  input,
  deps,
  ledger,
  ownership,
  taskRoot,
  handlers,
  interruptedBy,
  primaryError,
}) {
  let ownershipCleanup;
  if (ownership !== undefined) {
    try {
      ownershipCleanup = await cleanupProbeOwnership(ownership, checked);
    } catch (error) {
      ownershipCleanup = failedOwnershipCleanup(ledger.probeNonce, taskRoot, error);
    }
  } else {
    const taskRootAbsent = taskRoot === undefined || !await deps.exists(taskRoot);
    ownershipCleanup = {
      nonce: ledger.probeNonce,
      worktrees: [],
      taskRoot: { path: taskRoot ?? null, absent: taskRootAbsent, error: null },
      foreign: { paths: [], registries: [], preserved: true },
      errors: taskRootAbsent ? [] : ['task root exists without a complete ownership marker'],
    };
  }
  ledger.ownership = ownershipCleanup;
  for (const [signal, handler] of handlers) deps.signalTarget.off(signal, handler);
  ledger.cleanup = {
    aWorktreeAbsent: !await deps.exists(input.aWorktree),
    bWorktreeAbsent: !await deps.exists(input.bWorktree),
    taskRootAbsent: ownershipCleanup.taskRoot.absent,
    processGroupsAbsent: ledger.processes.every((entry) => entry.absent === true),
    portsAbsent: ledger.ports.every((entry) => entry.absent === true),
    interruptedBy: interruptedBy ?? null,
    foreignPreserved: ownershipCleanup.foreign.preserved,
    errors: [...ownershipCleanup.errors],
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
  ledger.status = primaryError === undefined && ledger.cleanup.errors.length === 0
    ? 'PASS'
    : 'FAIL';
  ledger.firstError = primaryError === undefined
    ? (ledger.cleanup.errors[0] ?? null)
    : errorMessage(primaryError);
}

function failedOwnershipCleanup(nonce, taskRoot, error) {
  return {
    nonce,
    worktrees: [],
    taskRoot: { path: taskRoot, absent: false, error: errorMessage(error) },
    foreign: { paths: [], registries: [], preserved: false },
    errors: [errorMessage(error)],
  };
}

async function installCombinedLedger(input, deps, ledger) {
  const temporaryPath = ledgerTemporaryPath(input.ledger, ledger.probeNonce);
  ledger.output = {
    combinedLedger: input.ledger,
    atomicWrite: 'PASS',
    method: 'wx+flush+close+hard-link',
    temporaryPath,
    ownedTemporaryAbsent: true,
    foreignDestinationPreserved: true,
    error: null,
  };
  let result = finalizeProbeDigest(ledger);
  try {
    await deps.writeLedger(input.ledger, result, { nonce: ledger.probeNonce });
  } catch (error) {
    const message = `failed to write combined ledger atomically: ${errorMessage(error)}`;
    const evidence = error?.ledgerInstallEvidence;
    ledger.status = 'FAIL';
    ledger.firstError ??= message;
    ledger.output = {
      combinedLedger: input.ledger,
      atomicWrite: 'FAIL',
      method: evidence?.method ?? 'wx+flush+close+hard-link',
      temporaryPath: evidence?.temporaryPath ?? temporaryPath,
      ownedTemporaryAbsent: evidence?.ownedTemporaryAbsent === true,
      foreignDestinationPreserved: evidence?.foreignDestinationPreserved === true,
      foreignDestinationSha256: evidence?.foreignDestinationSha256 ?? null,
      cleanupErrors: evidence?.cleanupErrors ?? [],
      error: message,
    };
    result = finalizeProbeDigest(ledger);
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
  await buildOrigin(state, input.aWorktree, 'A-origin');
  probes.push(await identityProbe(state, input.aWorktree, input.aWorktree));
  const aBeforeB = await deps.snapshotArtifacts(sharedTarget);
  const aWithBOutcome = await runIdentityCommand(state, input.bWorktree, input.bWorktree);
  const aAfterB = await deps.snapshotArtifacts(sharedTarget);
  recordArtifactEvidence(state, {
    mode: 'combined',
    label: 'A-origin/B-root',
    outcome: aWithBOutcome,
    before: aBeforeB,
    after: aAfterB,
    sourceRoot: input.aWorktree,
    targetRoot: input.bWorktree,
    requireIdentity: true,
  });
  probes.push(parseIdentityProbe(input, input.bWorktree, input.bWorktree, aWithBOutcome));

  await buildOrigin(state, input.bWorktree, 'B-origin');
  probes.push(await identityProbe(state, input.bWorktree, input.bWorktree));
  const bBeforeA = await deps.snapshotArtifacts(sharedTarget);
  const bWithAOutcome = await runIdentityCommand(state, input.aWorktree, input.aWorktree);
  const bAfterA = await deps.snapshotArtifacts(sharedTarget);
  recordArtifactEvidence(state, {
    mode: 'combined',
    label: 'B-origin/A-root',
    outcome: bWithAOutcome,
    before: bBeforeA,
    after: bAfterA,
    sourceRoot: input.bWorktree,
    targetRoot: input.aWorktree,
    requireIdentity: true,
  });
  probes.push(parseIdentityProbe(input, input.aWorktree, input.aWorktree, bWithAOutcome));

  await buildOrigin(state, input.aWorktree, 'final-A-origin');
  const artifacts = await deps.snapshotArtifacts(sharedTarget);
  ledger.artifacts = artifactSnapshotForLedger(artifacts);
  ledger.identityProbes = probes.map(({ output, ...probe }) => probe);
  ledger.structure = await inspectStructure(state, artifacts);
  const registry = await deps.loadRegistry(input.aWorktree, input.candidate);
  if (JSON.stringify(registry) !== JSON.stringify(canonicalSkiffSourceTestRegistry)) {
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
  const outcome = await deps.runCommand('cargo', runnerBuildArgs(input.bWorktree), {
    cwd: input.bWorktree,
    env: targetEnv(sharedTarget),
    signal,
  });
  recordOwnedResources(ledger, outcome);
  const after = await deps.snapshotArtifacts(sharedTarget);
  recordArtifactEvidence(state, {
    mode: 'full',
    label: 'full A-origin/B-root',
    outcome,
    before,
    after,
    sourceRoot: input.aWorktree,
    targetRoot: input.bWorktree,
    requireIdentity: false,
  });
  ledger.artifacts = artifactSnapshotForLedger(after);
  const hostAssertionPath = join(
    input.bWorktree,
    'test-runner',
    'fixtures',
    'package-service-host',
    'consumer-tests',
    'main.test.skiff',
  );
  const hostSource = await deps.readText(hostAssertionPath);
  const fixture = inspectHostFixture(hostSource, hostAssertionPath);
  const hostArgs = [join(input.bWorktree, 'scripts/run-skiff-tests.mjs')];
  ledger.fullProbeRuns += 1;
  ledger.hostAttempt = beginHostAttempt('node', hostArgs);
  let gate;
  try {
    gate = await deps.runCommand('node', hostArgs, {
      cwd: taskRoot,
      env: targetEnv(sharedTarget),
      signal,
      observePorts: true,
    });
  } catch (error) {
    ledger.hostAttempt = failThrownHostAttempt(ledger.hostAttempt, error);
    throw error;
  }
  recordOwnedResources(ledger, gate);
  ledger.hostAttempt = completeHostAttempt(ledger.hostAttempt, gate, fixture, {
    processEvidencePresent: ledger.processes.some((entry) => entry.pid === gate.pid),
    portEvidencePresent: (gate.observedPorts?.length ?? 0) > 0
      && gate.observedPorts.every((port) => ledger.ports.some((entry) => entry.port === port)),
  });
  assertHostAttempt(ledger.hostAttempt);
  ledger.sourceSuite = ledger.hostAttempt.sourceSuite;
}

async function addWorktrees(state) {
  const { input, ownership, signal } = state;
  await addOwnedWorktree(ownership, 'A', input.aWorktree, checked, signal);
  await addOwnedWorktree(ownership, 'B', input.bWorktree, checked, signal);
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
  const outcome = await runIdentityCommand(state, manifestRoot, platformRoot);
  if (outcome.code !== 0 || outcome.signal !== null || outcome.error != null) {
    throw commandFailure('cargo', outcome);
  }
  return parseIdentityProbe(state.input, manifestRoot, platformRoot, outcome);
}

async function runIdentityCommand(state, manifestRoot, platformRoot) {
  const { deps, sharedTarget, signal } = state;
  const outcome = await deps.runCommand('cargo', [
    'test', '--locked', '--manifest-path', join(manifestRoot, 'test-runner', 'Cargo.toml'),
    '--test', 'test_service_flow',
    'platform_source_identity_probe', '-vv', '--', '--ignored', '--exact', '--nocapture',
  ], {
    cwd: manifestRoot,
    env: {
      ...targetEnv(sharedTarget),
      SKIFF_TEST_PLATFORM_SOURCE_ROOT: platformRoot,
    },
    signal,
  });
  recordOwnedResources(deps.ledger, outcome);
  return outcome;
}

function parseIdentityProbe(input, manifestRoot, platformRoot, outcome) {
  const output = commandText(outcome);
  const prelude = labeledValue(output, PRELUDE_LABEL);
  const std = labeledValue(output, STD_LABEL);
  if (prelude !== input.expectedPreludeIdentity || std !== input.expectedStdPackageBuildId) {
    throw new Error(`identity probe mismatch for ${manifestRoot} using ${platformRoot}`);
  }
  return { manifestRoot, platformRoot, preludeIdentity: prelude, stdPackageBuildId: std, output };
}

function recordArtifactEvidence(state, evidenceInput) {
  const evidence = createArtifactEvidence(evidenceInput);
  state.ledger.artifactEvidence.push(evidence);
  assertArtifactEvidence(evidence);
  return evidence;
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

async function runOwnedCommand(state, command, args, options = {}) {
  const outcome = await state.deps.runCommand(command, args, options);
  recordOwnedResources(state.ledger, outcome);
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
