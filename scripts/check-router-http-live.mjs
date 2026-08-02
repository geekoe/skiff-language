#!/usr/bin/env node
// `router-live:http` managed harness (E-http gate, plan §7/§8/§11.2).
//
// Real HTTP → Router → Runtime unary + stream through three real Router
// process phases (TS → Rust → TS, §11.2 incremental rollback rehearsal):
// the same devHome/router.yml and the same committed activation tuple, a
// single real Runtime process kept alive through all phases, and a test-only
// WS relay that records every frame. The Rust phase additionally runs the
// full E-http surface: trusted selectors, service-scoped ingress, typed/raw
// opaque payloads, unary/stream mapping and sequencing, cumulative response
// ceiling, backpressure, disconnect/cancel/deadline, CORS preflight/
// service-managed and platform errors. Every race asserts one external
// terminal, at most one cancel frame per request and a successful follow-up
// unary; the process-level residue gate asserts Router SIGTERM exit 0 with
// closed listeners and Runtime SIGINT exit 0.
//
// The harness never touches the stable instance, stable Mongo, PM2 or the
// fixed 4004-4007 ports: it uses a temporary Mongo replica set and leased
// ports in 45000-45999.

import { access, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { captureCheckedCommand } from './lib/command-execution.mjs';
import { leaseConsecutiveLocalPorts } from './lib/local-port-lease.mjs';
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
  createHttpLiveRouterSpecs,
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
import { runFullSuite, runRollbackSuite } from './lib/http_live_suite.mjs';
import { createRuntimeRelay } from './lib/router-differential/relay.mjs';

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
  await writeHttpLiveServiceSource(sourceRoot);

  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });
  console.log('router-live:http: compiling real package/assembly/config artifact');
  const identities = await authorHttpLiveArtifact({
    skiffRoot: repoRoot,
    sourceRoot,
    artifactRoot,
    environment: HTTP_LIVE_ENVIRONMENT,
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
  mongoHarness = await ActivationStateMongoHarness.create({ repoRoot });
  await mongoHarness.start();

  const mongoUrl = httpLiveMongoUrl(mongoHarness.port);
  console.log('router-live:http: seeding committed activation state (TS + Rust namespaces)');
  const committed = await seedHttpLiveCommittedState({
    mongoUrl,
    environment: HTTP_LIVE_ENVIRONMENT,
    generation: HTTP_LIVE_GENERATION,
    assemblyIdentity: identities.assemblyIdentity,
    configSnapshotId: identities.configSnapshotId,
  });

  console.log('router-live:http: ensuring TS router dependencies');
  await ensureTsRouterDependencies({ repoRoot });

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
      environment: HTTP_LIVE_ENVIRONMENT,
      artifactsPath: artifactRoot,
      httpPort,
      runtimePort,
      mongoUrl,
    }),
  );

  const specs = createHttpLiveRouterSpecs({ repoRoot, devHome });
  evidence.manifests = {
    ts: specs.ts.manifest,
    rust: specs.rust.manifest,
  };

  const runtimeConfigPath = join(tempRoot, 'runtime.yml');
  await writeHttpLiveRuntimeConfig(runtimeConfigPath, {
    relayPort,
    runtimeHome,
    environment: HTTP_LIVE_ENVIRONMENT,
  });

  // Phase 1: TS Router. Each Router phase owns a fresh relay + Runtime
  // process pair: when the Router exits, the relay's downstream socket would
  // otherwise stay open forever (the relay only detaches, it does not close
  // the peer), so the Runtime is stopped inside the phase before the relay
  // closes. The Runtime reuses the same runtime-home/replica id and re-seeds
  // the exact committed tuple on every phase.
  await runRouterPhase({
    phase: 'ts-1',
    implementation: 'ts',
    invocation: specs.ts.invocation,
    httpPort,
    runtimePort,
    relayPort,
    full: false,
    routerLogsDir: join(tempRoot, 'phase-ts-1'),
    runtimeBin,
    runtimeConfigPath,
  });

  // Phase 2: Rust Router (same config, canonical Rust process command).
  await installHttpLiveRustBinary({ sourceBinary: routerSourceBinary, devHome });
  await runRouterPhase({
    phase: 'rust',
    implementation: 'rust',
    invocation: specs.rust.invocation,
    httpPort,
    runtimePort,
    relayPort,
    full: true,
    routerLogsDir: join(tempRoot, 'phase-rust'),
    runtimeBin,
    runtimeConfigPath,
  });

  // Phase 3: TS Router again (rollback to the immutable TS process command).
  await runRouterPhase({
    phase: 'ts-2',
    implementation: 'ts',
    invocation: specs.ts.invocation,
    httpPort,
    runtimePort,
    relayPort,
    full: false,
    routerLogsDir: join(tempRoot, 'phase-ts-2'),
    runtimeBin,
    runtimeConfigPath,
  });

  assertRollbackRoundtrip(evidence, committed);

  console.log('router-live:http: PASS');
  console.log(JSON.stringify(rollbackEvidence(evidence), null, 2));
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
  implementation,
  invocation,
  httpPort,
  runtimePort,
  relayPort,
  full,
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
  let suite;
  let phaseError;
  try {
    await waitForListeners({
      httpPort,
      runtimePort,
      child: router.child,
      stderrPath,
    });
    relay = await createRuntimeRelay({
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
    suite = full ? await runFullSuite(ctx) : await runRollbackSuite(ctx);
    evidence.suite.push({ phase, cases: suite });
    console.log(`router-live:http: ${phase} phase passed (${suite.length} cases)`);
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
      evidence.phases.push({ phase, implementation, exit });
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
  return suite;
}

function assertRollbackRoundtrip(evidence, committed) {
  if (evidence.bootstrapTuples.length !== 3) {
    throw new Error(
      `rollback roundtrip requires three phases, got ${evidence.bootstrapTuples.length}`,
    );
  }
  const [ts1, rust, ts2] = evidence.bootstrapTuples.map((entry) => entry.tuple);
  assertDeepEqual(ts1, rust, 'TS-1 and Rust bootstrap tuples');
  assertDeepEqual(rust, ts2, 'Rust and TS-2 bootstrap tuples');
  assertEqual(ts1.environment, committed.environment, 'bootstrap environment');
  assertEqual(ts1.generation, committed.generation, 'bootstrap generation');
  assertEqual(ts1.assemblyIdentity, committed.assemblyIdentity, 'bootstrap assembly identity');
  assertEqual(ts1.configSnapshotId, committed.configSnapshotId, 'bootstrap config snapshot id');

  const unaryByName = new Map();
  for (const phase of evidence.suite) {
    for (const entry of phase.cases) {
      if (!['unary-happy', 'typed-unary', 'stream-roundtrip'].includes(entry.name)) {
        continue;
      }
      const key = `${entry.name}:${entry.status}`;
      unaryByName.set(key, (unaryByName.get(key) ?? 0) + 1);
    }
  }
  for (const name of ['unary-happy', 'typed-unary', 'stream-roundtrip']) {
    const count = [...unaryByName.entries()]
      .filter(([key]) => key.startsWith(`${name}:`))
      .reduce((total, [, value]) => total + value, 0);
    if (count !== 3) {
      throw new Error(`rollback roundtrip ${name} must pass in all three phases, got ${count}/3`);
    }
  }
}

function rollbackEvidence(evidence) {
  return {
    roundtrip: {
      phases: evidence.bootstrapTuples.map(({ phase, tuple }) => ({ phase, tuple })),
      manifests: evidence.manifests,
      runtimeExit: evidence.runtimeExit,
    },
    suite: evidence.suite,
    phases: evidence.phases.map(({ phase, implementation, exit }) => ({
      phase,
      implementation,
      exit,
    })),
  };
}

function evidenceSummary(evidence) {
  return {
    manifests: evidence.manifests,
    bootstrapTuples: evidence.bootstrapTuples,
    phases: evidence.phases,
    suite: evidence.suite,
    runtimeExit: evidence.runtimeExit,
    relayTails: evidence.relayTails,
  };
}

function assertDeepEqual(left, right, label) {
  const leftJson = JSON.stringify(left);
  const rightJson = JSON.stringify(right);
  if (leftJson !== rightJson) {
    throw new Error(`${label} differ:\n${leftJson}\n${rightJson}`);
  }
}

function assertEqual(actual, expected, label) {
  if (!Object.is(actual, expected)) {
    throw new Error(
      `${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
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
