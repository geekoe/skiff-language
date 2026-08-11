import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import {
  runConfigSnapshotAuthoring,
  runStdSeedAuthoring,
} from './package-service-authoring.mjs';
import {
  loadStackConfig,
  parseStackYaml,
  requireRemoteStackConfig,
} from './stack-config.mjs';
import { renderEcosystemConfig } from './stack-deploy.mjs';
import { createStackShell } from './stack-shell.mjs';

const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';

export async function initStack({
  configDir,
  skiffRoot,
  shell = createStackShell({ skiffRoot }),
  authoring = {
    runConfigSnapshotAuthoring,
    runStdSeedAuthoring,
  },
}) {
  const stack = await loadStackConfig(configDir, { skiffRoot });
  if (stack.config.remote === undefined) {
    return initLocalStack({ stack, skiffRoot, authoring });
  }
  requireRemoteStackConfig(stack, 'stack init');
  const profile = stack.config.profile;
  const { host, remoteSkiff, nodeBin } = stack.config.remote;
  const serviceDbMongoUrl = stack.router.serviceDb?.mongoUrl;
  if (typeof serviceDbMongoUrl !== 'string' || serviceDbMongoUrl.trim().length === 0) {
    throw new Error('router.yml serviceDb.mongoUrl is required for stack init');
  }

  const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'skiff-stack-init-'));
  try {
    const artifactRoot = path.join(tempRoot, 'artifacts');

    const snapshotReceipt = await authoring.runConfigSnapshotAuthoring({
      skiffRoot,
      artifactRoot,
      profile,
      sources: [],
    });
    const configSnapshotId = snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
    if (typeof configSnapshotId !== 'string') {
      throw new Error('config snapshot production returned no exact snapshot reference');
    }

    const stdReceipt = await authoring.runStdSeedAuthoring({ skiffRoot, artifactRoot });
    await access(path.join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH));

    await shell.remoteRun(
      host,
      `mkdir -p ${remoteSkiff}/{artifacts,bin,config,logs,scripts,runtime-home}`,
    );
    await shell.rsync(
      `${artifactRoot}${path.sep}`,
      `${host}:${remoteSkiff}/artifacts/`,
      ['--delete'],
    );

    const ecosystemPath = path.join(tempRoot, 'ecosystem.config.cjs');
    await writeFile(ecosystemPath, renderEcosystemConfig({ remoteSkiff, nodeBin }));
    await shell.rsync(ecosystemPath, `${host}:${remoteSkiff}/ecosystem.config.cjs`);

    const pm2 = `PATH=${nodeBin}:$PATH pm2`;
    await shell.remoteRun(host, `cd ${remoteSkiff} && ${pm2} delete skiff-router || true`);
    await shell.remoteRun(
      host,
      `cd ${remoteSkiff} && ${pm2} startOrReload ecosystem.config.cjs --only skiff-router --update-env`,
    );
    await shell.remoteRun(host, `${pm2} save`);

    return {
      profile,
      configSnapshotId,
      std: stdReceipt,
      actorRoutingProjection: ACTOR_ROUTING_PROJECTION_RECORD_PATH,
      remoteSkiff,
      artifacts: `${remoteSkiff}/artifacts`,
      ecosystem: `${remoteSkiff}/ecosystem.config.cjs`,
    };
  } finally {
    await rm(tempRoot, { recursive: true, force: true });
  }
}

async function initLocalStack({
  stack,
  skiffRoot,
  authoring,
}) {
  const profile = stack.config.profile;
  const instanceFile = path.join(stack.paths.buildRoot, 'instance.yml');
  let instance;
  try {
    instance = parseStackYaml(await readFile(instanceFile, 'utf8'), 'instance.yml');
  } catch (error) {
    throw new Error(
      `local stack init requires instance.yml at ${instanceFile}; run "skiff stack build --configDir <dir> --profile debug" first`,
      { cause: error },
    );
  }
  if (instance.schemaVersion !== 'skiff-instance-v1' || instance.profile !== profile) {
    throw new Error('instance.yml must be skiff-instance-v1 with the configDir profile');
  }
  const artifactRoot = instance.artifactRoot;
  const serviceDbMongoUrl = stack.router.serviceDb?.mongoUrl;
  if (typeof serviceDbMongoUrl !== 'string' || serviceDbMongoUrl.trim().length === 0) {
    throw new Error('router.yml serviceDb.mongoUrl is required for stack init');
  }
  await mkdir(artifactRoot, { recursive: true });

  const snapshotReceipt = await authoring.runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    profile,
    sources: [],
  });
  const configSnapshotId = snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof configSnapshotId !== 'string') {
    throw new Error('config snapshot production returned no exact snapshot reference');
  }
  await authoring.runStdSeedAuthoring({ skiffRoot, artifactRoot });
  await access(path.join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH));

  return {
    profile,
    configSnapshotId,
    mode: 'local',
    artifactRoot,
  };
}
