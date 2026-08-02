#!/usr/bin/env node
// Router Rust migration Batch 10 — rollback final gate (plan §8/§11.2).
//
// Release-candidate level rollback rehearsal:
//
//   1. Builds the immutable TS rollback unit in a fresh temporary directory
//      (pinned self-contained Node runtime + last TS source + materialized
//      Router dependencies + package/lockfile + process spec + all file/
//      source identity), verifies it, and copies it to a second fresh
//      directory to prove relocatable immutability.
//   2. Runs the final process-switch rehearsal over one real Runtime process
//      and one real relay: ts-workspace -> ts-unit -> rust -> ts-unit-
//      relocated. Every phase follows stop admission -> shutdown -> verify
//      PID/listener exit -> start target -> Runtime reconnect exact
//      committed tuple -> activation/readiness -> open admission -> HTTP
//      unary smoke. TS phases additionally assert `/__router/health`
//      readiness; Rust phases use the recorded empty-health boundary plus
//      the relay bootstrap tuple.
//   3. Prepares the clean-host bundle (binary + config + artifacts + sh
//      start scripts) and runs the local equivalent with a PATH that cannot
//      resolve pnpm/tsx.
//
// The harness never touches the stable instance, stable Mongo, PM2 or the
// fixed 4004-4007 ports: it uses a temporary Mongo replica set and leased
// ports in 45000-45999. It never reuses the workspace `router/node_modules`
// at startup (the unit is a dereferenced, hash-verified copy in a fresh
// temp directory) and never touches the network after the unit is built.

import { execFile, spawn } from 'node:child_process';
import {
  access,
  mkdir,
  mkdtemp,
  open,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import {
  assertCleanHostBundle,
  assertNoPnpmOrTsxOnPath,
  buildCleanHostBundle,
  cleanHostEnv,
} from './lib/clean-host-bundle.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import {
  assertRouterProcessSpec,
  resolveRouterProcessSpec,
  routerProcessInvocation,
} from './lib/dev-runtime-paths.mjs';
import {
  HTTP_LIVE_ENVIRONMENT,
  HTTP_LIVE_GENERATION,
  HTTP_LIVE_REPLICA_ID,
  HTTP_LIVE_SERVICE_ID,
  HTTP_LIVE_VERSION,
  authorHttpLiveArtifact,
  httpLiveMongoUrl,
  seedHttpLiveCommittedState,
  writeHttpLiveServiceSource,
} from './lib/http_live_fixture.mjs';
import {
  assertRouterExit,
  assertRouterPortsClosed,
  closeLogs,
  ensureTsRouterDependencies,
  installHttpLiveRustBinary,
  latestBootstrapTupleAfter,
  renderHttpLiveRouterConfig,
  spawnLoggedProcess,
  stopProcess,
  waitForHandshakeAfter,
  waitForListeners,
  writeHttpLiveRouterConfig,
  writeHttpLiveRuntimeConfig,
} from './lib/http_live_process.mjs';
import { runRollbackSuite } from './lib/http_live_suite.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
import {
  assertRouterRollbackManifest,
  buildRouterRollbackManifest,
  resolveRouterRollbackUnitProcess,
} from './lib/rollback-manifest.mjs';
import {
  assertImmutableTsRollbackUnit,
  buildImmutableTsRollbackUnit,
  copyImmutableTsRollbackUnit,
  discoverPinnedNodeRuntimeDir,
} from './lib/rollback-unit.mjs';
import { createRollbackRelay } from './lib/rollback-relay.mjs';
import { runCleanHostHttpSuite } from './lib/rollback-clean-host-suite.mjs';
import { renderRuntimeConfig } from './lib/runtime-stack-config.mjs';

const execFileAsync = promisify(execFile);

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);

let mongoHarness;
let portLease;
let tempRoot;
let currentRelay;
let currentRuntime;

const evidence = {
  manifests: null,
  unit: null,
  switchPlan: null,
  admission: [],
  phases: [],
  runtimeExits: [],
  cleanHost: null,
  relayTails: [],
};

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-rollback-final-'));
  const sourceRoot = join(tempRoot, 'src');
  console.log('router-rollback-final: writing real HTTP service source');
  await writeHttpLiveServiceSource(sourceRoot, { burstSleepMs: 2_000 });

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });
  console.log('router-rollback-final: compiling real package/assembly/config artifact');
  const identities = await authorHttpLiveArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    environment: HTTP_LIVE_ENVIRONMENT,
  });

  console.log('router-rollback-final: leasing isolated router + relay ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 3,
  });
  portLease = { release };
  const [httpPort, runtimePort, relayPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log('router-rollback-final: starting isolated Mongo replica set');
  mongoHarness = await ActivationStateMongoHarness.create({ repoRoot });
  await mongoHarness.start();

  const mongoUrl = httpLiveMongoUrl(mongoHarness.port);
  console.log('router-rollback-final: seeding committed activation state');
  const committed = await seedHttpLiveCommittedState({
    mongoUrl,
    environment: HTTP_LIVE_ENVIRONMENT,
    generation: HTTP_LIVE_GENERATION,
    assemblyIdentity: identities.assemblyIdentity,
    configSnapshotId: identities.configSnapshotId,
  });

  console.log('router-rollback-final: ensuring TS router dependencies (frozen lockfile)');
  await ensureTsRouterDependencies({ repoRoot });

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-rollback-final: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const routerSourceBinary = join(targetDir, 'debug', 'skiff-router');
  await access(routerSourceBinary);

  console.log('router-rollback-final: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const devHome = join(tempRoot, 'dev-home');
  const routerConfigPath = join(devHome, 'router.yml');
  await writeHttpLiveRouterConfig(
    routerConfigPath,
    renderHttpLiveRouterConfig({
      environment: HTTP_LIVE_ENVIRONMENT,
      artifactsPath: artifactRoot,
      httpPort,
      runtimePort,
      mongoUrl,
    }),
  );
  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });
  await writeFile(join(runtimeHome, 'runtime-id'), `${HTTP_LIVE_REPLICA_ID}\n`);
  const runtimeConfigPath = join(tempRoot, 'runtime.yml');
  await writeHttpLiveRuntimeConfig(runtimeConfigPath, {
    relayPort,
    runtimeHome,
    environment: HTTP_LIVE_ENVIRONMENT,
  });

  const tsSpec = resolveRouterProcessSpec({
    devHome,
    implementation: 'ts',
    repoRoot,
  });
  const rustSpec = resolveRouterProcessSpec({
    devHome,
    implementation: 'rust',
    repoRoot,
  });
  assertRouterProcessSpec(tsSpec);
  assertRouterProcessSpec(rustSpec);
  evidence.manifests = {
    ts: assertRouterRollbackManifest(buildRouterRollbackManifest(tsSpec)),
    rust: assertRouterRollbackManifest(buildRouterRollbackManifest(rustSpec)),
  };

  console.log('router-rollback-final: building immutable TS rollback unit');
  const nodeRuntimeDir = await discoverPinnedNodeRuntimeDir();
  const sourceCommit = (await execFileAsync(
    'git',
    ['rev-parse', 'HEAD'],
    { cwd: repoRoot },
  )).stdout.trim();
  const unitRoot = join(tempRoot, 'ts-unit');
  const builtUnit = await buildImmutableTsRollbackUnit({
    unitRoot,
    repoRoot,
    nodeRuntimeDir,
    configPath: routerConfigPath,
    sourceCommit,
  });
  const unitManifest = await assertImmutableTsRollbackUnit(unitRoot);
  evidence.unit = {
    unitRoot,
    manifestPath: builtUnit.manifestPath,
    sourceCommit,
    pinnedNode: unitManifest.pinned_node,
    fileCount: unitManifest.file_count,
    sha256Tree: unitManifest.sha256_tree,
  };
  evidence.switchPlan = unitManifest.switch_commands;

  console.log('router-rollback-final: copying unit to a second fresh directory');
  const relocatedUnitRoot = join(tempRoot, 'ts-unit-relocated');
  const relocatedUnit = await copyImmutableTsRollbackUnit(unitRoot, relocatedUnitRoot);
  evidence.unit.relocated = {
    unitRoot: relocatedUnit.unitRoot,
    verified: true,
    sha256Tree: relocatedUnit.manifest.sha256_tree,
  };

  console.log('router-rollback-final: installing explicit Rust router binary into dev home');
  await installHttpLiveRustBinary({ sourceBinary: routerSourceBinary, devHome });

  console.log('router-rollback-final: starting persistent relay + Runtime');
  currentRelay = await createRollbackRelay({
    port: relayPort,
    routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
  });
  const rollbackRuntimeLogs = join(tempRoot, 'rollback-runtime');
  await mkdir(rollbackRuntimeLogs, { recursive: true });
  currentRuntime = await spawnLoggedProcess(runtimeBin, [runtimeConfigPath], {
    cwd: repoRoot,
    stdoutPath: join(rollbackRuntimeLogs, 'runtime.stdout.log'),
    stderrPath: join(rollbackRuntimeLogs, 'runtime.stderr.log'),
  });

  const unitEnvHome = join(tempRoot, 'unit-empty-home');
  await mkdir(unitEnvHome, { recursive: true });
  const unitEnv = cleanHostEnv(process.env, { home: unitEnvHome });
  await assertNoPnpmOrTsxOnPath({ env: unitEnv, label: 'immutable unit PATH' });
  evidence.unit.pathProbe = 'ABSENT';

  await runRouterPhase({
    phase: 'ts-workspace',
    implementation: 'ts',
    invocation: routerProcessInvocation(tsSpec),
    cwd: repoRoot,
    env: process.env,
    expectTsHealth: true,
    committedTuple: committed,
    httpPort,
    runtimePort,
  });

  const unitProcess = resolveRouterRollbackUnitProcess(
    unitManifest.process,
    unitRoot,
  );
  await runRouterPhase({
    phase: 'ts-unit',
    implementation: 'ts',
    invocation: unitProcess,
    cwd: join(unitRoot, 'router'),
    env: unitEnv,
    expectTsHealth: true,
    committedTuple: committed,
    httpPort,
    runtimePort,
  });

  await runRouterPhase({
    phase: 'rust',
    implementation: 'rust',
    invocation: routerProcessInvocation(rustSpec),
    cwd: repoRoot,
    env: process.env,
    expectTsHealth: false,
    committedTuple: committed,
    httpPort,
    runtimePort,
  });

  const relocatedProcess = resolveRouterRollbackUnitProcess(
    relocatedUnit.manifest.process,
    relocatedUnitRoot,
  );
  await runRouterPhase({
    phase: 'ts-unit-relocated',
    implementation: 'ts',
    invocation: relocatedProcess,
    cwd: join(relocatedUnitRoot, 'router'),
    env: unitEnv,
    expectTsHealth: true,
    committedTuple: committed,
    httpPort,
    runtimePort,
  });

  await stopRollbackRuntime();
  await closeRollbackRelay(relayPort);

  console.log('router-rollback-final: clean-host bundle rehearsal');
  evidence.cleanHost = await runCleanHostRehearsal({
    bundleRoot: join(tempRoot, 'clean-host-bundle'),
    devHome,
    runtimeBin,
    artifactRoot,
    httpPort,
    runtimePort,
    mongoUrl,
  });

  assertRollbackRoundtrip(evidence, committed);
  assertCleanHostEvidence(evidence);

  console.log('router-rollback-final: PASS');
  console.log(JSON.stringify(rollbackEvidence(evidence), null, 2));
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  process.stderr.write(`\nrouter-rollback-final evidence:\n${JSON.stringify(evidenceSummary(evidence), null, 2)}\n`);
  if (currentRelay !== undefined) {
    const tail = currentRelay.records.slice(-60).map((record) => ({
      direction: record.direction,
      type: record.type,
      header: record.header,
      kind: record.kind,
    }));
    process.stderr.write(`\nrouter-rollback-final relay tail:\n${JSON.stringify(tail, null, 2)}\n`);
  }
  throw error;
} finally {
  const errors = [];
  if (currentRuntime !== undefined) {
    try {
      const exit = await stopProcess(currentRuntime.child, 'SIGINT', {
        label: 'rollback final runtime (cleanup)',
      });
      evidence.runtimeExit = exit;
    } catch (error) {
      errors.push(error);
    }
  }
  if (currentRelay !== undefined) {
    try {
      await currentRelay.close();
    } catch (error) {
      errors.push(error);
    }
  }
  if (mongoHarness !== undefined) {
    try {
      await mongoHarness.cleanup();
    } catch (error) {
      errors.push(error);
    }
  }
  if (portLease !== undefined) {
    try {
      await portLease.release();
    } catch (error) {
      errors.push(error);
    }
  }
  if (tempRoot !== undefined) {
    try {
      await rm(tempRoot, { recursive: true, force: true });
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'router-rollback-final cleanup failed');
  }
}

async function runRouterPhase({
  phase,
  implementation,
  invocation,
  cwd,
  env,
  expectTsHealth,
  committedTuple,
  httpPort,
  runtimePort,
}) {
  const logsDir = join(tempRoot, `phase-${phase}`);
  await mkdir(logsDir, { recursive: true });
  const stdoutPath = join(logsDir, 'router.stdout.log');
  const stderrPath = join(logsDir, 'router.stderr.log');
  evidence.admission.push({
    phase,
    action: 'stop-admission',
    at: new Date().toISOString(),
  });
  const router = await spawnLoggedProcessEnv(invocation.command, invocation.args, {
    cwd,
    stdoutPath,
    stderrPath,
    env,
  });
  let suite;
  let phaseError;
  try {
    await waitForListeners({
      httpPort,
      runtimePort,
      child: router.child,
      stderrPath,
    });
    const fromIndex = currentRelay.records.length;
    await waitForHandshakeAfter(currentRelay, fromIndex);
    const tuple = latestBootstrapTupleAfter(currentRelay.records, fromIndex);
    if (tuple === null) {
      throw new Error(`${phase} phase observed no router.bootstrap frame`);
    }
    assertTuple(tuple, committedTuple, phase);
    if (expectTsHealth) {
      await assertTsReadiness({
        port: runtimePort,
        tuple: committedTuple,
        replicaId: HTTP_LIVE_REPLICA_ID,
        phase,
      });
    } else {
      await assertRustHealth({ port: runtimePort, phase });
    }
    evidence.admission.push({
      phase,
      action: 'open-admission',
      at: new Date().toISOString(),
    });
    suite = await runRollbackSuite({
      port: httpPort,
      serviceId: HTTP_LIVE_SERVICE_ID,
      version: HTTP_LIVE_VERSION,
      relay: currentRelay,
      phase,
    });
    evidence.phases.push({
      phase,
      implementation,
      tuple,
      cases: suite,
      process: { command: invocation.command, args: [...invocation.args] },
    });
    console.log(`router-rollback-final: ${phase} phase passed (${suite.length} cases)`);
  } catch (error) {
    phaseError = error;
    evidence.relayTails.push({
      phase,
      records: currentRelay.records.map((record) => ({
        direction: record.direction,
        type: record.type,
        header: record.header,
        kind: record.kind,
      })),
    });
  } finally {
    const stopErrors = [];
    try {
      const exit = await stopProcess(router.child, 'SIGTERM', {
        label: `${phase} router`,
      });
      evidence.phases.push({ phase, implementation, exit });
      assertRouterExit(`${phase} router`, exit);
    } catch (error) {
      stopErrors.push(error);
    }
    try {
      await closeLogs(router);
      await assertRouterPortsClosed([httpPort, runtimePort]);
    } catch (error) {
      stopErrors.push(error);
    }
    if (phaseError === undefined && stopErrors.length > 0) {
      phaseError = stopErrors.length === 1 ? stopErrors[0] : new AggregateError(
        stopErrors,
        `${phase} phase stop failed`,
      );
    }
  }
  if (phaseError !== undefined) {
    throw phaseError;
  }
  return suite;
}

async function runCleanHostRehearsal({
  bundleRoot,
  devHome,
  runtimeBin,
  artifactRoot,
  httpPort,
  runtimePort,
  mongoUrl,
}) {
  const bundleArtifactsPath = join(bundleRoot, 'artifacts');
  const routerConfigText = renderHttpLiveRouterConfig({
    environment: HTTP_LIVE_ENVIRONMENT,
    artifactsPath: bundleArtifactsPath,
    httpPort,
    runtimePort,
    mongoUrl,
  });
  const cleanHostRuntimeHome = join(tempRoot, 'clean-host-runtime-home');
  await mkdir(cleanHostRuntimeHome, { recursive: true });
  await writeFile(
    join(cleanHostRuntimeHome, 'runtime-id'),
    `${HTTP_LIVE_REPLICA_ID}\n`,
  );
  const runtimeConfigText = renderRuntimeConfig({
    routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
    runtimeHome: cleanHostRuntimeHome,
    environment: HTTP_LIVE_ENVIRONMENT,
  });
  const bundle = await buildCleanHostBundle({
    bundleRoot,
    routerBinary: join(devHome, 'bin', 'skiff-router'),
    runtimeBinary: runtimeBin,
    routerConfigText,
    runtimeConfigText,
    artifactRoot,
  });

  const envHome = join(tempRoot, 'clean-host-empty-home');
  await mkdir(envHome, { recursive: true });
  const env = cleanHostEnv(process.env, { home: envHome });
  await assertNoPnpmOrTsxOnPath({ env, label: 'clean-host PATH' });

  const logsDir = join(tempRoot, 'clean-host-logs');
  await mkdir(logsDir, { recursive: true });
  const router = await spawnLoggedProcessEnv(
    '/bin/sh',
    [join(bundleRoot, 'scripts/start-router.sh')],
    {
      cwd: bundleRoot,
      stdoutPath: join(logsDir, 'router.stdout.log'),
      stderrPath: join(logsDir, 'router.stderr.log'),
      env,
    },
  );
  let runtime;
  let suite;
  let result;
  let rehearsalError;
  try {
    await waitForListeners({
      httpPort,
      runtimePort,
      child: router.child,
      stderrPath: join(logsDir, 'router.stderr.log'),
    });
    await assertRustHealth({ port: runtimePort, phase: 'clean-host' });
    runtime = await spawnLoggedProcessEnv(
      '/bin/sh',
      [join(bundleRoot, 'scripts/start-runtime.sh')],
      {
        cwd: bundleRoot,
        stdoutPath: join(logsDir, 'runtime.stdout.log'),
        stderrPath: join(logsDir, 'runtime.stderr.log'),
        env,
      },
    );
    suite = await runCleanHostHttpSuite({
      port: httpPort,
      phase: 'clean-host',
    });
    result = {
      bundleRoot,
      manifestPath: bundle.manifestPath,
      platform: bundle.manifest.platform,
      fileCount: bundle.manifest.file_count,
      sha256Tree: bundle.manifest.sha256_tree,
      pathProbe: 'ABSENT',
      suite,
    };
    return result;
  } catch (error) {
    rehearsalError = error;
    throw error;
  } finally {
    const stopErrors = [];
    const stop = async (child, signal, label) => {
      try {
        const exit = await stopProcess(child, signal, { label });
        assertRouterExit(label, exit);
        return exit;
      } catch (error) {
        stopErrors.push(error);
        return null;
      }
    };
    const routerExit = await stop(router.child, 'SIGTERM', 'clean-host router');
    const runtimeExit = runtime === undefined
      ? null
      : await stop(runtime.child, 'SIGINT', 'clean-host runtime');
    if (result !== undefined) {
      result.exits = { router: routerExit, runtime: runtimeExit };
    }
    try {
      await closeLogs(router);
      if (runtime !== undefined) {
        await closeLogs(runtime);
      }
    } catch (error) {
      stopErrors.push(error);
    }
    try {
      await assertRouterPortsClosed([httpPort, runtimePort]);
      await assertCleanHostBundle(bundleRoot);
    } catch (error) {
      stopErrors.push(error);
    }
    if (rehearsalError === undefined && stopErrors.length > 0) {
      throw stopErrors.length === 1 ? stopErrors[0] : new AggregateError(
        stopErrors,
        'clean-host rehearsal stop failed',
      );
    }
  }
}

async function assertTsReadiness({ port, tuple, replicaId, phase }) {
  const response = await fetch(`http://127.0.0.1:${port}/__router/health`, {
    signal: AbortSignal.timeout(10_000),
  });
  assertEqual(response.status, 200, `${phase} health status`);
  const body = await response.json();
  const active = body?.activeAssembly;
  assertEqual(active?.environment, tuple.environment, `${phase} health environment`);
  assertEqual(String(active?.generation), String(tuple.generation), `${phase} health generation`);
  assertEqual(
    active?.assemblyIdentity,
    tuple.assemblyIdentity,
    `${phase} health assembly identity`,
  );
  assertEqual(
    active?.configSnapshotId,
    tuple.configSnapshotId,
    `${phase} health config snapshot id`,
  );
  assertEqual(body?.pendingActivation, null, `${phase} health pending activation`);
  const replicas = Array.isArray(body?.replicas) ? body.replicas : [];
  assert(
    replicas.some((replica) => replica?.replicaId === replicaId),
    `${phase} health replicas must include ${replicaId}`,
  );
  return body;
}

async function assertRustHealth({ port, phase }) {
  // Recorded differential boundary: the Rust runtime/control listener still
  // serves `/__router/health` as an empty 200 placeholder. Readiness for Rust
  // phases comes from the relay bootstrap tuple plus the unary suite.
  const response = await fetch(`http://127.0.0.1:${port}/__router/health`, {
    signal: AbortSignal.timeout(10_000),
  });
  assertEqual(response.status, 200, `${phase} health status`);
  return response;
}

async function stopRollbackRuntime() {
  if (currentRuntime === undefined) {
    return;
  }
  const exit = await stopProcess(currentRuntime.child, 'SIGINT', {
    label: 'rollback final runtime',
  });
  evidence.runtimeExits.push({ phase: 'rollback-phases', exit });
  assertRouterExit('rollback final runtime', exit);
  await closeLogs(currentRuntime);
  currentRuntime = undefined;
}

async function closeRollbackRelay(relayPort) {
  if (currentRelay === undefined) {
    return;
  }
  await currentRelay.close();
  currentRelay = undefined;
  await assertRouterPortsClosed([relayPort]);
}

function assertRollbackRoundtrip(evidence, committed) {
  const expectedPhases = ['ts-workspace', 'ts-unit', 'rust', 'ts-unit-relocated'];
  const suiteEntries = evidence.phases.filter((entry) => entry.tuple !== undefined);
  if (suiteEntries.map((entry) => entry.phase).join(',') !== expectedPhases.join(',')) {
    throw new Error(
      `rollback rehearsal requires phases ${expectedPhases.join(', ')}`,
    );
  }
  for (const entry of suiteEntries) {
    assertTuple(entry.tuple, committed, entry.phase);
    const names = entry.cases.map((entryCase) => entryCase.name);
    for (const name of ['unary-happy', 'typed-unary', 'stream-roundtrip']) {
      assert(
        names.includes(name),
        `${entry.phase} must include ${name}, got ${names.join(', ')}`,
      );
    }
  }
  const admissionPhases = expectedPhases.map((phase) => [
    evidence.admission.find((entry) => entry.phase === phase && entry.action === 'stop-admission'),
    evidence.admission.find((entry) => entry.phase === phase && entry.action === 'open-admission'),
  ]);
  for (const [stop, open] of admissionPhases) {
    assert(stop !== undefined && open !== undefined, 'admission gate sequence missing for a phase');
    assert(
      Date.parse(stop.at) <= Date.parse(open.at),
      'admission must open after it was stopped',
    );
  }
}

function assertCleanHostEvidence(evidence) {
  const cleanHost = evidence.cleanHost;
  if (!cleanHost || cleanHost.pathProbe !== 'ABSENT' || !Array.isArray(cleanHost.suite)) {
    throw new Error('clean-host rehearsal evidence is incomplete');
  }
  const names = cleanHost.suite.map((entry) => entry.name);
  for (const name of ['unary-happy', 'typed-unary', 'missing-selector', 'wrong-path', 'stream-roundtrip']) {
    assert(names.includes(name), `clean-host suite must include ${name}`);
  }
}

function assertTuple(tuple, committed, phase) {
  assertEqual(tuple.environment, committed.environment, `${phase} bootstrap environment`);
  assertEqual(String(tuple.generation), String(committed.generation), `${phase} bootstrap generation`);
  assertEqual(tuple.assemblyIdentity, committed.assemblyIdentity, `${phase} bootstrap assembly identity`);
  assertEqual(tuple.configSnapshotId, committed.configSnapshotId, `${phase} bootstrap config snapshot id`);
}

async function spawnLoggedProcessEnv(command, args, {
  cwd,
  stdoutPath,
  stderrPath,
  env,
}) {
  const stdoutLog = await open(stdoutPath, 'w');
  const stderrLog = await open(stderrPath, 'w');
  const child = spawn(command, args, {
    cwd,
    stdio: ['ignore', stdoutLog.fd, stderrLog.fd],
    env: env ?? process.env,
  });
  return { child, stdoutLog, stderrLog, command, args };
}

function rollbackEvidence(evidence) {
  return {
    unit: evidence.unit,
    manifests: evidence.manifests,
    switchPlan: evidence.switchPlan,
    admission: evidence.admission,
    phases: evidence.phases.map((entry) => ({
      phase: entry.phase,
      implementation: entry.implementation,
      tuple: entry.tuple,
      cases: entry.cases,
      process: entry.process,
      exit: entry.exit,
    })),
    runtimeExits: evidence.runtimeExits,
    cleanHost: evidence.cleanHost,
  };
}

function evidenceSummary(evidence) {
  return {
    unit: evidence.unit,
    manifests: evidence.manifests,
    admission: evidence.admission,
    phases: evidence.phases,
    runtimeExits: evidence.runtimeExits,
    cleanHost: evidence.cleanHost,
    relayTails: evidence.relayTails,
  };
}

function assertEqual(actual, expected, label) {
  if (!Object.is(actual, expected)) {
    throw new Error(
      `${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertNotForbidden(port) {
  if (FORBIDDEN_PORTS.has(port)) {
    throw new Error(`leased port ${port} is a forbidden stable port`);
  }
}

function range(start, end) {
  const values = [];
  for (let value = start; value <= end; value += 1) {
    values.push(value);
  }
  return values;
}
