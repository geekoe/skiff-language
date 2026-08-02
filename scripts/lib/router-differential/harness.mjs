// Top-level differential harness: builds explicit binaries and artifacts,
// runs each runnable scenario against isolated TS and Rust instances, then
// compares the normalized observations.

import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { ActivationStateMongoHarness } from '../activation-state-live-harness.mjs';
import { cargoTargetDir } from '../cargo-target-dir.mjs';
import { captureCheckedCommand } from '../command-execution.mjs';
import { leaseConsecutiveLocalPorts } from '../local-port-lease.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from '../package-service-authoring.mjs';

import {
  ACTOR_ROUTING_PROJECTION_CONTENT,
  ACTOR_ROUTING_PROJECTION_RECORD_PATH,
  ENVIRONMENT,
  GENERATION,
  ROUTER_PORT_MAX,
  ROUTER_PORT_MIN,
  ROUTER_PORTS_PER_SIDE,
  fixtureServicePath,
  routerBinaryPath,
  runtimeBinaryPath,
} from './constants.mjs';
import {
  compareObservations,
  renderDifferentialReport,
} from './compare.mjs';
import {
  assertSelectedScenarioRunnable,
  loadScenarioInventory,
} from './scenarios.mjs';
import {
  captureDifferentialSide,
  createSideContext,
  readSideLogs,
  sideContextObservation,
  startDifferentialSide,
  stopDifferentialSide,
  terminalObservation,
} from './instance.mjs';

const implementations = Object.freeze(['ts', 'rust']);

export async function runDifferentialHarness({
  repoRoot,
  scenarioId,
  only,
  keepTemp = false,
} = {}) {
  const inventory = await loadScenarioInventory({ skiffRoot: repoRoot });
  const scenarios = selectScenarios(inventory, scenarioId);
  const selectedOnly = only === undefined ? undefined : assertImplementation(only);

  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-differential-'));
  const targetDir = cargoTargetDir(repoRoot);
  const binaries = await buildExplicitBinaries({ repoRoot, targetDir });
  const resources = {
    tempRoot,
    mongos: [],
    leases: [],
    sides: [],
  };

  try {
    const reports = [];
    for (const scenario of scenarios) {
      const sideObservations = new Map();
      // Author the canonical artifact exactly once per scenario, then copy
      // it into each side's independent artifact root. Authoring per side
      // would embed the artifact root into the config snapshot record and
      // produce different snapshot ids for semantically identical input.
      const sourceArtifactRoot = join(tempRoot, `${scenario.id}-source-artifacts`);
      await mkdir(sourceArtifactRoot, { recursive: true });
      const identities = await authorArtifact({
        repoRoot,
        artifactRoot: sourceArtifactRoot,
        environment: ENVIRONMENT,
      });
      for (const implementation of selectedOnly ?? implementations) {
        const artifactRoot = join(tempRoot, `${scenario.id}-${implementation}-artifacts`);
        await cp(sourceArtifactRoot, artifactRoot, { recursive: true });
        const lease = await leaseConsecutiveLocalPorts({
          rangeStart: ROUTER_PORT_MIN,
          rangeEnd: ROUTER_PORT_MAX,
          count: ROUTER_PORTS_PER_SIDE,
        });
        resources.leases.push(lease);
        const mongo = await ActivationStateMongoHarness.create({ repoRoot });
        resources.mongos.push(mongo);
        await mongo.start();

        const side = await createSideContext({
          repoRoot,
          implementation,
          tempRoot,
          ports: lease.ports,
          mongoPort: mongo.port,
          artifactRoot,
          assemblyIdentity: identities.assemblyIdentity,
          configSnapshotId: identities.configSnapshotId,
          runtimeBin: binaries.runtimeBinary,
          routerSourceBinary: binaries.routerBinary,
          environment: ENVIRONMENT,
          generation: GENERATION,
        });
        resources.sides.push(side);
        console.log(`router-live:differential: starting ${implementation} side for ${scenario.id}`);
        await startDifferentialSide(side);
        side.capture = await captureDifferentialSide(side);
        await stopDifferentialSide(side);
        side.logs = await readSideLogs(side);
        side.observation = buildObservation(side);
        sideObservations.set(implementation, side);
        console.log(`router-live:differential: ${implementation} side captured for ${scenario.id}`);
      }

      if (selectedOnly !== undefined) {
        const side = sideObservations.get(selectedOnly);
        reports.push({
          scenarioId: scenario.id,
          mode: `single:${selectedOnly}`,
          observation: side.observation,
          passed: [],
          failures: [],
        });
        continue;
      }

      const report = compareObservations({
        scenario,
        tsObservation: sideObservations.get('ts').observation,
        rustObservation: sideObservations.get('rust').observation,
        tsSideContext: sideContextObservation(sideObservations.get('ts')),
        rustSideContext: sideContextObservation(sideObservations.get('rust')),
      });
      console.log(renderDifferentialReport(report));
      reports.push(report);
      if (report.failures.length > 0) {
        throw new Error(
          `differential scenario ${scenario.id} failed: ${report.failures.join('; ')}`,
        );
      }
    }
    return {
      tempRoot,
      reports,
      inventory: inventory.scenarios.map(({ id, status, lane }) => ({ id, status, lane })),
    };
  } catch (error) {
    error.differentialEvidence = await collectFailureEvidence(resources);
    throw error;
  } finally {
    const cleanupErrors = await cleanupResources(resources, { keepTemp });
    if (cleanupErrors.length > 0) {
      throw new AggregateError(
        cleanupErrors,
        `router-live:differential cleanup failed; evidence preserved at ${tempRoot}`,
      );
    }
  }
}

function selectScenarios(inventory, scenarioId) {
  if (scenarioId !== undefined) {
    const scenario = inventory.scenarios.find((candidate) => candidate.id === scenarioId);
    if (scenario === undefined) {
      throw new Error(
        `unknown differential scenario ${scenarioId}; `
        + `available: ${inventory.scenarios.map((candidate) => candidate.id).join(', ')}`,
      );
    }
    assertSelectedScenarioRunnable(scenario);
    return [scenario];
  }
  const runnable = inventory.scenarios.filter((scenario) => scenario.status === 'runnable');
  if (runnable.length === 0) {
    throw new Error('differential inventory has no runnable scenarios');
  }
  return runnable;
}

function assertImplementation(value) {
  if (!implementations.includes(value)) {
    throw new Error(`--only must be ts or rust, got ${value}`);
  }
  return value;
}

function buildObservation(side) {
  return {
    http: side.capture.http,
    runtimeFrames: side.capture.runtimeFrames,
    mongo: side.capture.mongo,
    terminal: terminalObservation(side),
    logs: side.logs,
  };
}

async function buildExplicitBinaries({ repoRoot, targetDir }) {
  const env = { ...process.env, CARGO_TARGET_DIR: targetDir };
  console.log('router-live:differential: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env },
  );
  console.log('router-live:differential: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env },
  );
  return {
    routerBinary: routerBinaryPath(targetDir),
    runtimeBinary: runtimeBinaryPath(targetDir),
  };
}

async function authorArtifact({
  repoRoot,
  artifactRoot,
  environment,
}) {
  const sourceRoot = fixtureServicePath(repoRoot);
  await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    environment,
  });
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot: repoRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    environment,
    rootDeployments: [],
  });
  const assembly = assemblyReceipt?.runtimeAssemblyReceipt?.assembly;
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  const assemblyIdentity = assembly?.assemblyIdentity;
  if (typeof assemblyIdentity !== 'string' || typeof recordPath !== 'string') {
    throw new Error('compiler assembly build returned no exact RuntimeAssembly receipt');
  }
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot: repoRoot,
    artifactRoot,
    environment,
    profile: 'dev',
    assemblyRecord: recordPath,
    sources: [],
  });
  const configSnapshotId =
    snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof configSnapshotId !== 'string') {
    throw new Error('config snapshot production returned no exact snapshot reference');
  }
  const projectionDirectory = join(artifactRoot, 'records', 'actor-routing');
  await mkdir(projectionDirectory, { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
    { encoding: 'utf8', flag: 'wx' },
  );
  return { assemblyIdentity, configSnapshotId };
}

async function collectFailureEvidence(resources) {
  const { readFile } = await import('node:fs/promises');
  const lines = [`evidence root: ${resources.tempRoot}`];
  for (const side of resources.sides) {
    lines.push(`side ${side.implementation}:`);
    for (const [label, path] of [
      ['router stdout', side.routerLogs.stdout],
      ['router stderr', side.routerLogs.stderr],
      ['runtime stdout', side.runtimeLogs.stdout],
      ['runtime stderr', side.runtimeLogs.stderr],
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

async function cleanupResources(resources, { keepTemp }) {
  const errors = [];
  for (const side of [...resources.sides].reverse()) {
    try {
      await stopDifferentialSide(side);
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
