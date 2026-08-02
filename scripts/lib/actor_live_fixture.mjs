// E-actor-rust live fixture authoring (`router-live:actor`, plan §7/§8).
//
// Writes a real ordinary service source with actor declarations and HTTP
// gateway entries (probe/slowGet/slowDedup/flaky/synchronous-self/spawn
// family/chain probes), then produces the real compiler package/assembly/
// config-snapshot artifacts through the actual compiler binary. The service
// body is copied verbatim from the canonical actor-full-chain-acceptance
// fixture's `main.skiff`; only the test-case-specific `http.yml` entries
// (actor test effects) are omitted so the committed epoch carries exactly
// one service deployment.

import { copyFile, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

import { captureCheckedCommand } from './command-execution.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './package-service-authoring.mjs';

export const ACTOR_LIVE_SERVICE_ID = 'test.skiff/router-rust-actor-live';
export const ACTOR_LIVE_VERSION = '1.0.0';
export const ACTOR_LIVE_ENVIRONMENT = 'actor-live';

export const ACTOR_LIVE_ENTRYPOINTS = Object.freeze({
  probe: { path: '/probe', handler: 'main.__skiffHttpProbe' },
  slowGet: { path: '/slow-get', handler: 'main.slowGetOnly' },
  slowDedup: { path: '/slow-dedup', handler: 'main.slowDedup' },
  slowIncrement: { path: '/slow-increment', handler: 'main.slowIncrement' },
  flakyGet: { path: '/flaky-get', handler: 'main.flakyGet' },
  synchronousSelfCall: { path: '/synchronous-self-call', handler: 'main.synchronousSelfCall' },
  synchronousSelfCount: { path: '/synchronous-self-count', handler: 'main.synchronousSelfCount' },
  spawnExternal: { path: '/spawn-external', handler: 'main.spawnExternal' },
  externalCount: { path: '/external-count', handler: 'main.externalCount' },
  externalHistory: { path: '/external-history', handler: 'main.externalHistory' },
  spawnSelfKick: { path: '/spawn-self-kick', handler: 'main.spawnSelfKick' },
  selfKickCount: { path: '/self-kick-count', handler: 'main.selfKickCount' },
  selfKickHistory: { path: '/self-kick-history', handler: 'main.selfKickHistory' },
  spawnFanout: { path: '/spawn-fanout', handler: 'main.spawnFanout' },
  fanoutCount: { path: '/fanout-count', handler: 'main.fanoutCount' },
  fanoutHistory: { path: '/fanout-history', handler: 'main.fanoutHistory' },
  chainKick: { path: '/chain-kick', handler: 'main.chainKick' },
  chainSteps: { path: '/chain-steps', handler: 'main.chainSteps' },
  chainHistory: { path: '/chain-history', handler: 'main.chainHistory' },
  spawnThrow: { path: '/spawn-throw', handler: 'main.spawnThrow' },
});

export async function writeActorLiveServiceSource(sourceRoot, actorFixtureRoot) {
  await mkdir(sourceRoot, { recursive: true });
  await writeFile(
    join(sourceRoot, 'package.yml'),
    `id: ${ACTOR_LIVE_SERVICE_ID}\nversion: ${ACTOR_LIVE_VERSION}\n`,
  );
  await writeFile(
    join(sourceRoot, 'service.yml'),
    `id: ${ACTOR_LIVE_SERVICE_ID}\n`,
  );
  await writeFile(join(sourceRoot, 'api.yml'), '{}\n');
  const httpLines = [];
  for (const [key, entry] of Object.entries(ACTOR_LIVE_ENTRYPOINTS)) {
    httpLines.push(
      `${key}:`,
      '  method: POST',
      `  path: ${entry.path}`,
      '  kind: typedJson',
      `  handler: ${entry.handler}`,
      '  adapterArgs:',
      '    - param: body',
      '      source: { kind: http.body }',
      '',
    );
  }
  await writeFile(join(sourceRoot, 'http.yml'), `${httpLines.join('\n')}\n`);
  await copyFile(join(actorFixtureRoot, 'main.skiff'), join(sourceRoot, 'main.skiff'));
}

export async function authorActorLiveArtifact({
  skiffRoot,
  sourceRoot,
  artifactRoot,
  environment = ACTOR_LIVE_ENVIRONMENT,
}) {
  await mkdir(artifactRoot, { recursive: true });
  // Seed the canonical skiff.run/std PackageArtifact records (the same
  // canonical records the Router/Runtime load at runtime).
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
      '--environment',
      environment,
      '--platform-source-root',
      skiffRoot,
    ],
    { cwd: skiffRoot },
  );
  const packageReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'package',
    action: 'build',
    root: sourceRoot,
    artifactRoot,
    environment,
  });
  const deploymentRef = packageReceipt?.serviceDeploymentReceipt?.deployment;
  if (
    deploymentRef === null
    || typeof deploymentRef !== 'object'
    || typeof deploymentRef.serviceId !== 'string'
    || typeof deploymentRef.contractVersion !== 'string'
  ) {
    throw new Error('actor live package build returned no exact ServiceDeployment reference');
  }
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    environment,
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
    throw new Error('actor live assembly build returned no exact RuntimeAssembly receipt');
  }
  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    environment,
    profile: 'dev',
    assemblyRecord: recordPath,
    sources: [{ root: sourceRoot, deployment: deploymentRef }],
  });
  const snapshotId =
    snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof snapshotId !== 'string') {
    throw new Error('actor live config snapshot production returned no exact snapshot reference');
  }
  return {
    assemblyIdentity: assembly.assemblyIdentity,
    configSnapshotId: snapshotId,
    deployment: deploymentRef,
  };
}

/// Reads the single service deployment record published by the compiler and
/// returns the exact `ServiceDeploymentRef` plus the per-gateway-entry
/// identities the probe needs to build canonical `request.start` frames.
export async function loadActorLiveDeploymentRecord(artifactRoot) {
  const deploymentRoot = join(artifactRoot, 'records', 'service-deployments');
  const files = [];
  await collectJsonFiles(deploymentRoot, files);
  if (files.length !== 1) {
    throw new Error(
      `actor live artifact must publish exactly one service deployment, got ${files.length}`,
    );
  }
  const record = JSON.parse(await readFile(files[0], 'utf8'));
  const contract = record?.contract;
  if (
    contract === null
    || typeof contract !== 'object'
    || typeof contract.serviceId !== 'string'
    || typeof contract.contractVersion !== 'string'
    || typeof record.deploymentRevision !== 'string'
    || typeof record.deploymentArtifactIdentity !== 'string'
    || record.gatewayEntries === null
    || typeof record.gatewayEntries !== 'object'
  ) {
    throw new Error('actor live service deployment record is missing required fields');
  }
  const gatewayEntries = {};
  for (const [key, entry] of Object.entries(record.gatewayEntries)) {
    if (typeof entry?.gatewayEntryIdentity !== 'string') {
      throw new Error(`actor live gateway entry ${key} has no gatewayEntryIdentity`);
    }
    gatewayEntries[key] = entry.gatewayEntryIdentity;
  }
  return {
    deployment: {
      serviceId: contract.serviceId,
      contractVersion: contract.contractVersion,
      deploymentRevision: record.deploymentRevision,
      deploymentArtifactIdentity: record.deploymentArtifactIdentity,
    },
    gatewayEntries,
  };
}

async function collectJsonFiles(directory, output) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return;
    }
    throw error;
  }
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await collectJsonFiles(path, output);
    } else if (entry.isFile() && entry.name.endsWith('.json')) {
      output.push(path);
    }
  }
}
