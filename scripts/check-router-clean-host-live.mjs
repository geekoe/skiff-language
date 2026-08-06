#!/usr/bin/env node
// Router Rust migration Batch 12 — Rust-only clean-host release rehearsal
// (plan §8 `router-live:clean-host` / §11.1 binary lifecycle).
//
// The TS rollback unit rehearsal was retired by the batch 11 cutover-delete
// node (plan §11.3: rollback is the previous complete release; the TS unit
// can no longer be rebuilt from the workspace). What remains is the
// Rust-only clean-host rehearsal: a deployment bundle that contains only
// router + runtime binaries, YAML configs, the compiler artifact root and
// POSIX `sh` start scripts, started with a PATH that cannot resolve
// pnpm/tsx, then exercised through start/health/Runtime reconnect/unary
// suite/shutdown with the bundle identity unchanged before and after.
//
// Modes:
//   default          full clean-host rehearsal in a fresh temp directory
//                    (temporary Mongo replica set, real compiler artifact,
//                    explicit Rust router/runtime debug binaries, leased
//                    ports in 45000-45999; never touches the stable
//                    instance, stable Mongo, PM2 or 4004-4007).
//   --preflight      verify the checker and its required libraries/tools
//                    exist without running the rehearsal.
//   --loop-risk-config <abs> --loop-risk-stop-file <abs>
//                    run the rehearsal, then hold a second instance from
//                    the same bundle and write the canonical loop-risk
//                    config JSON (healthUrl/runtimeIds/stress) so the
//                    release workflow can run check-loop-risk-health.mjs /
//                    check-loop-risk-stress-live.mjs against it; tear the
//                    instance down when the stop file appears.
//
// Platform contract: the real gate runs on a Linux GitHub runner. A local
// macOS run is a platform-equivalent dry run (the output records
// `platform: darwin`) and must never be reported as the Linux gate.

import { access, mkdir, mkdtemp, open, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { MongodLiveHarness } from './lib/mongod-live-harness.mjs';
import { cargoBuildEnv, cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import {
  assertCleanHostBundle,
  assertNoPnpmOrTsxOnPath,
  buildCleanHostBundle,
  cleanHostEnv,
} from './lib/clean-host-bundle.mjs';
import {
  HTTP_LIVE_PROFILE,
  HTTP_LIVE_REPLICA_ID,
  HTTP_LIVE_SERVICE_ID,
  HTTP_LIVE_VERSION,
  authorHttpLiveArtifact,
  httpLiveMongoUrl,
  seedHttpLiveReleasePointers,
  writeHttpLiveServiceSource,
} from './lib/http_live_fixture.mjs';
import {
  assertRouterExit,
  assertRouterPortsClosed,
  closeLogs,
  installHttpLiveRustBinary,
  renderHttpLiveRouterConfig,
  spawnLoggedProcess,
  stopProcess,
  waitForListeners,
} from './lib/http_live_process.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
import {
  runCleanHostHttpSuite,
  waitForCleanHostUnary,
} from './lib/rollback-clean-host-suite.mjs';
import { renderRuntimeConfig } from './lib/runtime-stack-config.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);
const LOOP_RISK_HOLD_TIMEOUT_MS = 3_600_000;
const PREFLIGHT_FILES = [
  'scripts/check-router-clean-host-live.mjs',
  'scripts/lib/clean-host-bundle.mjs',
  'scripts/lib/rollback-clean-host-suite.mjs',
  'scripts/lib/http_live_fixture.mjs',
  'scripts/lib/http_live_process.mjs',
  'scripts/lib/mongod-live-harness.mjs',
];

let mongoHarness;
let portLease;
let tempRoot;

try {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    printUsage();
    process.exit(0);
  }
  if (args.preflight) {
    await runPreflight();
    process.exit(0);
  }
  if ((args.loopRiskConfig === undefined) !== (args.loopRiskStopFile === undefined)) {
    throw new Error(
      '--loop-risk-config and --loop-risk-stop-file must be provided together',
    );
  }
  if (args.loopRiskConfig !== undefined) {
    if (!isAbsolute(args.loopRiskConfig)) {
      throw new Error('--loop-risk-config must be an absolute path');
    }
    if (!isAbsolute(args.loopRiskStopFile)) {
      throw new Error('--loop-risk-stop-file must be an absolute path');
    }
  }

  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-clean-host-'));
  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('check-router-clean-host-live: writing real HTTP service source');
  const sourceRoot = join(tempRoot, 'src');
  await writeHttpLiveServiceSource(sourceRoot, { burstSleepMs: 2_000 });

  console.log(
    'check-router-clean-host-live: compiling real package/assembly/config artifact',
  );
  const identities = await authorHttpLiveArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    profile: HTTP_LIVE_PROFILE,
  });

  console.log('check-router-clean-host-live: leasing isolated router ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 2,
  });
  portLease = { release };
  const [httpPort, runtimePort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log(
    'check-router-clean-host-live: starting isolated Mongo replica set',
  );
  mongoHarness = await MongodLiveHarness.create({ repoRoot });
  await mongoHarness.start();
  const mongoUrl = httpLiveMongoUrl(mongoHarness.port);

  console.log(
    'check-router-clean-host-live: seeding the release pointer table',
  );
  await seedHttpLiveReleasePointers({
    artifactRoot,
    profile: HTTP_LIVE_PROFILE,
    deployment: identities.deploymentRef,
  });

  const targetDir = cargoTargetDir(repoRoot);
  console.log(
    'check-router-clean-host-live: building explicit Rust router binary',
  );
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: cargoBuildEnv(repoRoot) },
  );
  const routerSourceBinary = join(targetDir, 'debug', 'skiff-router');
  await access(routerSourceBinary);

  console.log(
    'check-router-clean-host-live: building explicit Rust runtime binary',
  );
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: cargoBuildEnv(repoRoot) },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const devHome = join(tempRoot, 'dev-home');
  await installHttpLiveRustBinary({
    sourceBinary: routerSourceBinary,
    devHome,
  });

  const bundleRoot = join(tempRoot, 'clean-host-bundle');
  const cleanHostRuntimeHome = join(tempRoot, 'clean-host-runtime-home');
  await mkdir(cleanHostRuntimeHome, { recursive: true });
  await writeFile(
    join(cleanHostRuntimeHome, 'runtime-id'),
    `${HTTP_LIVE_REPLICA_ID}\n`,
  );
  const routerConfigText = renderHttpLiveRouterConfig({
    profile: HTTP_LIVE_PROFILE,
    artifactsPath: join(bundleRoot, 'artifacts'),
    httpPort,
    runtimePort,
    mongoUrl,
  });
  const runtimeConfigText = renderRuntimeConfig({
    routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
    runtimeHome: cleanHostRuntimeHome,
  });

  console.log(
    'check-router-clean-host-live: preparing clean-host deployment bundle',
  );
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
  const sanitizedEnv = cleanHostEnv(process.env, { home: envHome });
  await assertNoPnpmOrTsxOnPath({
    env: sanitizedEnv,
    label: 'clean-host PATH',
  });

  const logsDir = join(tempRoot, 'clean-host-logs');
  await mkdir(logsDir, { recursive: true });

  console.log('check-router-clean-host-live: clean-host bundle rehearsal');
  const rehearsal = await withProcessEnv(sanitizedEnv, () =>
    runCleanHostRehearsal({
      bundleRoot,
      httpPort,
      runtimePort,
      logsDir,
      manifestPath: bundle.manifestPath,
      bundleManifest: bundle.manifest,
    }));
  assertCleanHostRehearsal(rehearsal);

  let loopRiskTarget;
  if (args.loopRiskConfig !== undefined) {
    console.log(
      'check-router-clean-host-live: holding clean-host target for loop-risk',
    );
    loopRiskTarget = await withProcessEnv(sanitizedEnv, () =>
      runHeldLoopRiskTarget({
        bundleRoot,
        httpPort,
      runtimePort,
      logsDir,
      configPath: args.loopRiskConfig,
      stopFilePath: args.loopRiskStopFile,
      manifestPath: bundle.manifestPath,
      bundleManifest: bundle.manifest,
      }));
  }

  console.log('check-router-clean-host-live: PASS');
  console.log(JSON.stringify({
    ok: true,
    checker: 'router-live:clean-host',
    platform: process.platform,
    rehearsal,
    loopRiskTarget,
  }, null, 2));
} catch (error) {
  console.error(
    error instanceof Error ? error.message : String(error),
  );
  if (error?.stderr) {
    console.error(error.stderr);
  }
  process.exitCode = 1;
} finally {
  if (mongoHarness !== undefined) {
    try {
      await mongoHarness.cleanup();
    } catch (error) {
      console.error(
        `check-router-clean-host-live: mongo cleanup failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
  if (portLease !== undefined) {
    try {
      await portLease.release();
    } catch (error) {
      console.error(
        `check-router-clean-host-live: port lease release failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
  if (tempRoot !== undefined) {
    try {
      await rm(tempRoot, { recursive: true, force: true });
    } catch (error) {
      console.error(
        `check-router-clean-host-live: temp root cleanup failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
}

async function runCleanHostRehearsal({
  bundleRoot,
  httpPort,
  runtimePort,
  logsDir,
  manifestPath,
  bundleManifest,
}) {
  const router = await spawnLoggedProcess(
    '/bin/sh',
    [join(bundleRoot, 'scripts/start-router.sh')],
    {
      cwd: bundleRoot,
      stdoutPath: join(logsDir, 'router.stdout.log'),
      stderrPath: join(logsDir, 'router.stderr.log'),
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
    runtime = await spawnLoggedProcess(
      '/bin/sh',
      [join(bundleRoot, 'scripts/start-runtime.sh')],
      {
        cwd: bundleRoot,
        stdoutPath: join(logsDir, 'runtime.stdout.log'),
        stderrPath: join(logsDir, 'runtime.stderr.log'),
      },
    );
    suite = await runCleanHostHttpSuite({
      port: httpPort,
      phase: 'clean-host',
    });
    result = {
      manifestPath,
      fileCount: bundleManifest.file_count,
      sha256Tree: bundleManifest.sha256_tree,
      pathProbe: 'ABSENT',
      suite,
      exits: null,
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

async function runHeldLoopRiskTarget({
  bundleRoot,
  httpPort,
  runtimePort,
  logsDir,
  configPath,
  stopFilePath,
  manifestPath,
  bundleManifest,
}) {
  const router = await spawnLoggedProcess(
    '/bin/sh',
    [join(bundleRoot, 'scripts/start-router.sh')],
    {
      cwd: bundleRoot,
      stdoutPath: join(logsDir, 'hold-router.stdout.log'),
      stderrPath: join(logsDir, 'hold-router.stderr.log'),
    },
  );
  let runtime;
  let result;
  let holdError;
  try {
    await waitForListeners({
      httpPort,
      runtimePort,
      child: router.child,
      stderrPath: join(logsDir, 'hold-router.stderr.log'),
    });
    await assertRustHealth({ port: runtimePort, phase: 'loop-risk-target' });
    runtime = await spawnLoggedProcess(
      '/bin/sh',
      [join(bundleRoot, 'scripts/start-runtime.sh')],
      {
        cwd: bundleRoot,
        stdoutPath: join(logsDir, 'hold-runtime.stdout.log'),
        stderrPath: join(logsDir, 'hold-runtime.stderr.log'),
      },
    );
    await waitForCleanHostUnary({
      port: httpPort,
      serviceId: HTTP_LIVE_SERVICE_ID,
      version: HTTP_LIVE_VERSION,
      phase: 'loop-risk-target',
      timeoutMs: 90_000,
    });
    const runtimeLog = resolve(join(logsDir, 'hold-runtime.stderr.log'));
    const config = {
      healthUrl: `http://127.0.0.1:${runtimePort}/__router/health?detail=loop-risk`,
      runtimeIds: [HTTP_LIVE_REPLICA_ID],
      stress: {
        wsUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
        runtimePids: [runtime.child.pid],
        runtimeLogs: [runtimeLog],
      },
    };
    await writeExclusive(configPath, `${JSON.stringify(config, null, 2)}\n`);
    console.log(
      `check-router-clean-host-live: loop-risk target ready: ${configPath}`,
    );
    await waitForLoopRiskStopFile({
      stopFilePath,
      router,
      runtime,
      timeoutMs: LOOP_RISK_HOLD_TIMEOUT_MS,
    });
    result = {
      configPath,
      healthUrl: config.healthUrl,
      wsUrl: config.stress.wsUrl,
      runtimePids: config.stress.runtimePids,
      runtimeLogs: config.stress.runtimeLogs,
      exits: null,
    };
    return result;
  } catch (error) {
    holdError = error;
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
    const routerExit = await stop(router.child, 'SIGTERM', 'loop-risk target router');
    const runtimeExit = runtime === undefined
      ? null
      : await stop(runtime.child, 'SIGINT', 'loop-risk target runtime');
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
    if (holdError === undefined && stopErrors.length > 0) {
      throw stopErrors.length === 1 ? stopErrors[0] : new AggregateError(
        stopErrors,
        'loop-risk target stop failed',
      );
    }
  }
}

async function waitForLoopRiskStopFile({
  stopFilePath,
  router,
  runtime,
  timeoutMs,
}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (router.child.exitCode !== null || router.child.signalCode !== null) {
      throw new Error(
        `loop-risk target router exited before stop file appeared (${router.child.signalCode ?? router.child.exitCode})`,
      );
    }
    if (runtime.child.exitCode !== null || runtime.child.signalCode !== null) {
      throw new Error(
        `loop-risk target runtime exited before stop file appeared (${runtime.child.signalCode ?? runtime.child.exitCode})`,
      );
    }
    try {
      await access(stopFilePath);
      return;
    } catch {
      // Stop file not present yet; keep holding the target.
    }
    await delay(500);
  }
  throw new Error(
    `loop-risk target stop file ${stopFilePath} did not appear within ${timeoutMs}ms`,
  );
}

async function assertRustHealth({ port, phase }) {
  const response = await fetch(`http://127.0.0.1:${port}/__router/health`, {
    signal: AbortSignal.timeout(10_000),
  });
  assertEqual(response.status, 200, `${phase} health status`);
  return response;
}

function assertCleanHostRehearsal(rehearsal) {
  if (
    !rehearsal
    || rehearsal.pathProbe !== 'ABSENT'
    || !Array.isArray(rehearsal.suite)
    || !rehearsal.exits
  ) {
    throw new Error('clean-host rehearsal evidence is incomplete');
  }
  const names = rehearsal.suite.map((entry) => entry.name);
  for (const name of [
    'unary-happy',
    'typed-unary',
    'missing-selector',
    'wrong-path',
    'stream-roundtrip',
  ]) {
    assert(names.includes(name), `clean-host suite must include ${name}`);
  }
}

async function runPreflight() {
  const missing = [];
  for (const file of PREFLIGHT_FILES) {
    try {
      await access(join(repoRoot, file));
    } catch {
      missing.push(file);
    }
  }
  if (missing.length > 0) {
    throw new Error(
      `router-live:clean-host preflight missing files: ${missing.join(', ')}`,
    );
  }
  const tools = {};
  for (const tool of ['node', 'cargo', 'mongod', 'mongosh']) {
    const outcome = await captureCheckedCommand(tool, ['--version'], {
      cwd: repoRoot,
    });
    tools[tool] = outcome.stdout.trim().split('\n')[0];
  }
  console.log(JSON.stringify({
    ok: true,
    checker: 'router-live:clean-host',
    platform: process.platform,
    tools,
  }, null, 2));
}

function withProcessEnv(env, fn) {
  const original = snapshotEnv();
  try {
    for (const key of Object.keys(process.env)) {
      delete process.env[key];
    }
    Object.assign(process.env, env);
    return fn();
  } finally {
    restoreEnv(original);
  }
}

function snapshotEnv() {
  const snapshot = {};
  for (const key of Object.keys(process.env)) {
    snapshot[key] = process.env[key];
  }
  return snapshot;
}

function restoreEnv(snapshot) {
  for (const key of Object.keys(process.env)) {
    delete process.env[key];
  }
  Object.assign(process.env, snapshot);
}

async function writeExclusive(path, text) {
  const handle = await open(path, 'wx', 0o644);
  try {
    await handle.writeFile(text, 'utf8');
    await handle.sync();
  } finally {
    await handle.close();
  }
}

function parseArgs(argv) {
  const args = {
    help: false,
    preflight: false,
    loopRiskConfig: undefined,
    loopRiskStopFile: undefined,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    switch (arg) {
      case '--help':
        args.help = true;
        break;
      case '--preflight':
        args.preflight = true;
        break;
      case '--loop-risk-config':
        args.loopRiskConfig = requireValue(argv, ++index, arg);
        break;
      case '--loop-risk-stop-file':
        args.loopRiskStopFile = requireValue(argv, ++index, arg);
        break;
      default:
        throw new Error(`unknown argument ${arg}`);
    }
  }
  return args;
}

function requireValue(argv, index, flag) {
  const value = argv[index];
  if (value === undefined) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function printUsage() {
  console.log(`usage: node scripts/check-router-clean-host-live.mjs [options]

Rust-only clean-host release rehearsal (router-live:clean-host).

options:
  --preflight
      Check that the checker, its required libraries and node/cargo/mongod/
      mongosh are present without running the rehearsal.
  --loop-risk-config <absolute-path>
  --loop-risk-stop-file <absolute-path>
      After the rehearsal, hold a second instance from the same bundle,
      write the canonical loop-risk config JSON to the config path, and
      tear the instance down when the stop file appears.
  --help
      Show this help.

The real gate runs on a Linux GitHub runner; a local macOS run is a
platform-equivalent dry run and must not be reported as the Linux gate.`);
}

function assertNotForbidden(port) {
  if (FORBIDDEN_PORTS.has(port)) {
    throw new Error(`refusing forbidden port ${port}`);
  }
}

function range(start, end) {
  const values = [];
  for (let value = start; value <= end; value += 1) {
    values.push(value);
  }
  return values;
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertEqual(actual, expected, label) {
  if (!Object.is(actual, expected)) {
    throw new Error(
      `${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}
