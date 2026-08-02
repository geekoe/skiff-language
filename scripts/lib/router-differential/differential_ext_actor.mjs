// Differential extension: real actor call/control traffic
// (`differential_ext_actor_*` scenarios, plan §9).
//
// Actor get-or-create/invocation/owner-control runs on two real Runtime
// replicas per side (the synchronous self-call fixture deadlocks on a single
// replica). This extension spawns the second replica through its own
// test-only relay on a leased 45000-45999 port, drives HTTP typedJson probes
// into the ext-actor fixture, then derives deterministic actor frame
// observations: combined per-type counts (robust to which replica executed
// the probe) plus per-relay sequences as evidence. The second runtime is
// stopped and its relay closed before the extension returns; the harness
// still owns the primary side lifecycle.

import { spawn } from 'node:child_process';
import { access, mkdir, open, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { requestFull, selectorHeaders } from '../http_live_client.mjs';
import { leaseConsecutiveLocalPorts } from '../local-port-lease.mjs';
import { renderRuntimeConfig } from '../runtime-stack-config.mjs';
import {
  REPLICA_ID,
  ROUTER_PORT_MAX,
  ROUTER_PORT_MIN,
} from './constants.mjs';
import { createRuntimeRelay } from './relay.mjs';

export const EXT_ACTOR_SERVICE_ID = 'test.skiff/router-rust-differential-ext-actor';
export const EXT_ACTOR_VERSION = '1.0.0';

const NULL_BODY = Buffer.from('null', 'utf8');
const HANDSHAKE_TIMEOUT_MS = 60_000;
const STOP_TIMEOUT_MS = 15_000;

async function typedJsonProbe(side, path) {
  const response = await requestFull({
    port: side.httpPort,
    method: 'POST',
    path,
    headers: selectorHeaders({
      service: EXT_ACTOR_SERVICE_ID,
      version: EXT_ACTOR_VERSION,
    }),
    body: NULL_BODY,
  });
  return {
    status: response.status,
    body: response.body.toString('utf8'),
  };
}

function actorFrameTypes(relay) {
  return relay.records
    .filter((record) => typeof record.type === 'string' && record.type.startsWith('actor.'))
    .map((record) => `${record.direction}:${record.type}`);
}

function combinedActorFrameCounts(relays) {
  const counts = {};
  for (const relay of relays) {
    for (const entry of actorFrameTypes(relay)) {
      counts[entry] = (counts[entry] ?? 0) + 1;
    }
  }
  return Object.fromEntries(
    Object.entries(counts).sort(([a], [b]) => a.localeCompare(b)),
  );
}

async function spawnRuntimeReplica({ side, resources }) {
  const lease = await leaseConsecutiveLocalPorts({
    rangeStart: ROUTER_PORT_MIN,
    rangeEnd: ROUTER_PORT_MAX,
    count: 1,
  });
  resources.leases.push(lease);
  const relayPort = lease.ports[0];
  const relay = await createRuntimeRelay({
    port: relayPort,
    routerUrl: `ws://127.0.0.1:${side.runtimePort}/runtime`,
  });
  const runtimeHome = join(side.sideRoot, 'runtime-2-home');
  const runtimeConfigPath = join(side.sideRoot, 'runtime-2.yml');
  const stdoutLogPath = join(side.sideRoot, 'runtime-2.stdout.log');
  const stderrLogPath = join(side.sideRoot, 'runtime-2.stderr.log');
  await mkdir(runtimeHome, { recursive: true });
  await writeFile(join(runtimeHome, 'runtime-id'), `${REPLICA_ID}-2\n`, {
    encoding: 'utf8',
    flag: 'wx',
    mode: 0o600,
  });
  await writeFile(
    runtimeConfigPath,
    renderRuntimeConfig({
      routerUrl: `ws://127.0.0.1:${relayPort}/runtime`,
      runtimeHome,
      environment: side.environment,
    }),
    { encoding: 'utf8', flag: 'wx', mode: 0o600 },
  );
  await access(side.runtimeBin);
  const child = await spawnWithLogs(
    side.runtimeBin,
    [runtimeConfigPath],
    { cwd: side.repoRoot, stdoutPath: stdoutLogPath, stderrPath: stderrLogPath },
  );
  await relay.waitForHandshake({ timeoutMs: HANDSHAKE_TIMEOUT_MS });
  return { lease, relay, relayPort, child, runtimeHome, stdoutLogPath, stderrLogPath };
}

async function stopRuntimeReplica(replica, side) {
  const errors = [];
  if (replica.child.child.exitCode === null && replica.child.child.signalCode === null) {
    try {
      replica.child.child.kill('SIGINT');
      await new Promise((resolvePromise, reject) => {
        const timer = setTimeout(() => {
          replica.child.child.kill('SIGKILL');
          reject(new Error('differential actor second runtime did not exit'));
        }, STOP_TIMEOUT_MS);
        replica.child.child.once('exit', (code, signal) => {
          clearTimeout(timer);
          resolvePromise({ code, signal });
        });
      });
    } catch (error) {
      errors.push(error);
    }
  }
  try {
    await replica.relay.close();
  } catch (error) {
    errors.push(error);
  }
  for (const handle of [replica.child.stdoutLog, replica.child.stderrLog]) {
    try {
      await handle.close();
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'differential actor second replica cleanup failed');
  }
}

function spawnWithLogs(command, args, { cwd, stdoutPath, stderrPath }) {
  const stdoutLog = open(stdoutPath, 'w');
  const stderrLog = open(stderrPath, 'w');
  return Promise.all([stdoutLog, stderrLog]).then(([stdoutHandle, stderrHandle]) => {
    const child = spawn(command, args, {
      cwd,
      stdio: ['ignore', stdoutHandle.fd, stderrHandle.fd],
      env: process.env,
    });
    return { child, stdoutLog: stdoutHandle, stderrLog: stderrHandle };
  });
}

export async function captureDifferentialExtActor({ side, scenario, resources }) {
  const primaryStartIndex = side.relay.records.length;
  const replica = await spawnRuntimeReplica({ side, resources });
  const secondRelayStartIndex = replica.relay.records.length;
  const probes = [];
  try {
    switch (scenario.actorMode) {
      case 'call':
        probes.push(
          { name: 'probe-1', ...(await typedJsonProbe(side, '/probe')) },
          { name: 'probe-2', ...(await typedJsonProbe(side, '/probe')) },
        );
        break;
      case 'control':
        probes.push(
          { name: 'slow-get', ...(await typedJsonProbe(side, '/slow-get')) },
          { name: 'slow-increment', ...(await typedJsonProbe(side, '/slow-increment')) },
          { name: 'probe', ...(await typedJsonProbe(side, '/probe')) },
        );
        break;
      default:
        throw new Error(
          `differential_ext_actor scenario requires actorMode, got ${JSON.stringify(scenario.actorMode)}`,
        );
    }
  } finally {
    await stopRuntimeReplica(replica, side);
  }
  const primarySequence = actorFrameTypes(side.relay).slice(primaryStartIndex);
  const secondSequence = actorFrameTypes(replica.relay).slice(secondRelayStartIndex);
  return {
    actorTraffic: probes,
    actorFrames: {
      counts: combinedActorFrameCounts([
        { records: side.relay.records.slice(primaryStartIndex) },
        { records: replica.relay.records.slice(secondRelayStartIndex) },
      ]),
      primarySequence,
      secondSequence,
    },
  };
}
