import { access, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
  runStdSeedAuthoring,
} from './package-service-authoring.mjs';
import { captureCheckedCommand } from './command-execution.mjs';
import {
  loadStackConfig,
  parseStackYaml,
  posixShellQuote,
  requireRemoteStackConfig,
} from './stack-config.mjs';
import { renderEcosystemConfig } from './stack-deploy.mjs';
import { createStackShell } from './stack-shell.mjs';

const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTIVATION_STATE_SCHEMA_VERSION = 'skiff-profile-activation-state-v1';
const ACTIVATION_STATE_DATABASE = 'skiff-router';
const ACTIVATION_STATE_COLLECTION = 'activation_state';

export async function initStack({
  configDir,
  skiffRoot,
  shell = createStackShell({ skiffRoot }),
  authoring = {
    runCompilerAuthoring,
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

    const assemblyReceipt = await authoring.runCompilerAuthoring({
      skiffRoot,
      kind: 'assembly',
      action: 'build',
      artifactRoot,
      profile,
      rootDeployments: [],
    });
    const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
    const assemblyIdentity = assemblyReceipt?.runtimeAssemblyReceipt?.assembly?.assemblyIdentity;
    if (typeof recordPath !== 'string' || typeof assemblyIdentity !== 'string') {
      throw new Error('compiler assembly build returned no exact RuntimeAssembly receipt');
    }

    const snapshotReceipt = await authoring.runConfigSnapshotAuthoring({
      skiffRoot,
      artifactRoot,
      profile,
      assemblyRecord: recordPath,
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
      `mkdir -p ${remoteSkiff}/{artifacts,bin,config,logs,telemetry,scripts,runtime-home}`,
    );
    await shell.rsync(
      `${artifactRoot}${path.sep}`,
      `${host}:${remoteSkiff}/artifacts/`,
      ['--delete'],
    );

    const ecosystemPath = path.join(tempRoot, 'ecosystem.config.cjs');
    await writeFile(ecosystemPath, renderEcosystemConfig({ remoteSkiff, nodeBin }));
    await shell.rsync(ecosystemPath, `${host}:${remoteSkiff}/ecosystem.config.cjs`);

    const stateDocument = {
      _id: profile,
      revision: 0,
      state: {
        schemaVersion: ACTIVATION_STATE_SCHEMA_VERSION,
        profile,
        committed: {
          generation: 0,
          assembly: { assemblyIdentity },
          configSnapshot: { snapshotId: configSnapshotId },
        },
        pending: null,
      },
    };
    const evalScript = [
      `db.getSiblingDB(${JSON.stringify(ACTIVATION_STATE_DATABASE)})`,
      `.getCollection(${JSON.stringify(ACTIVATION_STATE_COLLECTION)})`,
      `.insertOne(${JSON.stringify(stateDocument)});`,
    ].join('');
    await shell.remoteRun(
      host,
      `mongosh ${posixShellQuote(serviceDbMongoUrl)} --quiet --eval ${posixShellQuote(evalScript)}`,
    );

    const pm2 = `PATH=${nodeBin}:$PATH pm2`;
    await shell.remoteRun(host, `cd ${remoteSkiff} && ${pm2} delete skiff-router || true`);
    await shell.remoteRun(
      host,
      `cd ${remoteSkiff} && ${pm2} startOrReload ecosystem.config.cjs --only skiff-router --update-env`,
    );
    await shell.remoteRun(host, `${pm2} save`);

    return {
      profile,
      generation: 0,
      assemblyIdentity,
      configSnapshotId,
      std: stdReceipt,
      actorRoutingProjection: ACTOR_ROUTING_PROJECTION_RECORD_PATH,
      remoteSkiff,
      artifacts: `${remoteSkiff}/artifacts`,
      activationState: `${ACTIVATION_STATE_DATABASE}.${ACTIVATION_STATE_COLLECTION}`,
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

  const assemblyReceipt = await authoring.runCompilerAuthoring({
    skiffRoot,
    kind: 'assembly',
    action: 'build',
    artifactRoot,
    profile,
    rootDeployments: [],
  });
  const recordPath = assemblyReceipt?.runtimeAssemblyReceipt?.recordPath;
  const assemblyIdentity = assemblyReceipt?.runtimeAssemblyReceipt?.assembly?.assemblyIdentity;
  if (typeof recordPath !== 'string' || typeof assemblyIdentity !== 'string') {
    throw new Error('compiler assembly build returned no exact RuntimeAssembly receipt');
  }
  const snapshotReceipt = await authoring.runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    profile,
    assemblyRecord: recordPath,
    sources: [],
  });
  const configSnapshotId = snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof configSnapshotId !== 'string') {
    throw new Error('config snapshot production returned no exact snapshot reference');
  }
  await authoring.runStdSeedAuthoring({ skiffRoot, artifactRoot });
  await access(path.join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH));

  const stateDocument = {
    _id: profile,
    revision: 0,
    state: {
      schemaVersion: ACTIVATION_STATE_SCHEMA_VERSION,
      profile,
      committed: {
        generation: 0,
        assembly: { assemblyIdentity },
        configSnapshot: { snapshotId: configSnapshotId },
      },
      pending: null,
    },
  };
  const evalScript = [
    `db.getSiblingDB(${JSON.stringify(ACTIVATION_STATE_DATABASE)})`,
    `.getCollection(${JSON.stringify(ACTIVATION_STATE_COLLECTION)})`,
    `.replaceOne({_id: ${JSON.stringify(profile)}}, ${JSON.stringify(stateDocument)}, {upsert: true});`,
  ].join('');
  await captureLocalMongo(evalScript, serviceDbMongoUrl);

  return {
    profile,
    generation: 0,
    assemblyIdentity,
    configSnapshotId,
    mode: 'local',
    artifactRoot,
  };
}

async function captureLocalMongo(evalScript, mongoUrl) {
  await captureCheckedCommand(
    'mongosh',
    [mongoUrl, '--quiet', '--eval', evalScript],
    { cwd: process.cwd() },
  );
}
