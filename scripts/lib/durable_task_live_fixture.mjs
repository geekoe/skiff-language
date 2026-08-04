// E3a durable-task E2E fixture authoring (`durable-task-e2e-live`).
//
// Writes the real service source from `test-runner/fixtures/durable-task-e2e-live`
// (dispatch statements/expressions, after/at timing, function and actor-method
// targets, std.task status/cancel, TaskRef stored DB field), then produces the
// real compiler package/assembly/config-snapshot artifacts through the actual
// compiler binary. The A0 actor-routing projection is synthesized by the shared
// test-side producer (`actor-live-projection.mjs`), and the deployment record is
// read with the shared reader (`actor_live_fixture.mjs`).

import { copyFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';

import { cargoBuildEnv } from './cargo-target-dir.mjs';
import { captureCheckedCommand } from './command-execution.mjs';
import { loadActorLiveDeploymentRecord } from './actor_live_fixture.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './package-service-authoring.mjs';

export const DURABLE_TASK_LIVE_SERVICE_ID = 'test.skiff/durable-task-e2e-live';
export const DURABLE_TASK_LIVE_VERSION = '1.0.0';
export const DURABLE_TASK_LIVE_PROFILE = 'durable-task-e2e-live';
export const DURABLE_TASK_LIVE_DATABASE = 'skiff_e2e_task_live';

export const DURABLE_TASK_LIVE_ENTRYPOINTS = Object.freeze({
  'submit-immediate': { path: '/submit-immediate' },
  'submit-after': { path: '/submit-after' },
  'submit-slow': { path: '/submit-slow' },
  'submit-actor': { path: '/submit-actor' },
  'submit-actor-after': { path: '/submit-actor-after' },
  'submit-actor-direct': { path: '/submit-actor-direct' },
  status: { path: '/status' },
  cancel: { path: '/cancel' },
  effect: { path: '/effect' },
  'actor-count': { path: '/actor-count' },
});

const FIXTURE_FILES = [
  'package.yml',
  'service.yml',
  'api.yml',
  'http.yml',
  'main.skiff',
];

export function durableTaskLiveMongoUrl() {
  return 'mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false';
}

/// The service-owned database name derived from `DURABLE_TASK_LIVE_SERVICE_ID`
/// (`~` replaces `.` and `~~` replaces `/`, matching the runtime's
/// `service_storage_database_name` in `runtime/service-db/src/storage_identity.rs`).
export function durableTaskLiveServiceDatabase() {
  return DURABLE_TASK_LIVE_SERVICE_ID.replace(/\./g, '~').replace(/\//g, '~~');
}

export async function writeDurableTaskServiceSource(sourceRoot, fixtureRoot) {
  await mkdir(sourceRoot, { recursive: true });
  for (const name of FIXTURE_FILES) {
    await copyFile(join(fixtureRoot, name), join(sourceRoot, name));
  }
}

export async function authorDurableTaskArtifact({
  skiffRoot,
  sourceRoot,
  artifactRoot,
  profile = DURABLE_TASK_LIVE_PROFILE,
}) {
  await mkdir(artifactRoot, { recursive: true });
  await captureCheckedCommand(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '--manifest-path',
      join(skiffRoot, 'test-runner', 'Cargo.toml'),
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      '--bootstrap-only',
      '--artifact-root',
      artifactRoot,
      '--profile',
      profile,
      '--platform-source-root',
      skiffRoot,
    ],
    { cwd: skiffRoot, env: cargoBuildEnv(skiffRoot) },
  );
  const packageReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    profile,
  });
  const deploymentRef = packageReceipt?.serviceDeploymentReceipt?.deployment;
  if (
    deploymentRef === null
    || typeof deploymentRef !== 'object'
    || typeof deploymentRef.serviceId !== 'string'
    || typeof deploymentRef.contractVersion !== 'string'
    || typeof deploymentRef.deploymentRevision !== 'string'
    || typeof deploymentRef.deploymentArtifactIdentity !== 'string'
  ) {
    throw new Error('durable task live package build returned no exact ServiceDeploymentRef');
  }
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    profile,
    rootDeployments: [deploymentRef],
  });
  const assembly = assemblyReceipt?.runtimeAssemblyReceipt?.assembly;
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  if (
    assembly === null
    || typeof assembly !== 'object'
    || typeof assembly.assemblyIdentity !== 'string'
    || typeof recordPath !== 'string'
  ) {
    throw new Error('durable task live assembly build returned no exact RuntimeAssembly receipt');
  }
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    profile,
    assemblyRecord: recordPath,
    sources: [{ root: sourceRoot, deployment: deploymentRef }],
  });
  const snapshotId =
    snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof snapshotId !== 'string') {
    throw new Error('durable task live config snapshot production returned no snapshot id');
  }
  return {
    assemblyIdentity: assembly.assemblyIdentity,
    configSnapshotId: snapshotId,
    deployment: deploymentRef,
  };
}

export async function entrypointList(deploymentRecord) {
  const entries = [];
  for (const [key, entry] of Object.entries(DURABLE_TASK_LIVE_ENTRYPOINTS)) {
    const identity = deploymentRecord.gatewayEntries[key];
    if (typeof identity !== 'string') {
      throw new Error(`durable task live gateway entry ${key} has no identity`);
    }
    entries.push({
      gatewayEntryKey: key,
      gatewayEntryIdentity: identity,
      deployment: deploymentRecord.deployment,
      selector: {
        method: 'POST',
        path: entry.path,
        protocol: 'http',
      },
    });
  }
  return entries;
}

export { loadActorLiveDeploymentRecord as loadDurableTaskDeploymentRecord };
