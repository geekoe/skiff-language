#!/usr/bin/env node
// `router-live:chat` local gate harness (E-chat, plan §7/§8).
//
// The real gate is owned by the private `internals` repository trusted
// workflow; this script is the local equivalent that uses the same manifest
// schema and the same command (`npm run e2e:chat-smoke`). It:
//   - pins the three repositories (Skiff, internals, skiff-packages) and
//     builds the real agine stack through the real compiler into a temporary
//     artifact root (std + internals packages + skiff-packages http-session/
//     track + agine.ai/api + agine.ai/aihub + agine.ai/codex-relay);
//   - records the service artifact manifest (assembly, config snapshot, every
//     service deployment artifact and package artifact identity);
//   - starts an isolated temporary Mongo replica set, builds the explicit
//     `skiff-router` Rust binary and the explicit `runtime` Rust binary,
//     seeds the committed activation state, spawns both real processes and
//     loads the pinned manifest artifacts (release mode);
//   - starts a local ingress mapping 127.0.0.1 -> agine.ai/api 0.1.0 and runs
//     `npm run e2e:chat-smoke` in internals/agine pointed at that ingress;
//   - requires PASS and writes the manifest evidence record.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4004-4007 ports. Router/ingress ports are leased in 45000-45999 and
// the temporary mongod uses the repository's activation-state convention.

import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  access,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { ActivationStateMongoHarness } from './lib/activation-state-live-harness.mjs';
import { cargoTargetDir } from './lib/cargo-target-dir.mjs';
import {
  captureCheckedCommand,
  runAttachedCommand,
} from './lib/command-execution.mjs';
import {
  assertPortsClosed,
  leaseConsecutiveLocalPorts,
} from './lib/local-port-lease.mjs';
import {
  runCompilerAuthoring,
  runConfigSnapshotAuthoring,
} from './lib/package-service-authoring.mjs';
import {
  routerChatLiveManifestSchemaVersion,
  validateRouterChatLiveManifest,
} from './lib/router-chat-live-manifest.mjs';
import { ensureLocalServiceDbKeyring } from './lib/service-db-keyring.mjs';
import { startLocalIngress } from './local-ingress.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const defaultInternalsRoot = resolve(repoRoot, '..', 'internals');
const defaultSkiffPackagesRoot = resolve(repoRoot, '..', 'skiff-packages');
const ENVIRONMENT = 'router-live-chat';
const GENERATION = 1;
const ACTIVATION_DATABASE = 'skiff-router';
const ACTIVATION_COLLECTION = 'activation_state';
const REPLICA_ID = 'skiff-runtime-live-chat-replica';
const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';
const SMOKE_COMMAND = 'npm run e2e:chat-smoke';
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);

const internalsRoot = resolve(
  process.env.SKIFF_ROUTER_CHAT_LIVE_INTERNALS_ROOT || defaultInternalsRoot,
);
const skiffPackagesRoot = resolve(
  process.env.SKIFF_ROUTER_CHAT_LIVE_SKIFF_PACKAGES_ROOT || defaultSkiffPackagesRoot,
);
const agineRoot = resolve(
  process.env.SKIFF_ROUTER_CHAT_LIVE_AGINE_ROOT || join(internalsRoot, 'agine'),
);
const aihubServiceRoot = resolve(
  process.env.SKIFF_ROUTER_CHAT_LIVE_AIHUB_SERVICE_ROOT
    || join(internalsRoot, 'aihub', 'service'),
);
const codexRelayServiceRoot = resolve(
  process.env.SKIFF_ROUTER_CHAT_LIVE_CODEX_RELAY_SERVICE_ROOT
    || join(internalsRoot, 'codex-relay', 'service'),
);

const BUILD_ROOTS = [
  join(internalsRoot, 'packages', 'agent'),
  join(internalsRoot, 'packages', 'llm-api'),
  join(internalsRoot, 'packages', 'llm-providers'),
  join(skiffPackagesRoot, 'http-session'),
  join(skiffPackagesRoot, 'track'),
  join(internalsRoot, 'agine', 'service'),
  aihubServiceRoot,
  codexRelayServiceRoot,
];

const PREFLIGHT = process.argv.includes('--preflight');

let harness;
let portLease;
let ingressServer;
let children = [];
let logFiles = [];
let tempRoot;
let manifestBase;
let smokeSecretPath;

async function main() {
try {
  if (PREFLIGHT) {
    await preflight();
    console.log('router-live:chat: preflight PASS');
    process.exitCode = 0;
    return;
  }
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-chat-live-'));
  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('router-live:chat: reading pinned repository commits');
  const skiffSha = await gitRevParse(repoRoot);
  const internalsSha = await gitRevParse(internalsRoot);
  const skiffPackagesSha = await gitRevParse(skiffPackagesRoot);
  const internalsDirty = await gitDirtyFingerprint(internalsRoot);
  console.log(`router-live:chat: skiff=${skiffSha} internals=${internalsSha} skiff-packages=${skiffPackagesSha}`);
  if (internalsDirty !== null) {
    console.log(`router-live:chat: internals working tree is dirty (${internalsDirty.paths.length} paths, diff sha256 ${internalsDirty.sha256}); smoke runs on that working tree without modifying internals`);
  }

  console.log('router-live:chat: seeding canonical std artifact');
  await captureCheckedCommand(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '--manifest-path',
      join(repoRoot, 'test-runner', 'Cargo.toml'),
      '--bin',
      'skiff-package-service-smoke-fixture',
      '--',
      '--bootstrap-only',
      '--artifact-root',
      artifactRoot,
      '--platform-source-root',
      repoRoot,
      '--environment',
      ENVIRONMENT,
    ],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: cargoTargetDir(repoRoot) } },
  );

  console.log('router-live:chat: authoring real agine stack artifacts');
  const {
    packageArtifactReceipts,
    deployments,
    serviceSources,
    assemblyIdentity,
    configSnapshotId,
    assemblyRecordPath,
  } = await authorAgineStack({ repoRoot, artifactRoot, environment: ENVIRONMENT });

  const projectionDirectory = join(artifactRoot, 'records', 'actor-routing');
  await mkdir(projectionDirectory, { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
  );

  console.log('router-live:chat: leasing isolated router + ingress ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 3,
  });
  portLease = { release };
  const [httpPort, runtimePort, ingressPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log('router-live:chat: starting isolated Mongo replica set');
  harness = await ActivationStateMongoHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:chat: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  console.log('router-live:chat: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const routerBin = join(targetDir, 'debug', 'skiff-router');
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  await Promise.all([access(routerBin), access(runtimeBin)]);

  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });
  await writeFile(join(runtimeHome, 'runtime-id'), `${REPLICA_ID}\n`);

  console.log('router-live:chat: provisioning service-db keyring and configs');
  const keyringPath = join(tempRoot, 'service-db-keyring.json');
  await ensureLocalServiceDbKeyring(keyringPath);
  const routerConfigPath = join(tempRoot, 'router.yml');
  const runtimeConfigPath = join(tempRoot, 'runtime.yml');
  await writeFile(routerConfigPath, renderRouterConfig({
    environment: ENVIRONMENT,
    artifactRoot,
    httpPort,
    runtimePort,
    mongoUrl: harness.mongoUrl,
  }));
  await writeFile(runtimeConfigPath, renderRuntimeConfig({
    runtimePort,
    runtimeHome,
    keyringPath,
  }));

  console.log('router-live:chat: seeding committed activation state');
  await seedCommittedActivationState({
    mongoPort: harness.port,
    environment: ENVIRONMENT,
    generation: GENERATION,
    assemblyIdentity,
    configSnapshotId,
  });

  console.log(`router-live:chat: spawning real Rust Router (http=${httpPort}, runtime=${runtimePort})`);
  const router = await spawnManaged(
    'router',
    routerBin,
    [routerConfigPath],
    { cwd: repoRoot, tempRoot },
  );
  children.push(router);
  const runtime = await spawnManaged(
    'runtime',
    runtimeBin,
    [runtimeConfigPath],
    { cwd: repoRoot, tempRoot },
  );
  children.push(runtime);

  console.log('router-live:chat: waiting for isolated Router/Runtime readiness');
  await waitForChatLiveReady({
    controlUrl: `http://127.0.0.1:${runtimePort}`,
    environment: ENVIRONMENT,
    generation: GENERATION,
    assemblyIdentity,
    configSnapshotId,
    children,
  });

  console.log('router-live:chat: starting isolated local ingress');
  const ingressConfig = {
    listen: { host: '127.0.0.1', port: ingressPort },
    upstream: { host: '127.0.0.1', port: httpPort },
    hosts: new Map([
      ['127.0.0.1', { service: 'agine.ai/api', version: '0.1.0' }],
    ]),
  };
  ingressServer = await startLocalIngress(ingressConfig);

  smokeSecretPath = join(tempRoot, 'chat-smoke-secret.yml');
  await writeSmokeSecret({
    secretPath: smokeSecretPath,
    aihubServiceRoot,
    env: process.env,
  });

  manifestBase = {
    schemaVersion: routerChatLiveManifestSchemaVersion(),
    pinned: {
      skiff: { repository: 'skiff', commit: skiffSha },
      internals: { repository: 'internals', commit: internalsSha },
      skiffPackages: { repository: 'skiff-packages', commit: skiffPackagesSha },
    },
    environment: ENVIRONMENT,
    generation: GENERATION,
    assembly: { assemblyIdentity },
    configSnapshot: { snapshotId: configSnapshotId },
    services: deployments.map((deployment) => {
      const implementation = packageArtifactReceipts.find(
        (receipt) => receipt?.artifact?.packageId === deployment.serviceId,
      );
      if (implementation?.artifact?.packageBuildId === undefined) {
        throw new Error(
          `service ${deployment.serviceId} has no implementation package receipt`,
        );
      }
      return {
        serviceId: deployment.serviceId,
        contractVersion: deployment.contractVersion,
        deploymentRevision: deployment.deploymentRevision,
        deploymentArtifactIdentity: deployment.deploymentArtifactIdentity,
        implementationPackageBuildId: implementation.artifact.packageBuildId,
      };
    }),
    packages: packageArtifactReceipts
      .map((receipt) => receipt?.artifact)
      .filter((artifact) => artifact !== undefined),
    smoke: {
      command: SMOKE_COMMAND,
      cwd: agineRoot,
      ingressBase: `http://127.0.0.1:${ingressPort}`,
      status: 'PASS',
      finishedAt: new Date().toISOString(),
    },
  };
  validateRouterChatLiveManifest(manifestBase);

  console.log(`router-live:chat: running ${SMOKE_COMMAND} in ${agineRoot}`);
  await runAttachedCommand('npm', ['run', 'e2e:chat-smoke'], {
    cwd: agineRoot,
    env: {
      ...process.env,
      AGINE_E2E_INGRESS_HTTP_BASE: manifestBase.smoke.ingressBase,
      AGINE_E2E_PROVIDER_SECRET_CONFIG: smokeSecretPath,
      AGINE_E2E_PROVIDER_ID: 'aihub',
      AGINE_E2E_MODEL_ID: 'deepseek-v4-flash',
    },
  });
  console.log('router-live:chat: chat smoke PASS');

  const manifestPath = resolve(
    process.env.SKIFF_ROUTER_CHAT_LIVE_MANIFEST_OUT || join(tempRoot, 'router-chat-live-manifest.json'),
  );
  await writeFile(manifestPath, `${JSON.stringify(manifestBase, null, 2)}\n`);
  console.log(`router-live:chat: manifest written to ${manifestPath}`);
  console.log(evidenceSummary({
    skiffSha,
    internalsSha,
    skiffPackagesSha,
    assemblyIdentity,
    configSnapshotId,
    manifestPath,
  }));
  console.log('router-live:chat: PASS');
} catch (error) {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  if (tempRoot !== undefined) {
    await dumpManagedLogs(tempRoot);
  }
  throw error;
} finally {
  const errors = [];
  for (const child of children.reverse()) {
    await settleCleanupStep(errors, `stop ${child.label}`, () => stopManagedChild(child));
  }
  for (const logFile of logFiles) {
    await settleCleanupStep(errors, `close ${logFile.path}`, async () => {
      await logFile.handle.close();
    });
  }
  if (ingressServer !== undefined) {
    await settleCleanupStep(errors, 'close local ingress', async () => {
      await new Promise((resolvePromise) => ingressServer.close(resolvePromise));
    });
  }
  if (harness !== undefined) {
    try {
      await harness.cleanup();
    } catch (error) {
      errors.push(error);
    }
  }
  if (portLease !== undefined) {
    try {
      await assertPortsClosed(portLease.ports);
    } catch (error) {
      errors.push(error);
    }
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
    throw new AggregateError(errors, 'router-live:chat cleanup failed');
  }
}
}

main().catch((error) => {
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  if (error instanceof AggregateError) {
    for (const detail of error.errors) {
      process.stderr.write(`cleanup detail: ${detail?.stack || detail}\n`);
    }
  }
  process.stderr.write(`${error?.stack || error}\n`);
  process.exitCode = 1;
});

async function authorAgineStack({
  repoRoot: skiffRoot,
  artifactRoot,
  environment,
}) {
  const packageArtifactReceipts = [];
  const deployments = [];
  const serviceSources = [];
  const pending = [...BUILD_ROOTS];
  const lastErrors = new Map();
  while (pending.length > 0) {
    const deferred = [];
    let progressed = false;
    for (const root of pending) {
      try {
        const receipt = await runCompilerAuthoring({
          skiffRoot,
          kind: 'package',
          action: 'publish',
          root,
          artifactRoot,
          environment,
        });
        packageArtifactReceipts.push(receipt.packageArtifactReceipt);
        if (receipt.serviceDeploymentReceipt?.deployment !== undefined) {
          deployments.push(receipt.serviceDeploymentReceipt.deployment);
          serviceSources.push({
            root,
            deployment: receipt.serviceDeploymentReceipt.deployment,
          });
        }
        progressed = true;
      } catch (error) {
        lastErrors.set(root, error);
        if (isUnpublishedExactDependency(error)) {
          deferred.push(root);
          continue;
        }
        throw error;
      }
    }
    if (!progressed) {
      const details = deferred
        .map((root) => `${root}: ${errorMessage(lastErrors.get(root) ?? new Error('unknown'))}`)
        .join('\n');
      throw new Error(
        `router-live:chat could not close exact package/service dependencies:\n${details}`,
      );
    }
    pending.splice(0, pending.length, ...deferred);
  }

  const rootDeployments = deployments;
  const assemblyReceipt = await runCompilerAuthoring({
    skiffRoot,
    kind: 'assembly',
    action: 'build',
    rootDeployments,
    artifactRoot,
    environment,
  });
  const runtimeAssembly = assemblyReceipt?.runtimeAssemblyReceipt;
  const assembly = runtimeAssembly?.assembly;
  const assemblyIdentity = assembly?.assemblyIdentity;
  const assemblyRecordPath = runtimeAssembly?.recordPath;
  if (typeof assemblyIdentity !== 'string' || typeof assemblyRecordPath !== 'string') {
    throw new Error('compiler assembly build returned no exact RuntimeAssembly receipt');
  }

  const snapshotReceipt = await runConfigSnapshotAuthoring({
    skiffRoot,
    artifactRoot,
    environment,
    profile: 'dev',
    assemblyRecord: assemblyRecordPath,
    sources: serviceSources,
  });
  const configSnapshotId =
    snapshotReceipt?.runtimeConfigSnapshotReceipt?.snapshot?.snapshotId;
  if (typeof configSnapshotId !== 'string') {
    throw new Error('config snapshot production returned no exact snapshot reference');
  }

  return {
    packageArtifactReceipts,
    deployments,
    serviceSources,
    assemblyIdentity,
    configSnapshotId,
    assemblyRecordPath,
  };
}

function isUnpublishedExactDependency(error) {
  return /has no published (?:PackageArtifact|ServiceContract) pointer/.test(
    errorMessage(error),
  );
}

function errorMessage(error) {
  return error?.message || String(error);
}

function renderRouterConfig({
  environment,
  artifactRoot,
  httpPort,
  runtimePort,
  mongoUrl,
}) {
  return [
    'profile: dev',
    `environment: ${yamlQuote(environment)}`,
    'host: 127.0.0.1',
    `artifactsPath: ${yamlQuote(artifactRoot)}`,
    'releaseMode: true',
    'requestTimeoutMs: 60000',
    'http:',
    `  port: ${httpPort}`,
    '  maxRequestBytes: 67108864',
    '  maxResponseBytes: 8388608',
    'runtime:',
    `  port: ${runtimePort}`,
    '  path: /runtime',
    '  maxConcurrency: 128',
    'websocket:',
    '  path: /ws',
    'serviceDb:',
    `  mongoUrl: ${yamlQuote(mongoUrl)}`,
    '',
  ].join('\n');
}

function renderRuntimeConfig({
  runtimePort,
  runtimeHome,
  keyringPath,
}) {
  return [
    `router: ${yamlQuote(`ws://127.0.0.1:${runtimePort}/runtime`)}`,
    `runtime-home: ${yamlQuote(runtimeHome)}`,
    `environment: ${yamlQuote(ENVIRONMENT)}`,
    'serviceDb:',
    '  encryption:',
    `    keyringFile: ${yamlQuote(keyringPath)}`,
    '',
  ].join('\n');
}

async function seedCommittedActivationState({
  mongoPort,
  environment,
  generation,
  assemblyIdentity,
  configSnapshotId,
}) {
  const state = {
    schemaVersion: 'skiff-environment-activation-state-v2',
    environment,
    committed: {
      generation,
      assembly: { assemblyIdentity },
      configSnapshot: { snapshotId: configSnapshotId },
    },
    pending: null,
  };
  const document = { _id: environment, state };
  const url =
    `mongodb://127.0.0.1:${mongoPort}/${ACTIVATION_DATABASE}?directConnection=true&replicaSet=rs0`;
  const script = [
    `db.${ACTIVATION_COLLECTION}.deleteMany({ _id: ${JSON.stringify(environment)} });`,
    `db.${ACTIVATION_COLLECTION}.insertOne(${JSON.stringify(document)});`,
  ].join('');
  await captureCheckedCommand(
    'mongosh',
    [url, '--quiet', '--eval', script],
    { cwd: repoRoot },
  );
}

async function writeSmokeSecret({ secretPath, aihubServiceRoot, env }) {
  const apiKey = env.SKIFF_ROUTER_CHAT_LIVE_AIHUB_API_KEY
    || await readDeepseekApiKey(aihubServiceRoot);
  if (typeof apiKey !== 'string' || apiKey.trim().length === 0) {
    throw new Error(
      'router-live:chat requires an aihub deepseek API key: set '
      + 'SKIFF_ROUTER_CHAT_LIVE_AIHUB_API_KEY or provide '
      + `${join(aihubServiceRoot, 'config.dev.secret.yml')} with a deepseek.apiKey`,
    );
  }
  const contents = [
    'service:',
    '  aihub:',
    `    apiKey: ${JSON.stringify(apiKey.trim())}`,
    '',
  ].join('\n');
  await writeFile(secretPath, contents, { encoding: 'utf8', mode: 0o600 });
}

async function readDeepseekApiKey(aihubServiceRoot) {
  const secretPath = join(aihubServiceRoot, 'config.dev.secret.yml');
  let source;
  try {
    source = await readFile(secretPath, 'utf8');
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  }
  let inDeepseek = false;
  for (const rawLine of source.split(/\r?\n/)) {
    if (/^\s*(?:#.*)?$/.test(rawLine)) continue;
    const indent = rawLine.match(/^\s*/)?.[0].length || 0;
    const line = rawLine.trim();
    if (line === 'deepseek:') {
      inDeepseek = true;
      continue;
    }
    if (!inDeepseek) continue;
    if (/^[A-Za-z0-9._-]+:$/.test(line) && !line.startsWith('apiKey:')) break;
    const match = /^apiKey:\s*(.*)$/.exec(line);
    if (!match) continue;
    const value = match[1].trim();
    if (value.startsWith('"')) return JSON.parse(value);
    if (value.startsWith("'") && value.endsWith("'")) {
      return value.slice(1, -1).replaceAll("''", "'");
    }
    return value.replace(/\s+#.*$/, '').trim();
  }
  return null;
}

async function spawnManaged(label, command, args, { cwd, tempRoot }) {
  const stdoutPath = join(tempRoot, `${label}.stdout.log`);
  const stderrPath = join(tempRoot, `${label}.stderr.log`);
  const stdout = await open(stdoutPath, 'w');
  const stderr = await open(stderrPath, 'w');
  logFiles.push({ path: stdoutPath, handle: stdout });
  logFiles.push({ path: stderrPath, handle: stderr });
  const child = spawn(command, args, {
    cwd,
    env: process.env,
    stdio: ['ignore', stdout.fd, stderr.fd],
  });
  const exited = new Promise((resolvePromise, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => resolvePromise({ code, signal }));
  });
  return { label, child, exited };
}

async function stopManagedChild(managed) {
  if (managed.child.exitCode !== null || managed.child.signalCode !== null) {
    return;
  }
  managed.child.kill('SIGTERM');
  const outcome = await Promise.race([
    managed.exited.then(() => true),
    delay(20_000).then(() => false),
  ]);
  if (outcome) return;
  managed.child.kill('SIGKILL');
  await managed.exited;
}

async function waitForChatLiveReady({
  controlUrl,
  environment,
  generation,
  assemblyIdentity,
  configSnapshotId,
  children,
}) {
  const startedAt = Date.now();
  let lastError;
  while (Date.now() - startedAt < 120_000) {
    for (const managed of children) {
      if (managed.child.exitCode !== null || managed.child.signalCode !== null) {
        throw new Error(
          `${managed.label} exited before readiness with ${managed.child.signalCode ?? managed.child.exitCode}`,
        );
      }
    }
    try {
      const response = await fetch(`${controlUrl}/__router/health`);
      if (response.ok) {
        const health = await response.json();
        const active = health?.activeAssembly;
        const replicas = Array.isArray(health?.replicas) ? health.replicas : [];
        const connections = Array.isArray(health?.capabilityConnections)
          ? health.capabilityConnections
          : [];
        const epochMatches = active?.environment === environment
          && active?.generation === generation
          && active?.assemblyIdentity === assemblyIdentity
          && active?.configSnapshotId === configSnapshotId;
        const replica = replicas.find((candidate) =>
          candidate?.connected === true
          && candidate?.state === 'healthy'
          && candidate?.environment === environment
          && candidate?.generation === generation
          && candidate?.assemblyIdentity === assemblyIdentity);
        if (
          epochMatches
          && replica !== undefined
          && connections.some((connection) =>
            connection?.connected === true
            && connection?.runtimeId === replica.replicaId)
        ) {
          return;
        }
      }
    } catch (error) {
      lastError = error;
    }
    await delay(100);
  }
  throw new Error(
    `isolated Router/Runtime did not become ready at ${controlUrl} with assembly ${assemblyIdentity}`,
  );
}

async function gitRevParse(root) {
  const outcome = await captureCheckedCommand('git', ['rev-parse', 'HEAD'], { cwd: root });
  const sha = outcome.stdout.trim();
  if (!/^[0-9a-f]{40}$/.test(sha)) {
    throw new Error(`${root} is not a git checkout with a full commit SHA`);
  }
  return sha;
}

async function gitDirtyFingerprint(root) {
  const status = await captureCheckedCommand(
    'git',
    ['status', '--porcelain'],
    { cwd: root },
  );
  const paths = status.stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => line.slice(3));
  if (paths.length === 0) {
    return null;
  }
  const diff = await captureCheckedCommand(
    'git',
    ['diff', '--binary', 'HEAD'],
    { cwd: root },
  );
  const sha256 = createHash('sha256').update(diff.stdout).digest('hex');
  return { paths, sha256 };
}

async function dumpManagedLogs(tempRoot) {
  const logPaths = [];
  try {
    const entries = await readdir(tempRoot);
    for (const entry of entries) {
      if (/^(router|runtime)\.(stdout|stderr)\.log$/.test(entry)) {
        logPaths.push(join(tempRoot, entry));
      }
    }
  } catch {
    // temp root may be gone
  }
  for (const logPath of logPaths.sort()) {
    try {
      const contents = await readFile(logPath, 'utf8');
      if (contents.trim().length > 0) {
        process.stderr.write(`\n===== ${logPath} =====\n${contents.slice(-8000)}\n`);
      }
    } catch {
      // log file may not exist yet
    }
  }
}

function evidenceSummary({
  skiffSha,
  internalsSha,
  skiffPackagesSha,
  assemblyIdentity,
  configSnapshotId,
  manifestPath,
}) {
  return [
    'router-live:chat evidence:',
    `  skiff: ${skiffSha}`,
    `  internals: ${internalsSha}`,
    `  skiff-packages: ${skiffPackagesSha}`,
    `  assembly: ${assemblyIdentity}`,
    `  configSnapshot: ${configSnapshotId}`,
    `  manifest: ${manifestPath}`,
  ].join('\n');
}

async function preflight() {
  for (const [label, root] of [
    ['internals', internalsRoot],
    ['skiff-packages', skiffPackagesRoot],
    ['agine', agineRoot],
    ['aihub service', aihubServiceRoot],
    ['codex-relay service', codexRelayServiceRoot],
    ...BUILD_ROOTS.map((root) => ['build root', root]),
  ]) {
    try {
      const metadata = await stat(root);
      if (!metadata.isDirectory()) {
        throw new Error(`${root} is not a directory`);
      }
    } catch (error) {
      throw new Error(`router-live:chat preflight: ${label} root missing: ${root} (${errorMessage(error)})`);
    }
  }
  const skiffSha = await gitRevParse(repoRoot);
  const internalsSha = await gitRevParse(internalsRoot);
  const skiffPackagesSha = await gitRevParse(skiffPackagesRoot);
  const apiKey = process.env.SKIFF_ROUTER_CHAT_LIVE_AIHUB_API_KEY
    || await readDeepseekApiKey(aihubServiceRoot);
  if (typeof apiKey !== 'string' || apiKey.trim().length === 0) {
    throw new Error(
      'router-live:chat preflight: no aihub deepseek apiKey; set '
      + 'SKIFF_ROUTER_CHAT_LIVE_AIHUB_API_KEY or provide the aihub secret config',
    );
  }
  for (const executable of ['cargo', 'node', 'npm', 'mongod', 'mongosh', 'git']) {
    const outcome = await captureCheckedCommand(
      executable === 'git' ? 'which' : 'which',
      [executable],
      { cwd: repoRoot },
    );
    if (outcome.stdout.trim().length === 0) {
      throw new Error(`router-live:chat preflight: ${executable} is not on PATH`);
    }
  }
  console.log(`router-live:chat preflight: skiff=${skiffSha} internals=${internalsSha} skiff-packages=${skiffPackagesSha}`);
}

function yamlQuote(value) {
  return JSON.stringify(value);
}

function assertNotForbidden(port) {
  if (FORBIDDEN_PORTS.has(port)) {
    throw new Error(`leased port ${port} is a forbidden stable port`);
  }
}

async function settleCleanupStep(errors, step, operation) {
  try {
    await operation();
  } catch (error) {
    errors.push(new Error(`${step}: ${errorMessage(error)}`, { cause: error }));
  }
}

function range(start, end) {
  const values = [];
  for (let value = start; value <= end; value += 1) {
    values.push(value);
  }
  return values;
}
