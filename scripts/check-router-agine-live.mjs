#!/usr/bin/env node
// `router-live:agine` Phase 0 combined managed Live selector (bytecode VM phases,
// phase-0-baseline-live.md §3.2/§3.3/§3.4 and phases/README.md §3.3 G3).
//
// Runs, in one isolated stack (temporary Mongo replica set, temporary artifact
// root authored through the real compiler, real Rust Router/Runtime binaries,
// dynamic ports 45000-45999, local ingress), in order:
//   1. `npm run e2e:chat-smoke` in internals/agine (existing canonical smoke);
//   2. `npm run e2e:host-tools -- --check` (host online/binding only, no chat);
//   3. strict full host-tools: `node client/e2e/host-tools.mjs` with an
//      explicit runtime PID, a read-only narrowed workspace (read-only copy of
//      the Skiff `doc` directory), an allowed-tools list without
//      `host.shell.run`, and an explicit sample file path.
//
// After the strict full host-tools run the harness mechanically asserts from
// its stdout: terminal completed, non-empty assistant reply, at least one
// allowed `host.file.*` tool call, zero `host.shell.run` calls, and a
// non-empty profiling sample file. Any failed assertion exits non-zero.
//
// Writes a provenance manifest pinning the three repositories (commit, tree
// hash, dirty status), the compiler/router/runtime binary absolute paths with
// SHA-256, artifact root, profile, ports, Mongo URL, every deployment/package
// identity, per-phase start/end timestamps and statuses, and the explicit
// `engine: "legacy-tree"` marker stating that Phase 0 Live verifies the tree
// evaluator, not the bytecode VM.
//
// The harness never touches the stable instance, stable Mongo, PM2, or the
// fixed 4000-4007 ports.

import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  access,
  chmod,
  cp,
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

import { MongodLiveHarness } from './lib/mongod-live-harness.mjs';
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
import { ensureLocalServiceDbKeyring } from './lib/service-db-keyring.mjs';
import { startLocalIngress } from './local-ingress.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const defaultInternalsRoot = resolve(repoRoot, '..', 'internals');
const defaultSkiffPackagesRoot = resolve(repoRoot, '..', 'skiff-packages');
const PROFILE = 'router-live-agine';
const REPLICA_ID = 'skiff-runtime-live-agine-replica';
const ACTOR_ROUTING_PROJECTION_RECORD_PATH = 'records/actor-routing/current.json';
const ACTOR_ROUTING_PROJECTION_CONTENT =
  '{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}';
const SMOKE_COMMAND = 'npm run e2e:chat-smoke';
const HOST_TOOLS_CHECK_COMMAND = 'npm run e2e:host-tools -- --check';
// Strict host-tools allowed tools: read-only file exploration only.
const ALLOWED_HOST_TOOLS = [
  'host.file.find',
  'host.file.search',
  'host.file.read',
];
const HOST_TOOLS_PROMPT =
  process.env.AGINE_HOST_TOOLS_PROMPT ||
  '请使用 host 文件工具浏览 implementation/bytecode-vm 目录下的文档，分析后回答：'
  + 'bytecode VM Phase 0 的 router-live:agine 组合 Live selector 的验收标准中，'
  + 'provenance manifest 必须 pin 哪些内容？';
const HOST_TOOLS_SYSTEM_PROMPT =
  process.env.AGINE_HOST_TOOLS_SYSTEM_PROMPT ||
  '你是资深后端架构师。回答前必须用 host 文件工具（find/search/read）实际浏览'
  + ' doc 目录中的文档，引用具体文件路径，用中文回答。';
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);

const internalsRoot = resolve(
  process.env.SKIFF_ROUTER_AGINE_LIVE_INTERNALS_ROOT || defaultInternalsRoot,
);
const skiffPackagesRoot = resolve(
  process.env.SKIFF_ROUTER_AGINE_LIVE_SKIFF_PACKAGES_ROOT || defaultSkiffPackagesRoot,
);
const agineRoot = resolve(
  process.env.SKIFF_ROUTER_AGINE_LIVE_AGINE_ROOT || join(internalsRoot, 'agine'),
);
const aihubServiceRoot = resolve(
  process.env.SKIFF_ROUTER_AGINE_LIVE_AIHUB_SERVICE_ROOT
    || join(internalsRoot, 'aihub', 'service'),
);
const codexRelayServiceRoot = resolve(
  process.env.SKIFF_ROUTER_AGINE_LIVE_CODEX_RELAY_SERVICE_ROOT
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
const KEEP_ON_FAILURE = process.env.SKIFF_ROUTER_AGINE_LIVE_KEEP_ON_FAILURE === '1';

let harness;
let portLease;
let ingressServer;
let children = [];
let logFiles = [];
let tempRoot;
let manifestBase;
let runFailed = false;

async function main() {
try {
  if (PREFLIGHT) {
    await preflight();
    console.log('router-live:agine: preflight PASS');
    process.exitCode = 0;
    return;
  }
  tempRoot = await mkdtemp(join(tmpdir(), 'skiff-router-agine-live-'));
  const artifactRoot = join(tempRoot, 'artifacts');
  await mkdir(artifactRoot, { recursive: true });

  console.log('router-live:agine: reading pinned repository commits');
  const pinned = await readPinnedRepositories();
  console.log(
    `router-live:agine: skiff=${pinned.skiff.commit} internals=${pinned.internals.commit} `
    + `skiff-packages=${pinned.skiffPackages.commit}`,
  );

  console.log('router-live:agine: seeding canonical std artifact');
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
      '--profile',
      PROFILE,
    ],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: cargoTargetDir(repoRoot) } },
  );

  console.log('router-live:agine: authoring real agine stack artifacts');
  const {
    packageArtifactReceipts,
    deployments,
    serviceSources,
    assemblyIdentity,
    configSnapshotId,
    assemblyRecordPath,
  } = await authorAgineStack({ repoRoot, artifactRoot, profile: PROFILE });

  const projectionDirectory = join(artifactRoot, 'records', 'actor-routing');
  await mkdir(projectionDirectory, { recursive: true });
  await writeFile(
    join(artifactRoot, ACTOR_ROUTING_PROJECTION_RECORD_PATH),
    ACTOR_ROUTING_PROJECTION_CONTENT,
  );

  console.log('router-live:agine: leasing isolated router + ingress ports');
  const { ports, release } = await leaseConsecutiveLocalPorts({
    rangeStart: 45000,
    rangeEnd: 45999,
    count: 3,
  });
  portLease = { ports, release };
  const [httpPort, runtimePort, ingressPort] = ports;
  for (const port of ports) {
    assertNotForbidden(port);
  }

  console.log('router-live:agine: starting isolated Mongo replica set');
  harness = await MongodLiveHarness.create({ repoRoot });
  await harness.start();

  const targetDir = cargoTargetDir(repoRoot);
  console.log('router-live:agine: building explicit Rust router binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'skiff-router', '--bin', 'skiff-router'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  console.log('router-live:agine: building explicit Rust runtime binary');
  await captureCheckedCommand(
    'cargo',
    ['build', '-p', 'runtime', '--bin', 'runtime'],
    { cwd: repoRoot, env: { ...process.env, CARGO_TARGET_DIR: targetDir } },
  );
  const routerBin = join(targetDir, 'debug', 'skiff-router');
  const runtimeBin = join(targetDir, 'debug', 'runtime');
  // The compiler binary is produced by the authoring `cargo run` above and is
  // the exact binary that authored every artifact in this stack.
  const compilerBin = join(targetDir, 'debug', 'skiff-compiler');
  await Promise.all([access(routerBin), access(runtimeBin), access(compilerBin)]);
  const binaries = {
    compiler: await sha256File(compilerBin),
    router: await sha256File(routerBin),
    runtime: await sha256File(runtimeBin),
  };

  const runtimeHome = join(tempRoot, 'runtime-home');
  await mkdir(runtimeHome, { recursive: true });
  await writeFile(join(runtimeHome, 'runtime-id'), `${REPLICA_ID}\n`);

  console.log('router-live:agine: provisioning service-db keyring and configs');
  const keyringPath = join(tempRoot, 'service-db-keyring.json');
  await ensureLocalServiceDbKeyring(keyringPath);
  const routerConfigPath = join(tempRoot, 'router.yml');
  const runtimeConfigPath = join(tempRoot, 'runtime.yml');
  await writeFile(routerConfigPath, renderRouterConfig({
    profile: PROFILE,
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

  console.log('router-live:agine: spawning real Rust Router (http=${httpPort}, runtime=${runtimePort})');
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
  const runtimePid = runtime.child.pid;

  console.log('router-live:agine: starting isolated local ingress');
  const ingressConfig = {
    listen: { host: '127.0.0.1', port: ingressPort },
    upstream: { host: '127.0.0.1', port: httpPort },
    hosts: new Map([
      ['127.0.0.1', { service: 'agine.ai/api', version: '0.1.0' }],
    ]),
  };
  ingressServer = await startLocalIngress(ingressConfig);

  console.log('router-live:agine: waiting for isolated Router/Runtime readiness');
  await waitForChatLiveReady({
    ingressBase: `http://127.0.0.1:${ingressPort}`,
    children,
  });

  await assertAihubDeepseekKeyConfigured(aihubServiceRoot);

  console.log('router-live:agine: preparing strict read-only host-tools workspace');
  const hostWorkspace = await prepareStrictHostWorkspace(tempRoot);

  manifestBase = {
    schemaVersion: agineLiveManifestSchemaVersion(),
    engine: 'legacy-tree',
    pinned,
    binaries,
    profile: PROFILE,
    artifactRoot,
    ports: { http: httpPort, runtime: runtimePort, ingress: ingressPort },
    mongo: { url: harness.mongoUrl },
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
    phases: {},
  };

  const ingressBase = `http://127.0.0.1:${ingressPort}`;
  const gatewayBase = `ws://127.0.0.1:${ingressPort}/ws`;

  console.log(`router-live:agine: phase 1/3 running ${SMOKE_COMMAND} in ${agineRoot}`);
  const chatSmoke = {
    command: SMOKE_COMMAND,
    cwd: agineRoot,
    ingressBase,
    startedAt: new Date().toISOString(),
  };
  await runAttachedCommand('npm', ['run', 'e2e:chat-smoke'], {
    cwd: agineRoot,
    env: {
      ...process.env,
      AGINE_E2E_INGRESS_HTTP_BASE: ingressBase,
      AGINE_E2E_PROVIDER_ID: 'aihub',
      AGINE_E2E_MODEL_ID: 'deepseek-v4-flash',
    },
  });
  chatSmoke.finishedAt = new Date().toISOString();
  chatSmoke.status = 'PASS';
  manifestBase.phases.chatSmoke = chatSmoke;
  console.log('router-live:agine: chat smoke PASS');

  console.log(`router-live:agine: phase 2/3 running ${HOST_TOOLS_CHECK_COMMAND} in ${agineRoot}`);
  const hostToolsCheck = {
    command: HOST_TOOLS_CHECK_COMMAND,
    cwd: agineRoot,
    ingressBase,
    startedAt: new Date().toISOString(),
  };
  await runAttachedCommand('npm', ['run', 'e2e:host-tools', '--', '--check'], {
    cwd: agineRoot,
    env: hostToolsEnv(ingressBase, gatewayBase, hostWorkspace, runtimePid, tempRoot),
  });
  hostToolsCheck.finishedAt = new Date().toISOString();
  hostToolsCheck.status = 'PASS';
  manifestBase.phases.hostToolsCheck = hostToolsCheck;
  console.log('router-live:agine: host-tools check PASS');

  console.log('router-live:agine: phase 3/3 running strict full host-tools');
  const hostToolsFull = {
    command: `node ${join(agineRoot, 'client', 'e2e', 'host-tools.mjs')}`,
    cwd: agineRoot,
    ingressBase,
    startedAt: new Date().toISOString(),
    runtimePid,
    workspace: hostWorkspace,
    allowedTools: ALLOWED_HOST_TOOLS,
    sampleFile: join(tempRoot, 'host-tools.sample.txt'),
  };
  const fullOutcome = await runCapturedTee(
    process.execPath,
    [join(agineRoot, 'client', 'e2e', 'host-tools.mjs')],
    { cwd: agineRoot, env: hostToolsEnv(ingressBase, gatewayBase, hostWorkspace, runtimePid, tempRoot) },
  );
  if (fullOutcome.error !== null || fullOutcome.signal !== null || fullOutcome.code !== 0) {
    throw new Error(
      `strict full host-tools exited ${fullOutcome.signal ?? fullOutcome.code}`
      + (fullOutcome.error?.message ? ` (${fullOutcome.error.message})` : '')
      + `\n${(fullOutcome.stderr || fullOutcome.stdout).slice(-4000)}`,
    );
  }
  const hostToolsEvidence = assertHostToolsFullEvidence(
    hostToolsFull,
    `${fullOutcome.stdout}\n${fullOutcome.stderr}`,
  );
  hostToolsFull.finishedAt = new Date().toISOString();
  hostToolsFull.status = 'PASS';
  hostToolsFull.sampleBytes = hostToolsEvidence.sampleBytes;
  hostToolsFull.terminal = hostToolsEvidence.terminal;
  hostToolsFull.assistantChars = hostToolsEvidence.assistantChars;
  hostToolsFull.toolCalls = hostToolsEvidence.toolCalls;
  hostToolsFull.allowedToolCalls = hostToolsEvidence.allowedToolCalls;
  manifestBase.phases.hostToolsFull = hostToolsFull;
  validateAgineLiveManifest(manifestBase);
  console.log('router-live:agine: strict full host-tools PASS');

  const manifestPath = resolve(
    process.env.SKIFF_ROUTER_AGINE_LIVE_MANIFEST_OUT || join(tempRoot, 'router-agine-live-manifest.json'),
  );
  await writeFile(manifestPath, `${JSON.stringify(manifestBase, null, 2)}\n`);
  console.log(`router-live:agine: manifest written to ${manifestPath}`);
  console.log(evidenceSummary({
    pinned,
    binaries,
    assemblyIdentity,
    configSnapshotId,
    manifestPath,
  }));
  console.log('router-live:agine: PASS');
} catch (error) {
  runFailed = true;
  process.stdout.write(error?.stdout ?? '');
  process.stderr.write(error?.stderr ?? '');
  if (tempRoot !== undefined) {
    await dumpManagedLogs(tempRoot, harness);
  }
  throw error;
} finally {
  const keepDebug = KEEP_ON_FAILURE && runFailed;
  if (keepDebug) {
    console.log(
      `router-live:agine: KEEP_ON_FAILURE preserving temp workspace ${tempRoot} `
      + '(processes, ingress, mongo and ports are left running for diagnosis)',
    );
  }
  if (!keepDebug) {
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
      throw new AggregateError(errors, 'router-live:agine cleanup failed');
    }
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
  profile,
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
          profile,
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
        `router-live:agine could not close exact package/service dependencies:\n${details}`,
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
    profile,
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
    profile,
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

function hostToolsEnv(ingressBase, gatewayBase, hostWorkspace, runtimePid, tempRoot) {
  return {
    ...process.env,
    AGINE_E2E_INGRESS_HTTP_BASE: ingressBase,
    AGINE_E2E_PROVIDER_ID: 'aihub',
    AGINE_E2E_MODEL_ID: 'deepseek-v4-flash',
    AGINE_HOST_TOOLS_GATEWAY: gatewayBase,
    AGINE_HOST_TOOLS_WORKSPACE: hostWorkspace,
    AGINE_HOST_TOOLS_ALLOWED: ALLOWED_HOST_TOOLS.join(','),
    // Explicit runtime PID from the harness; takes precedence over the
    // host-tools pgrep fallback once the internals-side contract lands.
    AGINE_HOST_TOOLS_RUNTIME_PID: String(runtimePid),
    AGINE_HOST_TOOLS_SAMPLE_FILE: join(tempRoot, 'host-tools.sample.txt'),
    AGINE_HOST_TOOLS_PROMPT: HOST_TOOLS_PROMPT,
    AGINE_HOST_TOOLS_SYSTEM_PROMPT: HOST_TOOLS_SYSTEM_PROMPT,
  };
}

async function prepareStrictHostWorkspace(tempRoot) {
  // Narrowed read-only test root: a read-only copy of the Skiff `doc`
  // directory (the host file-tool root is the host process cwd).
  const sourceDoc = join(repoRoot, 'doc');
  const workspace = join(tempRoot, 'host-tools-workspace');
  await cp(sourceDoc, workspace, { recursive: true });
  const outcome = spawnSync('chmod', ['-R', 'a-w', workspace], { encoding: 'utf8' });
  if (outcome.status !== 0) {
    throw new Error(
      `router-live:agine could not make host workspace read-only: `
      + `${outcome.stderr || outcome.status}`,
    );
  }
  await chmod(workspace, 0o555);
  return workspace;
}

function assertHostToolsFullEvidence(phase, output) {
  const done = /\[host-tools\] done elapsedMs=\d+ terminal=(\w+) assistantChars=(\d+) toolCalls=(\d+)/
    .exec(output);
  if (!done) {
    throw new Error(
      `strict full host-tools produced no terminal evidence line\n${output.slice(-4000)}`,
    );
  }
  const [, terminal, assistantChars, toolCalls] = done;
  if (terminal !== 'completed') {
    throw new Error(`strict full host-tools terminal must be completed, got ${terminal}`);
  }
  if (Number(assistantChars) <= 0) {
    throw new Error('strict full host-tools assistant reply must be non-empty');
  }
  if (Number(toolCalls) < 1) {
    throw new Error('strict full host-tools must include at least one host.file.* tool call');
  }
  const calls = [...output.matchAll(/\[host-tools\] tool-call (host\.[A-Za-z0-9_.-]+)/g)]
    .map((match) => match[1]);
  if (calls.length !== Number(toolCalls)) {
    throw new Error(
      `strict full host-tools tool-call ledger mismatch: done says ${toolCalls}, parsed ${calls.length}`,
    );
  }
  for (const toolName of calls) {
    if (!toolName.startsWith('host.file.')) {
      throw new Error(
        `strict full host-tools executed forbidden tool ${toolName}; only host.file.* allowed`,
      );
    }
  }
  const sample = /\[host-tools\] sample file: (\S+) \((?:(\d+) bytes|missing)\)/.exec(output);
  if (!sample || sample[2] === undefined || Number(sample[2]) <= 0) {
    throw new Error('strict full host-tools profiling sample file must be non-empty');
  }
  return {
    terminal,
    assistantChars: Number(assistantChars),
    toolCalls: Number(toolCalls),
    allowedToolCalls: calls.filter((name) => name.startsWith('host.file.')).length,
    sampleBytes: Number(sample[2]),
  };
}

async function runCapturedTee(command, args, { cwd, env }) {
  const child = spawn(command, args, {
    cwd,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  let spawnError = null;
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', (chunk) => {
    stdout += chunk;
    process.stdout.write(chunk);
  });
  child.stderr.on('data', (chunk) => {
    stderr += chunk;
    process.stderr.write(chunk);
  });
  const outcome = await new Promise((resolvePromise) => {
    child.once('error', (error) => {
      spawnError = error;
    });
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal });
    });
  });
  return { ...outcome, stdout, stderr, error: spawnError };
}

function isUnpublishedExactDependency(error) {
  return /has no published (?:provider )?(?:PackageArtifact|ServiceContract) pointer/.test(
    errorMessage(error),
  );
}

function errorMessage(error) {
  return error?.message || String(error);
}

function renderRouterConfig({
  profile,
  artifactRoot,
  httpPort,
  runtimePort,
  mongoUrl,
}) {
  return [
    `profile: ${yamlQuote(profile)}`,
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
    'serviceDb:',
    '  encryption:',
    `    keyringFile: ${yamlQuote(keyringPath)}`,
    '',
  ].join('\n');
}

async function assertAihubDeepseekKeyConfigured(aihubServiceRoot) {
  const secretPath = join(aihubServiceRoot, 'config.dev.secret.yml');
  let source;
  try {
    source = await readFile(secretPath, 'utf8');
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(
        `router-live:agine requires ${secretPath} with a deepseek.apiKey; `
        + 'the aihub service config supplies the key used by the smoke',
      );
    }
    throw error;
  }
  let inDeepseek = false;
  for (const rawLine of source.split(/\r?\n/)) {
    if (/^\s*(?:#.*)?$/.test(rawLine)) continue;
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
    if (value.length > 0 && value !== 'null') {
      return;
    }
  }
  throw new Error(
    `router-live:agine requires a deepseek.apiKey in ${secretPath}; `
    + 'the aihub service config supplies the key used by the smoke',
  );
}

async function spawnManaged(label, command, args, { cwd, tempRoot }) {
  const stdoutPath = join(tempRoot, `${label}.stdout.log`);
  const stderrPath = join(tempRoot, `${label}.stderr.log`);
  const stdout = await open(stdoutPath, 'w');
  const stderr = await open(stderrPath, 'w');
  logFiles.push({ path: stdoutPath, handle: stdout });
  logFiles.push({ path: stderrPath, handle: stderr });
  // child-process-owner: router-agine-live-spawn
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
  ingressBase,
  children,
}) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < 120_000) {
    for (const managed of children) {
      if (managed.child.exitCode !== null || managed.child.signalCode !== null) {
        throw new Error(
          `${managed.label} exited before readiness with ${managed.child.signalCode ?? managed.child.exitCode}`,
        );
      }
    }
    try {
      const response = await fetch(`${ingressBase}/session`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: '{}',
      });
      if (response.ok) {
        const setCookies = typeof response.headers.getSetCookie === 'function'
          ? response.headers.getSetCookie()
          : [response.headers.get('set-cookie')].filter(Boolean);
        if (setCookies.some((value) => typeof value === 'string' && value.includes('='))) {
          return;
        }
      }
    } catch {
      // Router/Runtime are still starting; retry.
    }
    await delay(100);
  }
  throw new Error(
    `isolated Router/Runtime did not complete a /session roundtrip at ${ingressBase} within 120s`,
  );
}

async function gitRevParse(root, revision = 'HEAD') {
  const outcome = await captureCheckedCommand('git', ['rev-parse', revision], { cwd: root });
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
  return { paths: paths.length, sha256 };
}

async function readPinnedRepositories() {
  const pinned = {
    skiff: {
      repository: 'skiff',
      commit: await gitRevParse(repoRoot),
      treeHash: await gitRevParse(repoRoot, 'HEAD^{tree}'),
      dirty: await gitDirtyFingerprint(repoRoot),
    },
    internals: {
      repository: 'internals',
      commit: await gitRevParse(internalsRoot),
      treeHash: await gitRevParse(internalsRoot, 'HEAD^{tree}'),
      dirty: await gitDirtyFingerprint(internalsRoot),
    },
    skiffPackages: {
      repository: 'skiff-packages',
      commit: await gitRevParse(skiffPackagesRoot),
      treeHash: await gitRevParse(skiffPackagesRoot, 'HEAD^{tree}'),
      dirty: await gitDirtyFingerprint(skiffPackagesRoot),
    },
  };
  return pinned;
}

async function sha256File(filePath) {
  const hash = createHash('sha256');
  const handle = await open(filePath, 'r');
  try {
    const buffer = Buffer.alloc(1024 * 1024);
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, null);
      if (bytesRead === 0) break;
      hash.update(buffer.subarray(0, bytesRead));
    }
  } finally {
    await handle.close();
  }
  return {
    path: resolve(filePath),
    sha256: hash.digest('hex'),
  };
}

async function dumpManagedLogs(tempRoot, harness) {
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
  if (harness !== undefined) {
    logPaths.push(join(harness.tempRoot, 'mongod.log'));
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
  pinned,
  binaries,
  assemblyIdentity,
  configSnapshotId,
  manifestPath,
}) {
  return [
    'router-live:agine evidence:',
    `  skiff: ${pinned.skiff.commit} tree=${pinned.skiff.treeHash}`,
    `  internals: ${pinned.internals.commit} tree=${pinned.internals.treeHash}`,
    `  skiff-packages: ${pinned.skiffPackages.commit} tree=${pinned.skiffPackages.treeHash}`,
    `  compiler: ${binaries.compiler.sha256} ${binaries.compiler.path}`,
    `  router: ${binaries.router.sha256} ${binaries.router.path}`,
    `  runtime: ${binaries.runtime.sha256} ${binaries.runtime.path}`,
    `  engine: legacy-tree`,
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
      throw new Error(`router-live:agine preflight: ${label} root missing: ${root} (${errorMessage(error)})`);
    }
  }
  const hostToolsPath = join(agineRoot, 'client', 'e2e', 'host-tools.mjs');
  try {
    await access(hostToolsPath);
  } catch {
    throw new Error(
      `router-live:agine preflight: host-tools missing: ${hostToolsPath}`,
    );
  }
  const pinned = await readPinnedRepositories();
  await assertAihubDeepseekKeyConfigured(aihubServiceRoot);
  for (const executable of [
    'cargo',
    'node',
    'npm',
    'mongod',
    'mongosh',
    'git',
    'pgrep',
    'sample',
    'chmod',
  ]) {
    const outcome = await captureCheckedCommand('which', [executable], { cwd: repoRoot });
    if (outcome.stdout.trim().length === 0) {
      throw new Error(`router-live:agine preflight: ${executable} is not on PATH`);
    }
  }
  console.log(
    `router-live:agine preflight: skiff=${pinned.skiff.commit} internals=${pinned.internals.commit} `
    + `skiff-packages=${pinned.skiffPackages.commit}`,
  );
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

// ---------------------------------------------------------------------------
// Provenance manifest schema (`router-live:agine` Phase 0)
// ---------------------------------------------------------------------------

const AGINE_LIVE_MANIFEST_SCHEMA_VERSION = 'skiff-router-agine-live-manifest-v1';

export function agineLiveManifestSchemaVersion() {
  return AGINE_LIVE_MANIFEST_SCHEMA_VERSION;
}

const COMMIT_PATTERN = /^[0-9a-f]{40}$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const PROFILE_PATTERN = /^[A-Za-z0-9._-]{1,200}$/;
const SERVICE_ID_PATTERN =
  /^[A-Za-z0-9][A-Za-z0-9._-]*(\/[A-Za-z0-9][A-Za-z0-9._-]*)+$/;
const VERSION_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+$/;
const ASSEMBLY_IDENTITY_PATTERN =
  /^skiff-runtime-assembly-v3:sha256:[0-9a-f]{64}$/;
const CONFIG_SNAPSHOT_ID_PATTERN =
  /^skiff-runtime-config-snapshot-v1:[0-9a-f]{32}$/;
const DEPLOYMENT_REVISION_PATTERN = /^sha256-[0-9a-f]{64}$/;
const DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN =
  /^skiff-deployment-artifact-v4:sha256:[0-9a-f]{64}$/;
const PACKAGE_BUILD_ID_PATTERN =
  /^skiff-package-build-v10:sha256:[0-9a-f]{64}$/;
const PACKAGE_LOCAL_ABI_IDENTITY_PATTERN =
  /^skiff-package-local-abi-v7:sha256:[0-9a-f]{64}$/;
const STATUS_PATTERN = /^(PASS|FAIL)$/;

/**
 * Strictly validates one `router-live:agine` provenance manifest and returns a
 * frozen copy. Unknown keys, wrong types and identity pattern mismatches are
 * rejected so the Phase 0 evidence record cannot drift from the contract.
 */
export function validateAgineLiveManifest(value, label = 'manifest') {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  exactKeys(value, [
    'schemaVersion',
    'engine',
    'pinned',
    'binaries',
    'profile',
    'artifactRoot',
    'ports',
    'mongo',
    'assembly',
    'configSnapshot',
    'services',
    'packages',
    'phases',
  ], label);
  if (value.schemaVersion !== AGINE_LIVE_MANIFEST_SCHEMA_VERSION) {
    throw new Error(`${label} schemaVersion must be ${AGINE_LIVE_MANIFEST_SCHEMA_VERSION}`);
  }
  if (value.engine !== 'legacy-tree') {
    throw new Error(`${label} engine must be legacy-tree in Phase 0`);
  }
  const pinned = validatePinned(value.pinned, `${label}.pinned`);
  const binaries = validateBinaries(value.binaries, `${label}.binaries`);
  const profile = validatePattern(value.profile, PROFILE_PATTERN, `${label}.profile`);
  const artifactRoot = validateNonEmptyString(value.artifactRoot, `${label}.artifactRoot`);
  const ports = exactObject(value.ports, ['http', 'runtime', 'ingress'], `${label}.ports`);
  for (const key of ['http', 'runtime', 'ingress']) {
    validatePort(ports[key], `${label}.ports.${key}`);
  }
  const mongo = exactObject(value.mongo, ['url'], `${label}.mongo`);
  const mongoUrl = validateNonEmptyString(mongo.url, `${label}.mongo.url`);
  const assembly = exactObject(value.assembly, ['assemblyIdentity'], `${label}.assembly`);
  const assemblyIdentity = validatePattern(
    assembly.assemblyIdentity,
    ASSEMBLY_IDENTITY_PATTERN,
    `${label}.assembly.assemblyIdentity`,
  );
  const configSnapshot = exactObject(
    value.configSnapshot,
    ['snapshotId'],
    `${label}.configSnapshot`,
  );
  const snapshotId = validatePattern(
    configSnapshot.snapshotId,
    CONFIG_SNAPSHOT_ID_PATTERN,
    `${label}.configSnapshot.snapshotId`,
  );
  const services = value.services.map((entry, index) =>
    validateServiceEntry(entry, `${label}.services[${index}]`));
  const packages = value.packages.map((entry, index) =>
    validatePackageEntry(entry, `${label}.packages[${index}]`));
  const phases = validatePhases(value.phases, `${label}.phases`);

  return deepFreeze({
    schemaVersion: AGINE_LIVE_MANIFEST_SCHEMA_VERSION,
    engine: 'legacy-tree',
    pinned,
    binaries,
    profile,
    artifactRoot,
    ports: { http: ports.http, runtime: ports.runtime, ingress: ports.ingress },
    mongo: { url: mongoUrl },
    assembly: { assemblyIdentity },
    configSnapshot: { snapshotId },
    services,
    packages,
    phases,
  });
}

function validatePinned(value, label) {
  exactKeys(value, ['skiff', 'internals', 'skiffPackages'], label);
  const entries = {};
  for (const key of ['skiff', 'internals', 'skiffPackages']) {
    const repository = exactKeys(
      value[key],
      ['repository', 'commit', 'treeHash', 'dirty'],
      `${label}.${key}`,
    );
    const dirty = repository.dirty === null
      ? null
      : exactKeys(repository.dirty, ['paths', 'sha256'], `${label}.${key}.dirty`);
    if (dirty !== null) {
      if (!Number.isSafeInteger(dirty.paths) || dirty.paths < 0) {
        throw new Error(`${label}.${key}.dirty.paths must be a non-negative integer`);
      }
      validatePattern(dirty.sha256, SHA256_PATTERN, `${label}.${key}.dirty.sha256`);
    }
    entries[key] = {
      repository: validatePattern(
        repository.repository,
        /^[A-Za-z0-9][A-Za-z0-9._-]*$/,
        `${label}.${key}.repository`,
      ),
      commit: validatePattern(
        repository.commit,
        COMMIT_PATTERN,
        `${label}.${key}.commit`,
      ),
      treeHash: validatePattern(
        repository.treeHash,
        COMMIT_PATTERN,
        `${label}.${key}.treeHash`,
      ),
      dirty,
    };
  }
  return entries;
}

function validateBinaries(value, label) {
  exactKeys(value, ['compiler', 'router', 'runtime'], label);
  const entries = {};
  for (const key of ['compiler', 'router', 'runtime']) {
    const binary = exactKeys(value[key], ['path', 'sha256'], `${label}.${key}`);
    entries[key] = {
      path: validateNonEmptyString(binary.path, `${label}.${key}.path`),
      sha256: validatePattern(binary.sha256, SHA256_PATTERN, `${label}.${key}.sha256`),
    };
  }
  return entries;
}

function validatePhases(value, label) {
  exactKeys(value, ['chatSmoke', 'hostToolsCheck', 'hostToolsFull'], label);
  const chatSmoke = validatePhaseRun(
    value.chatSmoke,
    ['command', 'cwd', 'ingressBase', 'startedAt', 'finishedAt', 'status'],
    `${label}.chatSmoke`,
  );
  const hostToolsCheck = validatePhaseRun(
    value.hostToolsCheck,
    ['command', 'cwd', 'ingressBase', 'startedAt', 'finishedAt', 'status'],
    `${label}.hostToolsCheck`,
  );
  const hostToolsFull = validatePhaseRun(
    value.hostToolsFull,
    [
      'command',
      'cwd',
      'ingressBase',
      'startedAt',
      'finishedAt',
      'status',
      'runtimePid',
      'workspace',
      'allowedTools',
      'sampleFile',
      'sampleBytes',
      'terminal',
      'assistantChars',
      'toolCalls',
      'allowedToolCalls',
    ],
    `${label}.hostToolsFull`,
  );
  if (
    !Number.isSafeInteger(hostToolsFull.runtimePid)
    || hostToolsFull.runtimePid <= 0
  ) {
    throw new Error(`${label}.hostToolsFull.runtimePid must be a positive integer`);
  }
  for (const tool of hostToolsFull.allowedTools) {
    validatePattern(tool, /^host\.[A-Za-z0-9_.-]+$/, `${label}.hostToolsFull.allowedTools`);
    if (tool === 'host.shell.run') {
      throw new Error(`${label}.hostToolsFull.allowedTools must not include host.shell.run`);
    }
  }
  if (hostToolsFull.sampleBytes < 0) {
    throw new Error(`${label}.hostToolsFull.sampleBytes must be non-negative`);
  }
  if (hostToolsFull.assistantChars <= 0) {
    throw new Error(`${label}.hostToolsFull.assistantChars must be positive`);
  }
  if (
    hostToolsFull.toolCalls < 1
    || hostToolsFull.allowedToolCalls < 1
    || hostToolsFull.allowedToolCalls > hostToolsFull.toolCalls
  ) {
    throw new Error(
      `${label}.hostToolsFull tool call counts are inconsistent with the strict evidence`,
    );
  }
  if (hostToolsFull.terminal !== 'completed') {
    throw new Error(`${label}.hostToolsFull.terminal must be completed`);
  }
  return { chatSmoke, hostToolsCheck, hostToolsFull };
}

function validatePhaseRun(value, keys, label) {
  exactKeys(value, keys, label);
  const command = validatePattern(value.command, /^\S+( \S+)*$/, `${label}.command`);
  const cwd = validateNonEmptyString(value.cwd, `${label}.cwd`);
  const ingressBase = validatePattern(
    value.ingressBase,
    /^https?:\/\/127\.0\.0\.1:[0-9]{2,5}$/,
    `${label}.ingressBase`,
  );
  const startedAt = validateTimestamp(value.startedAt, `${label}.startedAt`);
  const finishedAt = validateTimestamp(value.finishedAt, `${label}.finishedAt`);
  const status = validatePattern(value.status, STATUS_PATTERN, `${label}.status`);
  const extra = {};
  for (const key of keys) {
    if (
      !['command', 'cwd', 'ingressBase', 'startedAt', 'finishedAt', 'status'].includes(key)
    ) {
      extra[key] = value[key];
    }
  }
  return deepFreeze({
    command,
    cwd,
    ingressBase,
    startedAt,
    finishedAt,
    status,
    ...extra,
  });
}

function validateServiceEntry(value, label) {
  exactKeys(value, [
    'serviceId',
    'contractVersion',
    'deploymentRevision',
    'deploymentArtifactIdentity',
    'implementationPackageBuildId',
  ], label);
  return deepFreeze({
    serviceId: validatePattern(value.serviceId, SERVICE_ID_PATTERN, `${label}.serviceId`),
    contractVersion: validatePattern(
      value.contractVersion,
      VERSION_PATTERN,
      `${label}.contractVersion`,
    ),
    deploymentRevision: validatePattern(
      value.deploymentRevision,
      DEPLOYMENT_REVISION_PATTERN,
      `${label}.deploymentRevision`,
    ),
    deploymentArtifactIdentity: validatePattern(
      value.deploymentArtifactIdentity,
      DEPLOYMENT_ARTIFACT_IDENTITY_PATTERN,
      `${label}.deploymentArtifactIdentity`,
    ),
    implementationPackageBuildId: validatePattern(
      value.implementationPackageBuildId,
      PACKAGE_BUILD_ID_PATTERN,
      `${label}.implementationPackageBuildId`,
    ),
  });
}

function validatePackageEntry(value, label) {
  exactKeys(value, [
    'packageId',
    'packageVersion',
    'packageBuildId',
    'packageLocalAbiIdentity',
  ], label);
  return deepFreeze({
    packageId: validatePattern(
      value.packageId,
      SERVICE_ID_PATTERN,
      `${label}.packageId`,
    ),
    packageVersion: validatePattern(
      value.packageVersion,
      VERSION_PATTERN,
      `${label}.packageVersion`,
    ),
    packageBuildId: validatePattern(
      value.packageBuildId,
      PACKAGE_BUILD_ID_PATTERN,
      `${label}.packageBuildId`,
    ),
    packageLocalAbiIdentity: validatePattern(
      value.packageLocalAbiIdentity,
      PACKAGE_LOCAL_ABI_IDENTITY_PATTERN,
      `${label}.packageLocalAbiIdentity`,
    ),
  });
}

function validateTimestamp(value, label) {
  return validatePattern(
    value,
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/,
    label,
  );
}

function validatePort(value, label) {
  if (!Number.isSafeInteger(value) || value < 1 || value > 65535) {
    throw new Error(`${label} must be an integer from 1 to 65535`);
  }
  return value;
}

function exactKeys(value, expected, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new Error(`${label} must contain exactly ${expected.join(', ')}`);
  }
  return value;
}

function exactObject(value, keys, label) {
  exactKeys(value, keys, label);
  return value;
}

function validatePattern(value, pattern, label) {
  if (typeof value !== 'string' || !pattern.test(value)) {
    throw new Error(`${label} must match ${pattern}`);
  }
  return value;
}

function validateNonEmptyString(value, label) {
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
