#!/usr/bin/env node
// `router-live:http` managed harness (E-http gate, plan §7/§8, post-cutover
// Rust-only).
//
// Real HTTP → Router → Runtime unary + stream through the Rust `skiff-router`
// binary phases: the same devHome/router.yml and the same release pointer
// table, a test-only WS relay that records every frame, and the full E-http
// surface: trusted selectors, service-scoped ingress, typed/raw opaque
// payloads, unary/stream mapping and sequencing, cumulative response ceiling,
// backpressure, disconnect/cancel/deadline, CORS preflight/service-managed
// and platform errors. Every race asserts one external terminal, at most one
// cancel frame per request and a successful follow-up unary; the
// process-level residue gate asserts Router SIGTERM exit 0 with closed
// listeners and Runtime SIGINT exit 0.
//
// The harness never touches the stable instance, stable Mongo, PM2 or the
// fixed 4004-4007 ports: it uses a temporary Mongo replica set and leased
// ports in 45000-45999.

import { access, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { MongodLiveHarness } from './lib/mongod-live-harness.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
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
  createHttpLiveRouterSpecs,
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
import {
  runBasicSuite,
  runBackpressureSuite,
  runFullSuite,
} from './lib/http_live_suite.mjs';
import { createRollbackRelay } from './lib/rollback-relay.mjs';

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
  bootstrapTuples: [],
  phases: [],
  runtimeExits: [],
  suite: [],
  relayTails: [],
};

try {
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-http-live-'));
  const sourceRoot = join(tempRoot, 'src');
  console.log('router-live:http: writing real HTTP service source');
  // The burst sleep is platform-adaptive: on Linux CI the request must stay
  // active past the router's 10s drain deadline so the `backpressure` cancel
  // wins; on OS-absorption hosts (macOS) a short burst keeps the session
  // under the 64-frame inbound budget while the boundary is recorded.
  const burstSleepMs = process.platform === 'linux' ? 20_000 : 2_000;
  await writeHttpLiveServiceSource(sourceRoot, { burstSleepMs });

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });
  console.log('router-live:http: compiling real package/assembly/config artifact');
  const identities = await authorHttpLiveArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    profile: HTTP_LIVE_PROFILE,
  });

  console.log('router-live:http: leasing isolated router + relay ports');
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

  console.log('router-live:http: starting isolated Mongo replica set');
  mongoHarness = await MongodLiveHarness.create({ repoRoot });
  await mongoHarness.start();

  const mongoUrl = httpLiveMongoUrl(mongoHarness.port);
  console.log('router-live:http: seeding the release pointer table');
  await seedHttpLiveReleasePointers({
    artifactRoot,
    profile: HTTP_LIVE_PROFILE,
    deployment: identities.deploymentRef,
  });

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:http: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const routerSourceBinary = join(targetDir, 'debug', 'skiff-router');
  await access(routerSourceBinary);

  console.log('router-live:http: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await access(runtimeBin);

  const devHome = join(tempRoot, 'dev-home');
  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });
  await writeFile(join(runtimeHome, 'runtime-id'), `${HTTP_LIVE_REPLICA_ID}\n`);

  const routerConfigPath = join(devHome, 'router.yml');
  await writeHttpLiveRouterConfig(
    routerConfigPath,
    renderHttpLiveRouterConfig({
      profile: HTTP_LIVE_PROFILE,
      artifactsPath: artifactRoot,
      httpPort,
      runtimePort,
      mongoUrl,
    }),
  );

  const rustSpec = createHttpLiveRouterSpecs({ repoRoot, devHome }).rust;

  const runtimeConfigPath = join(tempRoot, 'runtime.yml');
  await writeHttpLiveRuntimeConfig(runtimeConfigPath, {
    relayPort,
    runtimeHome,
    profile: HTTP_LIVE_PROFILE,
  });

  // Each Router phase owns a fresh relay + Runtime process pair: when the
  // Router exits, the relay's downstream socket would otherwise stay open
  // forever (the relay only detaches, it does not close the peer), so the
  // Runtime is stopped inside the phase before the relay closes. The Runtime
  // reuses the same runtime-home/replica id and the same release pointer
  // table on every phase.
  await installHttpLiveRustBinary({ sourceBinary: routerSourceBinary, devHome });
  await runRouterPhase({
    phase: 'rust',
    invocation: rustSpec.invocation,
    httpPort,
    runtimePort,
    relayPort,
    full: true,
    routerLogsDir: join(tempRoot, 'phase-rust'),
    runtimeBin,
    runtimeConfigPath,
  });

  // Backpressure is untriggerable under the strict 4096-byte response
  // ceiling used by the ceiling cases (the Runtime rejects the burst before
  // the HTTP writer can stall), so it runs in its own Rust-only phase with a
  // 16 MiB ceiling over the same artifact and release pointer table.
  const backpressureConfigPath = join(devHome, 'router-backpressure.yml');
  await writeHttpLiveRouterConfig(
    backpressureConfigPath,
    renderHttpLiveRouterConfig({
      profile: HTTP_LIVE_PROFILE,
      artifactsPath: artifactRoot,
      httpPort,
      runtimePort,
      mongoUrl,
      httpMaxResponseBytes: 16 * 1024 * 1024,
      requestTimeoutMs: 30_000,
    }),
  );
  await runRouterPhase({
    phase: 'rust-bp',
    invocation: {
      command: rustSpec.spec.rust_binary_path,
      args: [backpressureConfigPath],
    },
    httpPort,
    runtimePort,
    relayPort,
    suite: 'backpressure',
    full: false,
    routerLogsDir: join(tempRoot, 'phase-rust-bp'),
    runtimeBin,
    runtimeConfigPath,
  });

  console.log('router-live:http: PASS');
  console.log(JSON.stringify(httpLiveEvidence(evidence), null, 2));
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  process.stderr.write(`\nrouter-live:http evidence:\n${JSON.stringify(evidenceSummary(evidence), null, 2)}\n`);
  if (currentRelay !== undefined) {
    const tail = currentRelay.records.slice(-60).map((record) => ({
      direction: record.direction,
      type: record.type,
      header: record.header,
      kind: record.kind,
    }));
    process.stderr.write(`\nrouter-live:http relay tail:\n${JSON.stringify(tail, null, 2)}\n`);
  }
  throw error;
} finally {
  const errors = [];
  if (currentRuntime !== undefined) {
    try {
      const exit = await stopProcess(currentRuntime.child, 'SIGINT', {
        label: 'http live runtime (cleanup)',
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
    throw new AggregateError(errors, 'router-live:http cleanup failed');
  }
}

async function runRouterPhase({
  phase,
  invocation,
  httpPort,
  runtimePort,
  relayPort,
  full,
  suite,
  routerLogsDir,
  runtimeBin,
  runtimeConfigPath,
}) {
  await mkdir(routerLogsDir, { recursive: true });
  const stderrPath = join(routerLogsDir, 'router.stderr.log');
  const router = await spawnLoggedProcess(invocation.command, invocation.args, {
    cwd: repoRoot,
    stdoutPath: join(routerLogsDir, 'router.stdout.log'),
    stderrPath,
  });
  let relay;
  let runtime;
  let suiteResult;
  let phaseError;
  try {
    await waitForListeners({
      httpPort,
      runtimePort,
      child: router.child,
      stderrPath,
    });
    relay = await createRollbackRelay({
      port: relayPort,
      routerUrl: `ws://127.0.0.1:${runtimePort}/runtime`,
    });
    currentRelay = relay;
    runtime = await spawnLoggedProcess(runtimeBin, [runtimeConfigPath], {
      cwd: repoRoot,
      stdoutPath: join(routerLogsDir, 'runtime.stdout.log'),
      stderrPath: join(routerLogsDir, 'runtime.stderr.log'),
    });
    currentRuntime = runtime;
    await waitForHandshakeAfter(relay, 0);
    const tuple = latestBootstrapTupleAfter(relay.records, 0);
    if (tuple === null) {
      throw new Error(`${phase} phase observed no router.bootstrap frame`);
    }
    evidence.bootstrapTuples.push({ phase, tuple });
    const ctx = {
      port: httpPort,
      serviceId: HTTP_LIVE_SERVICE_ID,
      version: HTTP_LIVE_VERSION,
      relay,
      phase,
    };
    if (suite === 'backpressure') {
      suiteResult = await runBackpressureSuite(ctx);
    } else {
      suiteResult = full ? await runFullSuite(ctx) : await runBasicSuite(ctx);
    }
    evidence.suite.push({ phase, cases: suiteResult });
    console.log(`router-live:http: ${phase} phase passed (${suiteResult.length} cases)`);
  } catch (error) {
    phaseError = error;
    if (relay !== undefined) {
      evidence.relayTails.push({
        phase,
        records: relay.records.map((record) => ({
          direction: record.direction,
          type: record.type,
          header: record.header,
          kind: record.kind,
        })),
      });
    }
  } finally {
    const stopErrors = [];
    try {
      const exit = await stopProcess(router.child, 'SIGTERM', {
        label: `${phase} router`,
      });
      evidence.phases.push({ phase, implementation: 'rust', exit });
      assertRouterExit(`${phase} router`, exit);
    } catch (error) {
      stopErrors.push(error);
    }
    // The Runtime must be stopped before the relay closes: the relay never
    // closes a detached downstream socket, so `server.close` would otherwise
    // wait forever for the Runtime's open WebSocket connection.
    try {
      if (runtime !== undefined) {
        const runtimeExit = await stopProcess(runtime.child, 'SIGINT', {
          label: `${phase} runtime`,
        });
        evidence.runtimeExits.push({ phase, exit: runtimeExit });
        assertRouterExit(`${phase} runtime`, runtimeExit);
        await closeLogs(runtime);
        currentRuntime = undefined;
      }
    } catch (error) {
      stopErrors.push(error);
    }
    try {
      await closeLogs(router);
      if (relay !== undefined) {
        await relay.close();
        currentRelay = undefined;
      }
      await assertRouterPortsClosed([httpPort, runtimePort, relayPort]);
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
  return suiteResult;
}

function httpLiveEvidence(evidence) {
  return {
    bootstrapTuples: evidence.bootstrapTuples.map(({ phase, tuple }) => ({ phase, tuple })),
    suite: evidence.suite,
    processExits: evidence.phases.map(({ phase, implementation, exit }) => ({
      phase,
      implementation,
      exit,
    })),
    runtimeExit: evidence.runtimeExit,
  };
}

function evidenceSummary(evidence) {
  return {
    bootstrapTuples: evidence.bootstrapTuples,
    phases: evidence.phases,
    suite: evidence.suite,
    runtimeExit: evidence.runtimeExit,
    relayTails: evidence.relayTails,
  };
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
