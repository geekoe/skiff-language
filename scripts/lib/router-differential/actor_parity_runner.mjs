// E-actor-parity differential runner (plan §7/§8/§9).
//
// Authors nothing itself: it consumes the byte-identical source artifact
// already produced by `scripts/check-router-actor-live.mjs` (including the
// canonical actor-routing projection record), copies it into independent
// per-side artifact roots, runs the identical two-replica real-HTTP full
// chain against the TS and Rust Router sides, and compares the normalized
// observations with the shared compare engine.

import { cp, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { ActivationStateMongoHarness } from '../activation-state-live-harness.mjs';
import { leaseConsecutiveLocalPorts } from '../local-port-lease.mjs';

import {
  ACTOR_PARITY_ENVIRONMENT,
  ACTOR_PARITY_GENERATION,
  ACTOR_PARITY_PORTS_PER_SIDE,
  ROUTER_PORT_MAX,
  ROUTER_PORT_MIN,
} from './actor_parity_constants.mjs';
import { runActorParityFullChain } from './actor_parity_driver.mjs';
import { actorParityEntrypoints } from './actor_parity_fixture.mjs';
import {
  actorParitySideContextObservation,
  actorParityTerminalObservation,
  captureActorParitySide,
  createActorParitySide,
  readActorParitySideLogs,
  startActorParitySide,
  stopActorParitySide,
} from './actor_parity_instance.mjs';
import {
  assertActorParityScenarioRunnable,
  loadActorParityInventory,
} from './actor_parity_scenarios.mjs';
import {
  compareObservations,
  renderDifferentialReport,
} from './compare.mjs';

const implementations = Object.freeze(['ts', 'rust']);

export async function runActorParityDifferential({
  repoRoot,
  sourceArtifactRoot,
  assemblyIdentity,
  configSnapshotId,
  deploymentRecord,
  runtimeBin,
  routerBinary,
  environment = ACTOR_PARITY_ENVIRONMENT,
  keepTemp = false,
}) {
  const inventory = await loadActorParityInventory({ skiffRoot: repoRoot });
  const scenario = inventory.scenarios.find(
    (candidate) => candidate.id === 'actor_parity_full_chain',
  );
  if (scenario === undefined) {
    throw new Error('actor parity inventory is missing actor_parity_full_chain');
  }
  assertActorParityScenarioRunnable(scenario);

  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-actor-parity-'));
  const resources = {
    tempRoot,
    mongos: [],
    leases: [],
    sides: [],
  };
  try {
    const sideArtifacts = join(tempRoot, 'source-artifacts');
    await cp(sourceArtifactRoot, sideArtifacts, { recursive: true });
    const entrypoints = actorParityEntrypoints(deploymentRecord.gatewayEntries);
    const deployment = deploymentRecord.deployment;
    const sideObservations = new Map();

    for (const implementation of implementations) {
      const artifactRoot = join(tempRoot, `${implementation}-artifacts`);
      await cp(sideArtifacts, artifactRoot, { recursive: true });
      const lease = await leaseConsecutiveLocalPorts({
        rangeStart: ROUTER_PORT_MIN,
        rangeEnd: ROUTER_PORT_MAX,
        count: ACTOR_PARITY_PORTS_PER_SIDE,
      });
      resources.leases.push(lease);
      const mongo = await ActivationStateMongoHarness.create({ repoRoot });
      resources.mongos.push(mongo);
      await mongo.start();

      const side = await createActorParitySide({
        repoRoot,
        implementation,
        tempRoot,
        ports: lease.ports,
        mongoPort: mongo.port,
        artifactRoot,
        assemblyIdentity,
        configSnapshotId,
        runtimeBin,
        routerSourceBinary: routerBinary,
        environment,
        generation: ACTOR_PARITY_GENERATION,
      });
      resources.sides.push(side);
      console.log(
        `router-live:actor: starting actor parity ${implementation} side`,
      );
      await startActorParitySide(side);
      const driverResult = await runActorParityFullChain({
        httpPort: side.httpPort,
        entrypoints,
        deployment,
      });
      side.driverTimings = driverResult.timings;
      side.capture = await captureActorParitySide(side, driverResult);
      await stopActorParitySide(side);
      side.logs = await readActorParitySideLogs(side);
      side.observation = buildActorParityObservation(side);
      sideObservations.set(implementation, side);
      console.log(
        `router-live:actor: actor parity ${implementation} side captured`,
      );
    }

    const report = compareObservations({
      scenario,
      tsObservation: sideObservations.get('ts').observation,
      rustObservation: sideObservations.get('rust').observation,
      tsSideContext: actorParitySideContextObservation(sideObservations.get('ts')),
      rustSideContext: actorParitySideContextObservation(sideObservations.get('rust')),
    });
    console.log(renderDifferentialReport(report));
    return report;
  } catch (error) {
    error.actorParityEvidence = await collectFailureEvidence(resources);
    error.actorParityFrames = collectFrameEvidence(resources);
    throw error;
  } finally {
    const cleanupErrors = await cleanupResources(resources, { keepTemp });
    if (cleanupErrors.length > 0) {
      throw new AggregateError(
        cleanupErrors,
        `actor parity differential cleanup failed; evidence preserved at ${tempRoot}`,
      );
    }
  }
}

function buildActorParityObservation(side) {
  return {
    http: side.capture.http,
    frameEvents: side.capture.frameEvents,
    rawFrames: side.capture.rawFrames,
    mongo: side.capture.mongo,
    terminal: actorParityTerminalObservation(side),
    timings: side.driverTimings,
    logs: side.logs,
  };
}

async function collectFailureEvidence(resources) {
  const lines = [`actor parity evidence root: ${resources.tempRoot}`];
  for (const side of resources.sides) {
    lines.push(`side ${side.implementation}:`);
    for (const [label, path] of [
      ['router stdout', side.routerLogs.stdout],
      ['router stderr', side.routerLogs.stderr],
      ['runtime one stdout', side.runtimeOneLogs.stdout],
      ['runtime one stderr', side.runtimeOneLogs.stderr],
      ['runtime two stdout', side.runtimeTwoLogs.stdout],
      ['runtime two stderr', side.runtimeTwoLogs.stderr],
    ]) {
      try {
        const text = await readFile(path, 'utf8');
        lines.push(`  ${label}:\n${text.trim().slice(-8000)}`);
      } catch {
        lines.push(`  ${label}: <unavailable>`);
      }
    }
  }
  return lines.join('\n');
}

function collectFrameEvidence(resources) {
  const frames = {};
  for (const side of resources.sides) {
    const sides = [];
    for (const [label, relay] of [
      ['relay1', side.relayOne],
      ['relay2', side.relayTwo],
    ]) {
      if (relay === undefined) {
        continue;
      }
      const relevant = relay.records
        .filter((record) => typeof record.type === 'string')
        .filter((record) =>
          record.type === 'spawn.submit.error'
          || record.type === 'actor.method.error'
          || record.type === 'actor.getOrCreate.error')
        .map((record) => ({
          direction: record.direction,
          type: record.type,
          rpcId: record.header?.rpcId,
          requestId: record.header?.requestId,
          invocationId: record.header?.invocationId,
          error: record.header?.error,
        }));
      if (relevant.length > 0) {
        sides.push({ relay: label, frames: relevant });
      }
    }
    frames[side.implementation] = sides;
  }
  return frames;
}

async function cleanupResources(resources, { keepTemp = false } = {}) {
  const errors = [];
  for (const side of [...resources.sides].reverse()) {
    try {
      await stopActorParitySide(side);
    } catch (error) {
      errors.push(error);
    }
  }
  for (const mongo of [...resources.mongos].reverse()) {
    try {
      await mongo.cleanup();
    } catch (error) {
      errors.push(error);
    }
  }
  for (const lease of [...resources.leases].reverse()) {
    try {
      await lease.release();
    } catch (error) {
      errors.push(error);
    }
  }
  if (!keepTemp) {
    try {
      await rm(resources.tempRoot, { recursive: true, force: true });
    } catch (error) {
      errors.push(error);
    }
  }
  return errors;
}
