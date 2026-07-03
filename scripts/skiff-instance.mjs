#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import { access, chmod, copyFile, mkdir, open, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { cargoBuildEnv } from './lib/cargo-target-dir.mjs';
import {
  devSyncCheckFlags,
  instanceDevSyncOptions,
  parseDevSyncArgs,
  renderDevSyncArgs,
} from './lib/dev-sync-args.mjs';
import {
  defaultInstanceConfig,
  defaultInstanceConfigText,
  instanceBasePaths,
  instanceSummary,
  readInstanceConfig,
} from './lib/local-instance-config.mjs';
import {
  renderRouterConfig,
  renderRuntimeConfig,
  renderTelemetryConfig,
} from './lib/runtime-stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');
const pidMetadataSchemaVersion = 1;
const startTimeoutMs = 20000;
const stopTimeoutMs = 5000;
const instanceCommands = new Set([
  'init',
  'paths',
  'status',
  'doctor',
  'repair',
  'build',
  'up',
  'restart',
  'supervise',
  'run',
  'down',
  'reload',
  'sync',
  'watch',
]);
const usage = `usage:
  skiff instance init <config> [--force]
  skiff instance paths <config> [--json]
  skiff instance status <config> [--json]
  skiff instance doctor <config>
  skiff instance repair <config>
  skiff instance build <config>
  skiff instance up <config> [--repair-owned-conflicts]
  skiff instance restart <config> [component]
  skiff instance supervise <config>
  skiff instance run <config>  # deprecated alias for supervise
  skiff instance down <config>
  skiff instance reload <config>
  skiff instance sync <config> [root] [--profile <name>] [--service-id <id>] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--check|--check-sync]
  skiff instance watch <config> [root] [--profile <name>] [--service-id <id>] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--poll-interval-ms <ms>]`;

try {
  await main(process.argv.slice(2));
} catch (error) {
  console.error(`error: ${error?.message || String(error)}`);
  process.exitCode = 1;
}

export async function main(rawArgs) {
  const { configPath, args } = parseInstanceConfig(rawArgs);
  const subcommand = args.shift();
  switch (subcommand) {
    case undefined:
    case '-h':
    case '--help':
      console.log(usage);
      return;
    case 'init':
      await initInstance(args, configPath);
      return;
    case 'paths':
      await printPaths(args, configPath);
      return;
    case 'status':
      await printStatus(args, configPath);
      return;
    case 'doctor':
      await doctorInstance(args, configPath);
      return;
    case 'repair':
      await repairInstance(args, configPath);
      return;
    case 'build':
      await buildInstance(args, configPath);
      return;
    case 'up':
      await upInstance(args, configPath);
      return;
    case 'restart':
      await restartInstance(args, configPath);
      return;
    case 'supervise':
      await superviseCommand(args, configPath);
      return;
    case 'run':
      await runInstance(args, configPath);
      return;
    case 'down':
      await downInstance(args, configPath);
      return;
    case 'reload':
      await reloadInstance(args, configPath);
      return;
    case 'sync':
      await syncInstance(args, configPath, false);
      return;
    case 'watch':
      await syncInstance(args, configPath, true);
      return;
    default:
      throw new Error(`unknown instance command ${subcommand}\n${usage}`);
  }
}

async function initInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set(['--force']) });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const paths = instanceBasePaths({ configPath, repoRoot: skiffRoot });
  const force = args.flags.has('--force');
  const config = defaultInstanceConfig({ configPath: paths.configPath, repoRoot: skiffRoot });
  await ensureInstanceDirs(config.paths);
  const configWrite = await writeIfMissing(paths.configPath, defaultInstanceConfigText(), force);
  const writes = [configWrite, ...await writeRuntimeConfigs(config, force)];
  console.log(`instance config: ${paths.configPath}`);
  for (const write of writes) {
    console.log(`${write.action}: ${write.path}`);
  }
}

async function printPaths(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set(['--json']) });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  const result = instanceSummary(config);
  if (args.flags.has('--json')) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  for (const [key, value] of Object.entries(result)) {
    console.log(`${key}: ${typeof value === 'object' ? JSON.stringify(value) : value}`);
  }
}

async function printStatus(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set(['--json']) });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  const result = await instanceStatus(config);
  if (args.flags.has('--json')) {
    console.log(JSON.stringify(result, null, 2));
    return;
  }
  console.log(`configPath: ${config.paths.configPath}`);
  console.log(`routerHttpUrl: ${config.urls.routerHttp}`);
  console.log(`routerReloadUrl: ${config.urls.routerReload}`);
  console.log(`telemetryUrl: ${config.urls.telemetry}`);
  for (const processStatus of result.processes) {
    console.log(renderProcessStatusLine(processStatus));
    for (const port of processStatus.ports) {
      const listeners = port.listeners.length === 0
        ? 'no listeners'
        : port.listeners
            .map((listener) =>
              `pid=${listener.pid} pgid=${listener.pgid ?? '?'} owner=${listener.ownership}`)
            .join(', ');
      console.log(`  port ${port.port}: ${listeners}`);
    }
  }
}

async function doctorInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  const result = await instanceStatus(config);
  console.log(`configPath: ${config.paths.configPath}`);
  for (const processStatus of result.processes) {
    console.log(renderProcessStatusLine(processStatus));
    for (const recommendation of recommendationsForProcess(processStatus)) {
      console.log(`  fix: ${recommendation}`);
    }
  }
}

async function buildInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  await ensureInstanceDirs(config.paths);
  await buildComponentBinaries(config);
  console.log(JSON.stringify({
    runtime: {
      mode: config.components.runtime,
      path: config.paths.runtimeBinary,
    },
    identityCli: {
      mode: config.components.identityCli,
      path: config.paths.identityCli,
    },
  }, null, 2));
}

async function upInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set(['--repair-owned-conflicts']) });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  await ensureInstanceDirs(config.paths);
  await writeRuntimeConfigs(config, true);
  await buildComponentBinaries(config);

  for (const spec of managedProcessSpecs(config)) {
    await ensureManagedProcessRunning(config, spec, {
      repairOwnedConflicts: args.flags.has('--repair-owned-conflicts'),
    });
  }

  console.log(JSON.stringify(await instanceStatus(config), null, 2));
}

async function runInstance(rawArgs, configPath) {
  console.warn('[skiff-instance] skiff instance run is deprecated; use skiff instance supervise');
  await superviseCommand(rawArgs, configPath);
}

async function superviseCommand(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  await ensureInstanceDirs(config.paths);
  await writeRuntimeConfigs(config, true);
  await buildComponentBinaries(config);
  await superviseInstance(config);
}

async function downInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  const stopped = [];
  for (const spec of [...managedProcessSpecs(config)].reverse()) {
    stopped.push(await stopManagedProcess(config, spec.name));
  }
  console.log(JSON.stringify({ stopped }, null, 2));
}

async function restartInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length > 1) {
    throw new Error(`unexpected argument ${args.positionals[1]}`);
  }
  const config = await loadInstance(configPath);
  const specs = managedProcessSpecs(config);
  const component = args.positionals[0];
  if (component !== undefined) {
    const spec = specs.find((candidate) => candidate.name === component);
    if (spec === undefined) {
      throw new Error(`unknown or unmanaged instance component ${component}; expected ${specs.map(({ name }) => name).join(', ')}`);
    }
    await ensureInstanceDirs(config.paths);
    await writeRuntimeConfigs(config, true);
    await buildComponentBinaries(config);
    const stopped = await stopManagedProcess(config, component);
    await ensureManagedProcessRunning(config, spec, { repairOwnedConflicts: false });
    console.log(JSON.stringify({ restarted: [component], stopped, status: await instanceStatus(config) }, null, 2));
    return;
  }
  await ensureInstanceDirs(config.paths);
  await writeRuntimeConfigs(config, true);
  await buildComponentBinaries(config);
  const stopped = [];
  for (const spec of [...specs].reverse()) {
    stopped.push(await stopManagedProcess(config, spec.name));
  }
  for (const spec of specs) {
    await ensureManagedProcessRunning(config, spec, { repairOwnedConflicts: false });
  }
  console.log(JSON.stringify({ restarted: specs.map(({ name }) => name), stopped, status: await instanceStatus(config) }, null, 2));
}

async function repairInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  await ensureInstanceDirs(config.paths);
  await writeRuntimeConfigs(config, true);
  await buildComponentBinaries(config);
  const repaired = [];
  const before = await instanceStatus(config);
  for (const processStatus of before.processes) {
    if (hasUnknownPortConflict(processStatus)) {
      throw new Error(`${processStatus.name} has unknown port conflict; refusing repair`);
    }
  }
  for (const processStatus of before.processes) {
    if (processStatus.category === 'stale-pid') {
      await rm(processStatus.pidPath, { force: true });
      repaired.push({ name: processStatus.name, action: 'removed-stale-pid' });
      continue;
    }
    if (isRepairableProcess(processStatus)) {
      repaired.push(await stopComponentStatus(config, processStatus));
    }
  }
  for (const spec of managedProcessSpecs(config)) {
    await ensureManagedProcessRunning(config, spec, { repairOwnedConflicts: true });
  }
  console.log(JSON.stringify({ repaired, status: await instanceStatus(config) }, null, 2));
}

async function reloadInstance(rawArgs, configPath) {
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  const response = await fetch(config.urls.routerReload, { method: 'POST' });
  const body = await response.text();
  if (!response.ok) {
    throw new Error(`router reload returned HTTP ${response.status}${body ? `: ${body}` : ''}`);
  }
  console.log(`requested router reload at ${config.urls.routerReload}`);
  if (body.trim()) {
    console.log(body.trim());
  }
}

async function syncInstance(rawArgs, configPath, watch) {
  const args = parseDevSyncArgs(rawArgs, {
    flags: devSyncCheckFlags,
    options: instanceDevSyncOptions,
    resolve,
    allowEmptyEquals: true,
    allowDashEquals: true,
  });
  const config = await loadInstance(configPath);
  await ensureInstanceDirs(config.paths);
  await run('node', [
    join(scriptDir, 'skiff-dev-sync.mjs'),
    ...renderDevSyncArgs(args, {
      prefix: watch ? ['--watch'] : [],
      injectOptions: {
        artifactRoot: config.paths.artifactRoot,
        reloadUrl: config.urls.routerReload,
        defaultPackagesDir: config.packageDirs,
      },
    }),
  ], process.cwd());
}

async function loadInstance(configPath) {
  return readInstanceConfig({
    configPath,
    repoRoot: skiffRoot,
  });
}

async function ensureInstanceDirs(paths) {
  await mkdir(paths.instanceRoot, { recursive: true });
  await mkdir(paths.devHome, { recursive: true });
  await mkdir(paths.artifactRoot, { recursive: true });
  await mkdir(paths.serviceBuildRoot, { recursive: true });
  await mkdir(paths.runtimeHome, { recursive: true });
  await mkdir(paths.binDir, { recursive: true });
  await mkdir(paths.serviceDbPath, { recursive: true });
  await mkdir(dirname(paths.watchConfig), { recursive: true });
  await mkdir(paths.pidDir, { recursive: true });
  await mkdir(paths.logDir, { recursive: true });
  await mkdir(paths.buildRoot, { recursive: true });
}

async function writeRuntimeConfigs(config, force) {
  return [
    await writeIfMissing(config.paths.routerConfig, routerConfigText(config), force),
    await writeIfMissing(config.paths.runtimeConfig, runtimeConfigText(config), force),
    ...(config.components.telemetry === 'disabled'
      ? []
      : [await writeIfMissing(config.paths.telemetryConfig, telemetryConfigText(config), force)]),
  ];
}

function routerConfigText(config) {
  return renderRouterConfig({
    profile: 'dev',
    host: '127.0.0.1',
    artifactRoots: [config.paths.artifactRoot],
    identityCliPath: config.paths.identityCli,
    devReload: true,
    requestTimeoutMs: 20000,
    httpPort: config.ports.routerHttp,
    runtimePort: config.ports.routerControl,
    runtimePath: '/runtime',
    serviceDbMongoUrl: `mongodb://127.0.0.1:${config.ports.mongo}/?directConnection=true&replicaSet=rs0&retryWrites=false`,
    telemetryEndpoint: config.components.telemetry === 'disabled' ? undefined : config.urls.telemetry,
  });
}

function runtimeConfigText(config) {
  return renderRuntimeConfig({
    routerUrl: config.urls.routerRuntime,
    runtimeHome: config.paths.runtimeHome,
    artifactRoots: [config.paths.artifactRoot],
  });
}

function telemetryConfigText(config) {
  return renderTelemetryConfig({
    host: '127.0.0.1',
    port: config.ports.telemetry,
    path: '/telemetry',
    memory: config.telemetry.memory,
    emitMemory: true,
    mongo: config.telemetry.memory
      ? undefined
      : {
          url: `mongodb://127.0.0.1:${config.ports.mongo}/?directConnection=true&replicaSet=rs0&retryWrites=false`,
          database: 'skiff',
        },
  });
}

async function buildComponentBinaries(config) {
  if (config.components.runtime === 'installed') {
    await copyBinary(config.installed.runtimeBinary, config.paths.runtimeBinary);
  } else {
    await buildRustBinary({
      manifest: join(skiffRoot, 'runtime', 'Cargo.toml'),
      bin: 'runtime',
      source: join(config.paths.cargoTargetDir, 'debug', process.platform === 'win32' ? 'runtime.exe' : 'runtime'),
      destination: config.paths.runtimeBinary,
      config,
    });
  }

  if (config.components.identityCli === 'installed') {
    await copyBinary(config.installed.identityCli, config.paths.identityCli);
  } else {
    await buildRustBinary({
      manifest: join(skiffRoot, 'artifact-identity', 'Cargo.toml'),
      bin: 'skiff-artifact-identity',
      source: join(
        config.paths.cargoTargetDir,
        'debug',
        process.platform === 'win32' ? 'skiff-artifact-identity.exe' : 'skiff-artifact-identity',
      ),
      destination: config.paths.identityCli,
      config,
    });
  }
}

async function buildRustBinary({ manifest, bin, source, destination, config }) {
  await mkdir(config.paths.cargoTargetDir, { recursive: true });
  await run('cargo', ['build', '--manifest-path', manifest, '--bin', bin], skiffRoot, {
    ...cargoBuildEnv(skiffRoot),
    CARGO_TARGET_DIR: config.paths.cargoTargetDir,
  });
  await copyBinary(source, destination);
}

async function copyBinary(source, destination) {
  await assertFile(source);
  await mkdir(dirname(destination), { recursive: true });
  if (resolve(source) !== resolve(destination)) {
    await copyFile(source, destination);
  }
  if (process.platform !== 'win32') {
    await chmod(destination, 0o755);
  }
  await assertFile(destination);
}

function managedProcessSpecs(config) {
  return [
    ...(config.components.mongo === 'managed'
      ? [{
          name: 'mongo',
          command: config.mongo.binary,
          args: [
            '--dbpath',
            config.paths.serviceDbPath,
            '--port',
            String(config.ports.mongo),
            '--replSet',
            'rs0',
            '--bind_ip',
            '127.0.0.1',
          ],
          cwd: skiffRoot,
          ports: [config.ports.mongo],
        }]
      : []),
    ...(config.components.telemetry === 'disabled'
      ? []
      : [{
          name: 'telemetry',
          command: 'pnpm',
          args: ['--dir', join(skiffRoot, 'telemetry'), 'dev', '--config', config.paths.telemetryConfig],
          cwd: skiffRoot,
          ports: [config.ports.telemetry],
        }]),
    {
      name: 'router',
      command: 'pnpm',
      args: ['--dir', join(skiffRoot, 'router'), 'dev', '--config', config.paths.routerConfig],
      cwd: skiffRoot,
      ports: [config.ports.routerHttp, config.ports.routerControl],
    },
    {
      name: 'runtime',
      command: config.paths.runtimeBinary,
      args: [config.paths.runtimeConfig],
      cwd: skiffRoot,
      ports: [],
    },
    ...(config.components.watch === 'managed'
      ? [{
          name: 'watch',
          command: 'node',
          args: [
            join(scriptDir, 'skiff-dev-sync.mjs'),
            '--watch',
            '--config',
            config.paths.watchConfig,
            ...config.packageDirs.flatMap((dir) => ['--default-packages-dir', dir]),
          ],
          cwd: skiffRoot,
          ports: [],
        }]
      : []),
  ];
}

function processEnv(config) {
  return {
    ...process.env,
    SKIFF_DEV_HOME: config.paths.devHome,
    SKIFF_ARTIFACT_ROOT: config.paths.artifactRoot,
    SKIFF_DEV_RELOAD_URL: config.urls.routerReload,
    RUST_LOG: process.env.RUST_LOG ?? 'info',
  };
}

async function ensureManagedProcessRunning(config, spec, options) {
  while (true) {
    const status = await componentStatus(config, spec.name);
    if (status.category === 'running') {
      console.log(`[skiff-instance] ${spec.name} already running pid=${status.pid}`);
      return { name: spec.name, started: false, reason: 'already-running' };
    }
    if (status.category === 'stopped') {
      return startManagedProcess(config, spec);
    }
    if (status.category === 'stale-pid') {
      await rm(status.pidPath, { force: true });
      return startManagedProcess(config, spec);
    }
    if (hasUnknownPortConflict(status)) {
      throw new Error(`${spec.name} has unknown port conflict; refusing to start`);
    }
    if (options.repairOwnedConflicts && isRepairableProcess(status)) {
      await stopComponentStatus(config, status);
      continue;
    }
    const repairHint = isRepairableProcess(status)
      ? '; retry with --repair-owned-conflicts to stop same-instance conflicts'
      : '';
    throw new Error(`${spec.name} is ${status.category}${repairHint}`);
  }
}

async function componentStatus(config, name) {
  const status = await instanceStatus(config);
  const processStatus = status.processes.find((candidate) => candidate.name === name);
  if (processStatus === undefined) {
    throw new Error(`unknown or unmanaged instance component ${name}`);
  }
  return processStatus;
}

async function startManagedProcess(config, spec, options = {}) {
  await rm(pidPath(config, spec.name), { force: true });
  const out = await open(join(config.paths.logDir, `${spec.name}.log`), 'a');
  const err = await open(join(config.paths.logDir, `${spec.name}.err.log`), 'a');
  const child = spawn(spec.command, spec.args, {
    cwd: spec.cwd,
    env: processEnv(config),
    detached: true,
    stdio: ['ignore', out.fd, err.fd],
  });
  let spawnError = null;
  child.once('error', (error) => {
    spawnError = error;
  });
  if (child.pid === undefined) {
    await out.close();
    await err.close();
    throw new Error(`failed to start ${spec.name}; see ${join(config.paths.logDir, `${spec.name}.err.log`)}`);
  }
  await writePidMetadata(config, spec, child.pid);
  if (!options.supervised) {
    child.unref();
    await out.close();
    await err.close();
  }
  await delay(250);
  if (spawnError !== null) {
    throw new Error(`failed to start ${spec.name}: ${spawnError.message}`);
  }
  if (!isProcessAlive(child.pid)) {
    throw new Error(`${spec.name} exited after start; see ${join(config.paths.logDir, `${spec.name}.err.log`)}`);
  }
  console.log(`[skiff-instance] started ${spec.name} pid=${child.pid} pgid=${child.pid}`);
  if (!options.supervised) {
    await waitForComponentRunning(config, spec, child.pid);
    return { name: spec.name, started: true, pid: child.pid, pgid: child.pid };
  }
  return { name: spec.name, started: true, pid: child.pid, pgid: child.pid, child, out, err };
}

async function waitForComponentRunning(config, spec, pid) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < startTimeoutMs) {
    const status = await componentStatus(config, spec.name);
    if (status.category === 'running') {
      return;
    }
    if (!isProcessAlive(pid)) {
      throw new Error(`${spec.name} exited before becoming healthy; see ${join(config.paths.logDir, `${spec.name}.err.log`)}`);
    }
    if (hasUnknownPortConflict(status)) {
      throw new Error(`${spec.name} encountered unknown port conflict while starting`);
    }
    await delay(250);
  }
  const status = await componentStatus(config, spec.name);
  throw new Error(`${spec.name} did not become healthy within ${startTimeoutMs}ms; status=${status.category}`);
}

async function superviseInstance(config) {
  const specs = managedProcessSpecs(config);
  const running = new Map();
  let stopping = false;

  const stopAll = async (signal) => {
    if (stopping) {
      return;
    }
    stopping = true;
    console.log(`[skiff-instance] stopping after ${signal}`);
    for (const spec of [...specs].reverse()) {
      await stopManagedProcess(config, spec.name);
    }
    process.exit(0);
  };

  process.on('SIGTERM', () => {
    void stopAll('SIGTERM');
  });
  process.on('SIGINT', () => {
    void stopAll('SIGINT');
  });

  const start = async (spec) => {
    if (stopping) {
      return;
    }
    const status = await componentStatus(config, spec.name);
    if (status.category === 'stale-pid') {
      await rm(status.pidPath, { force: true });
    } else if (status.category !== 'stopped') {
      throw new Error(`${spec.name} is ${status.category}; stop or repair it before supervise`);
    }
    const entry = await startManagedProcess(config, spec, { supervised: true });
    running.set(spec.name, entry);
    entry.child.on('exit', (code, signal) => {
      const current = running.get(spec.name);
      running.delete(spec.name);
      void current?.out.close();
      void current?.err.close();
      void (async () => {
        if (current !== undefined && isProcessGroupAlive(current.pgid)) {
          await stopProcessGroup(current.pgid);
        }
        if (current === undefined || !isProcessGroupAlive(current.pgid)) {
          await rm(pidPath(config, spec.name), { force: true });
        }
        if (!stopping) {
          console.warn(`[skiff-instance] ${spec.name} exited with ${signal ?? code}; restarting`);
          setTimeout(() => {
            void start(spec);
          }, 1000);
        }
      })();
    });
  };

  for (const spec of specs) {
    await start(spec);
  }
  console.log(JSON.stringify(await instanceStatus(config), null, 2));
  await new Promise(() => {});
}

async function stopManagedProcess(config, name) {
  return stopComponentStatus(config, await componentStatus(config, name));
}

async function stopComponentStatus(config, processStatus) {
  if (processStatus.category === 'stopped') {
    return { name: processStatus.name, stopped: false, reason: 'stopped' };
  }
  if (processStatus.category === 'stale-pid') {
    await rm(processStatus.pidPath, { force: true });
    return { name: processStatus.name, stopped: false, reason: 'stale-pid-removed' };
  }
  if (hasUnknownPortConflict(processStatus)) {
    return { name: processStatus.name, stopped: false, reason: 'unknown-port-conflict' };
  }
  const groups = [...new Set(processStatus.repairableGroups)].filter((pgid) => Number.isInteger(pgid) && pgid > 0);
  if (groups.length === 0) {
    return { name: processStatus.name, stopped: false, reason: 'no-owned-process-group' };
  }
  const stoppedGroups = [];
  for (const pgid of groups) {
    stoppedGroups.push(await stopProcessGroup(pgid));
  }
  const groupsGone = stoppedGroups.every((group) => !isProcessGroupAlive(group.pgid));
  if (groupsGone) {
    await rm(processStatus.pidPath, { force: true });
  }
  return {
    name: processStatus.name,
    stopped: stoppedGroups.some((group) => group.stopped),
    pidMetadataRemoved: groupsGone,
    groups: stoppedGroups,
  };
}

async function stopProcessGroup(pgid) {
  const result = { pgid, stopped: false, forced: false };
  if (!isProcessGroupAlive(pgid)) {
    return result;
  }
  sendProcessGroupSignal(pgid, 'SIGTERM');
  if (await waitForProcessGroupStopped(pgid, stopTimeoutMs)) {
    result.stopped = true;
    return result;
  }
  result.forced = true;
  sendProcessGroupSignal(pgid, 'SIGKILL');
  await waitForProcessGroupStopped(pgid, stopTimeoutMs);
  result.stopped = !isProcessGroupAlive(pgid);
  return result;
}

async function waitForProcessGroupStopped(pgid, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (!isProcessGroupAlive(pgid)) {
      return true;
    }
    await delay(100);
  }
  return !isProcessGroupAlive(pgid);
}

function sendProcessGroupSignal(pgid, signal) {
  try {
    process.kill(process.platform === 'win32' ? pgid : -pgid, signal);
  } catch (error) {
    if (error?.code !== 'ESRCH') {
      throw error;
    }
  }
}

async function instanceStatus(config) {
  const specs = managedProcessSpecs(config);
  const pidRecords = new Map();
  for (const spec of specs) {
    pidRecords.set(spec.name, await readPidMetadata(config, spec.name));
  }
  const pidProcesses = new Map();
  for (const pidRecord of pidRecords.values()) {
    if (pidRecord.pid !== null && isProcessAlive(pidRecord.pid) && !pidProcesses.has(pidRecord.pid)) {
      pidProcesses.set(pidRecord.pid, await inspectProcess(pidRecord.pid));
    }
  }
  const listenerDiscovery = await discoverPortListeners(specs.flatMap((spec) => spec.ports));
  const ownedGroups = ownedProcessGroups(config, specs, pidRecords, pidProcesses);
  return {
    configPath: config.paths.configPath,
    instanceRoot: config.paths.instanceRoot,
    instanceId: instanceId(config),
    urls: {
      routerHttp: config.urls.routerHttp,
      routerControl: config.urls.routerControl,
      routerReload: config.urls.routerReload,
      telemetry: config.urls.telemetry,
    },
    listenerDiscovery: {
      available: listenerDiscovery.available,
      errors: listenerDiscovery.errors,
    },
    processes: specs.map((spec) =>
      processStatus(config, spec, pidRecords.get(spec.name), listenerDiscovery.byPort, ownedGroups, specs, pidProcesses)),
  };
}

function processStatus(config, spec, pidRecord, listenersByPort, ownedGroups, specs, pidProcesses) {
  const metadata = pidRecord.metadata;
  const sameInstance = metadata !== null && isSameInstanceMetadata(config, metadata);
  const pid = pidRecord.pid;
  const pidProcess = pid === null ? null : pidProcesses.get(pid) ?? null;
  const pidProcessOwner = pidProcess === null ? null : ownedComponentFromCommand(config, specs, pidProcess.command);
  const pidAlive = pid === null ? false : isProcessAlive(pid);
  const inspectedPgid = pidProcess === null ? null : readPositiveInteger(pidProcess.pgid);
  const jsonPidMatchesSpec = pidRecord.format === 'json' && sameInstance && pidAlive && pidProcessOwner === spec.name;
  const legacyPlainPidMatchesSpec = pidRecord.format === 'plain' && pidAlive && pidProcessOwner === spec.name;
  const trustedRecordedPidMatchesSpec = jsonPidMatchesSpec || legacyPlainPidMatchesSpec;
  const pgid = trustedRecordedPidMatchesSpec ? inspectedPgid : null;
  const processGroupAlive = pgid === null ? null : isProcessGroupAlive(pgid);
  const metadataMatchesSpec = sameInstance ? pidMetadataMatchesSpec(metadata, spec) : legacyPlainPidMatchesSpec;
  const repairableGroups = new Set();
  if (trustedRecordedPidMatchesSpec && Number.isInteger(pgid) && pgid > 0 && (pidAlive || processGroupAlive)) {
    repairableGroups.add(pgid);
  }
  if (!sameInstance && pidProcessOwner === spec.name && Number.isInteger(pidProcess.pgid) && pidProcess.pgid > 0) {
    repairableGroups.add(pidProcess.pgid);
  }
  const ports = spec.ports.map((port) => {
    const listeners = (listenersByPort.get(port) ?? []).map((listener) => {
      const ownership = listenerOwnership(config, spec, specs, listener, ownedGroups, {
        componentPgid: pgid,
        commandMatchIsComponent: legacyPlainPidMatchesSpec,
      });
      if (ownership !== 'unknown' && Number.isInteger(listener.pgid) && listener.pgid > 0) {
        repairableGroups.add(listener.pgid);
      }
      return { ...listener, ownership };
    });
    return { port, listeners };
  });
  const allListeners = ports.flatMap((port) => port.listeners);
  const unknownPortConflicts = allListeners.filter((listener) => listener.ownership === 'unknown');
  const ownedPortConflicts = allListeners.filter((listener) => listener.ownership.startsWith('other-owned-component:'));
  const sameInstanceOrphanListeners = allListeners.filter((listener) => listener.ownership === 'same-instance-orphan');
  const missingPorts = ports
    .filter((port) => !port.listeners.some((listener) => listener.ownership === 'component'))
    .map((port) => port.port);
  const jsonMetadataRunning = jsonPidMatchesSpec
    && processGroupAlive !== false
    && metadataMatchesSpec
    && missingPorts.length === 0;
  const legacyPlainRunning = legacyPlainPidMatchesSpec
    && processGroupAlive !== false
    && missingPorts.length === 0;

  let category;
  if (unknownPortConflicts.length > 0 || ownedPortConflicts.length > 0) {
    category = 'port-conflict';
  } else if (jsonMetadataRunning || legacyPlainRunning) {
    category = 'running';
  } else if (pidRecord.format === 'missing' && allListeners.length === 0) {
    category = 'stopped';
  } else if (pidRecord.format !== 'missing' && !pidAlive && allListeners.length === 0) {
    category = 'stale-pid';
  } else if (
    sameInstanceOrphanListeners.length > 0
    || (!sameInstance && pidProcessOwner === spec.name)
    || (sameInstance && processGroupAlive && !pidAlive)
  ) {
    category = 'orphaned';
  } else if (pidRecord.format === 'missing') {
    category = 'stopped';
  } else {
    category = 'unhealthy';
  }

  return {
    name: spec.name,
    category,
    running: category === 'running',
    pid,
    pgid,
    pidAlive,
    pidProcess,
    processGroupAlive,
    metadataMatchesSpec,
    pidPath: pidRecord.path,
    pidMetadata: {
      format: pidRecord.format,
      sameInstance,
      metadata,
      error: pidRecord.error,
    },
    ports,
    missingPorts,
    repairableGroups: [...repairableGroups].sort((left, right) => left - right),
    logPath: join(config.paths.logDir, `${spec.name}.log`),
    errorLogPath: join(config.paths.logDir, `${spec.name}.err.log`),
  };
}

function listenerOwnership(config, spec, specs, listener, ownedGroups, options = {}) {
  if (listener.pgid !== null) {
    const groupOwners = ownedGroups.get(listener.pgid);
    if (groupOwners !== undefined && groupOwners.has(spec.name)) {
      return 'component';
    }
    if (options.componentPgid !== null && options.componentPgid !== undefined && listener.pgid === options.componentPgid) {
      return 'component';
    }
    if (groupOwners !== undefined && groupOwners.size > 0) {
      return `other-owned-component:${formatOwnedComponentNames(groupOwners)}`;
    }
  }
  const commandOwner = ownedComponentFromCommand(config, specs, listener.command);
  if (commandOwner !== null) {
    if (commandOwner === spec.name) {
      return options.commandMatchIsComponent === true ? 'component' : 'same-instance-orphan';
    }
    return `other-owned-component:${commandOwner}`;
  }
  return 'unknown';
}

function ownedProcessGroups(config, specs, pidRecords, pidProcesses) {
  const groups = new Map();
  for (const spec of specs) {
    const pidRecord = pidRecords.get(spec.name);
    const metadata = pidRecord?.metadata;
    if (metadata !== null && metadata !== undefined && isSameInstanceMetadata(config, metadata)) {
      const owner = pidProcessOwner(config, specs, spec, pidRecord, pidProcesses);
      if (owner?.component === spec.name) {
        addOwnedGroup(groups, owner.pgid, spec.name);
      }
    }
    if (pidRecord?.format === 'plain' && pidRecord.pid !== null) {
      const owner = pidProcessOwner(config, specs, spec, pidRecord, pidProcesses);
      if (owner?.component === spec.name) {
        addOwnedGroup(groups, owner.pgid, spec.name);
      }
    }
  }
  return groups;
}

function pidProcessOwner(config, specs, spec, pidRecord, pidProcesses) {
  if (pidRecord?.pid === null || pidRecord?.pid === undefined) {
    return null;
  }
  const pidProcess = pidProcesses.get(pidRecord.pid);
  if (pidProcess === undefined) {
    return null;
  }
  const component = ownedComponentFromCommand(config, specs, pidProcess.command);
  if (component !== spec.name) {
    return null;
  }
  const pgid = readPositiveInteger(pidProcess.pgid);
  return pgid === null ? null : { component, pgid };
}

function addOwnedGroup(groups, pgid, owner) {
  const groupId = readPositiveInteger(pgid);
  if (groupId === null) {
    return;
  }
  if (!groups.has(groupId)) {
    groups.set(groupId, new Set());
  }
  groups.get(groupId).add(owner);
}

function formatOwnedComponentNames(owners) {
  return [...owners].sort().join(',');
}

function ownedComponentFromCommand(config, specs, command) {
  if (command === undefined || command.length === 0) {
    return null;
  }
  const tokens = tokenizeCommandLine(command);
  for (const spec of specs) {
    if (commandMatchesComponent(config, spec, tokens)) {
      return spec.name;
    }
  }
  return null;
}

function commandMatchesComponent(config, spec, tokens) {
  switch (spec.name) {
    case 'mongo':
      return commandLooksLikeMongo(config, tokens);
    case 'telemetry':
      return commandLooksLikePnpmDev(tokens, join(skiffRoot, 'telemetry'), config.paths.telemetryConfig)
        || commandLooksLikeTsxService(tokens, join(skiffRoot, 'telemetry', 'src', 'main.ts'), 'src/main.ts', config.paths.telemetryConfig);
    case 'router':
      return commandLooksLikePnpmDev(tokens, join(skiffRoot, 'router'), config.paths.routerConfig)
        || commandLooksLikeTsxService(tokens, join(skiffRoot, 'router', 'src', 'router', 'server.ts'), 'src/router/server.ts', config.paths.routerConfig);
    case 'runtime':
      return commandLooksLikeRuntime(config, tokens);
    case 'watch':
      return commandLooksLikeWatch(config, tokens);
    default:
      return false;
  }
}

function commandLooksLikePnpmDev(tokens, projectDir, configPath) {
  return commandStartsWithPnpm(tokens)
    && hasPathOptionValue(tokens, '--dir', projectDir)
    && tokens.includes('dev')
    && hasPathOptionValue(tokens, '--config', configPath);
}

function commandLooksLikeTsxService(tokens, absoluteEntry, relativeEntry, configPath) {
  return hasEntryToken(tokens, absoluteEntry, relativeEntry)
    && hasPathOptionValue(tokens, '--config', configPath);
}

function commandLooksLikeMongo(config, tokens) {
  const executableName = tokens[0] === undefined ? '' : basename(tokens[0]);
  return (executableName === 'mongod' || executableName === basename(config.mongo.binary))
    && hasPathOptionValue(tokens, '--dbpath', config.paths.serviceDbPath)
    && hasOptionValue(tokens, '--port', String(config.ports.mongo));
}

function commandLooksLikeRuntime(config, tokens) {
  const commandIndex = nodeWrapperScriptIndex(tokens);
  return commandIndex !== null
    && pathTokenMatches(tokens[commandIndex], config.paths.runtimeBinary)
    && tokens.some((token, index) => index !== commandIndex && pathTokenMatches(token, config.paths.runtimeConfig));
}

function commandLooksLikeWatch(config, tokens) {
  const commandIndex = nodeWrapperScriptIndex(tokens);
  return commandIndex !== null
    && hasEntryTokenAt(tokens, commandIndex, join(scriptDir, 'skiff-dev-sync.mjs'), 'scripts/skiff-dev-sync.mjs')
    && tokens.includes('--watch')
    && hasPathOptionValue(tokens, '--config', config.paths.watchConfig);
}

function commandStartsWithPnpm(tokens) {
  const commandIndex = nodeWrapperScriptIndex(tokens);
  return commandIndex !== null && basename(tokens[commandIndex]) === 'pnpm';
}

function nodeWrapperScriptIndex(tokens) {
  if (tokens[0] === undefined) {
    return null;
  }
  if (isNodeExecutableName(basename(tokens[0]))) {
    return tokens[1] === undefined ? null : 1;
  }
  return 0;
}

function isNodeExecutableName(name) {
  return name === 'node' || name === 'node.exe';
}

function hasEntryToken(tokens, absoluteEntry, relativeEntry) {
  return tokens.some((token) =>
    pathTokenMatches(token, absoluteEntry)
    || token === relativeEntry
    || token.endsWith(`/${relativeEntry}`));
}

function hasEntryTokenAt(tokens, index, absoluteEntry, relativeEntry) {
  const token = tokens[index];
  return token !== undefined
    && (pathTokenMatches(token, absoluteEntry)
      || token === relativeEntry
      || token.endsWith(`/${relativeEntry}`));
}

function hasPathToken(tokens, expectedPath) {
  return tokens.some((token) => pathTokenMatches(token, expectedPath));
}

function hasPathOptionValue(tokens, option, expectedPath) {
  return optionValues(tokens, option).some((value) => pathTokenMatches(value, expectedPath));
}

function hasOptionValue(tokens, option, expectedValue) {
  return optionValues(tokens, option).some((value) => value === expectedValue);
}

function optionValues(tokens, option) {
  const values = [];
  const prefix = `${option}=`;
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (token === option && tokens[index + 1] !== undefined) {
      values.push(tokens[index + 1]);
      index += 1;
    } else if (token.startsWith(prefix)) {
      values.push(token.slice(prefix.length));
    }
  }
  return values;
}

function pathTokenMatches(token, expectedPath) {
  if (token === expectedPath) {
    return true;
  }
  if (!token.startsWith('/')) {
    return false;
  }
  return resolve(token) === resolve(expectedPath);
}

function tokenizeCommandLine(command) {
  const tokens = [];
  let current = '';
  let quote = null;
  let escaping = false;
  for (const char of command) {
    if (escaping) {
      current += char;
      escaping = false;
      continue;
    }
    if (char === '\\') {
      escaping = true;
      continue;
    }
    if (quote !== null) {
      if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (/\s/.test(char)) {
      if (current.length > 0) {
        tokens.push(current);
        current = '';
      }
      continue;
    }
    current += char;
  }
  if (escaping) {
    current += '\\';
  }
  if (current.length > 0) {
    tokens.push(current);
  }
  return tokens;
}

async function discoverPortListeners(ports) {
  const byPort = new Map();
  const errors = [];
  let available = true;
  for (const port of [...new Set(ports)].sort((left, right) => left - right)) {
    const result = await capture('lsof', ['-nP', `-tiTCP:${port}`, '-sTCP:LISTEN']);
    if (result.error !== undefined) {
      available = false;
      errors.push(`lsof failed for port ${port}: ${result.error}`);
      byPort.set(port, []);
      continue;
    }
    if (result.code !== 0 && result.stdout.trim().length === 0) {
      byPort.set(port, []);
      continue;
    }
    if (result.code !== 0) {
      errors.push(`lsof returned ${result.code} for port ${port}: ${result.stderr.trim()}`);
    }
    const pids = [...new Set(result.stdout
      .split(/\s+/)
      .map((value) => Number(value))
      .filter((value) => Number.isInteger(value) && value > 0))];
    byPort.set(port, await Promise.all(pids.map(async (pid) => ({
      port,
      ...await inspectProcess(pid),
    }))));
  }
  return { available, errors, byPort };
}

async function inspectProcess(pid) {
  const fallback = { pid, ppid: null, pgid: null, commandName: null, command: '', alive: isProcessAlive(pid) };
  const result = await capture('ps', ['-o', 'pid=', '-o', 'ppid=', '-o', 'pgid=', '-o', 'comm=', '-o', 'command=', '-p', String(pid)]);
  if (result.error !== undefined || result.code !== 0) {
    return fallback;
  }
  const line = result.stdout.split(/\r?\n/).find((item) => item.trim().length > 0);
  if (line === undefined) {
    return fallback;
  }
  const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s*(.*)$/.exec(line);
  if (!match) {
    return { ...fallback, command: line.trim() };
  }
  return {
    pid: Number(match[1]),
    ppid: Number(match[2]),
    pgid: Number(match[3]),
    commandName: match[4],
    command: match[5].trim(),
    alive: true,
  };
}

function hasUnknownPortConflict(processStatus) {
  return processStatus.ports.some((port) =>
    port.listeners.some((listener) => listener.ownership === 'unknown'));
}

function isRepairableProcess(processStatus) {
  return ['orphaned', 'port-conflict', 'unhealthy'].includes(processStatus.category)
    && processStatus.repairableGroups.length > 0
    && !hasUnknownPortConflict(processStatus);
}

function renderProcessStatusLine(processStatus) {
  const pid = processStatus.pid === null ? '' : ` pid=${processStatus.pid}`;
  const pgid = processStatus.pgid === null ? '' : ` pgid=${processStatus.pgid}`;
  const pidFormat = processStatus.pidMetadata.format === 'missing'
    ? ''
    : ` pidFile=${processStatus.pidMetadata.format}`;
  return `${processStatus.name}: ${processStatus.category}${pid}${pgid}${pidFormat}`;
}

function recommendationsForProcess(processStatus) {
  if (processStatus.category === 'running') {
    return [];
  }
  if (processStatus.category === 'stopped') {
    return ['start with skiff instance up <config>'];
  }
  if (processStatus.category === 'stale-pid') {
    return ['run skiff instance repair <config> to remove stale pid metadata'];
  }
  if (hasUnknownPortConflict(processStatus)) {
    return ['unknown process owns a required port; stop it manually or change the instance port'];
  }
  if (isRepairableProcess(processStatus)) {
    return ['run skiff instance repair <config> or skiff instance up <config> --repair-owned-conflicts'];
  }
  return ['inspect pid metadata and component logs'];
}

function pidPath(config, name) {
  return join(config.paths.pidDir, `${name}.pid`);
}

async function readPidMetadata(config, name) {
  const path = pidPath(config, name);
  try {
    const text = await readFile(path, 'utf8');
    const trimmed = text.trim();
    if (trimmed.startsWith('{')) {
      const metadata = JSON.parse(trimmed);
      return {
        path,
        format: 'json',
        metadata,
        pid: readPositiveInteger(metadata.pid),
        error: null,
      };
    }
    const pid = Number(trimmed);
    return {
      path,
      format: Number.isInteger(pid) && pid > 0 ? 'plain' : 'invalid',
      metadata: null,
      pid: Number.isInteger(pid) && pid > 0 ? pid : null,
      error: Number.isInteger(pid) && pid > 0 ? null : 'invalid plain pid',
    };
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return { path, format: 'missing', metadata: null, pid: null, error: null };
    }
    if (error instanceof SyntaxError) {
      return { path, format: 'invalid', metadata: null, pid: null, error: error.message };
    }
    throw error;
  }
}

async function writePidMetadata(config, spec, pid) {
  const metadata = {
    schemaVersion: pidMetadataSchemaVersion,
    component: spec.name,
    pid,
    pgid: pid,
    instanceId: instanceId(config),
    configPath: config.paths.configPath,
    instanceRoot: config.paths.instanceRoot,
    command: spec.command,
    args: spec.args,
    cwd: spec.cwd,
    ports: spec.ports,
    startedAt: new Date().toISOString(),
  };
  await writeFile(pidPath(config, spec.name), `${JSON.stringify(metadata, null, 2)}\n`);
}

function isSameInstanceMetadata(config, metadata) {
  return metadata?.schemaVersion === pidMetadataSchemaVersion
    && metadata.instanceId === instanceId(config)
    && resolve(metadata.configPath) === config.paths.configPath
    && resolve(metadata.instanceRoot) === config.paths.instanceRoot;
}

function pidMetadataMatchesSpec(metadata, spec) {
  return metadata.component === spec.name
    && metadata.command === spec.command
    && metadata.cwd === spec.cwd
    && arraysEqual(metadata.args, spec.args)
    && arraysEqual(metadata.ports, spec.ports);
}

function instanceId(config) {
  return createHash('sha256')
    .update(config.paths.configPath)
    .update('\0')
    .update(config.paths.instanceRoot)
    .digest('hex')
    .slice(0, 24);
}

function readPositiveInteger(value) {
  return Number.isInteger(value) && value > 0 ? value : null;
}

function arraysEqual(left, right) {
  return Array.isArray(left)
    && Array.isArray(right)
    && left.length === right.length
    && left.every((value, index) => value === right[index]);
}

function isProcessAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

function isProcessGroupAlive(pgid) {
  try {
    process.kill(process.platform === 'win32' ? pgid : -pgid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

async function writeIfMissing(path, contents, force) {
  await mkdir(dirname(path), { recursive: true });
  if (!force && await fileExists(path)) {
    return { action: 'kept', path };
  }
  await writeFile(path, contents);
  return { action: force ? 'wrote' : 'created', path };
}

async function assertFile(path) {
  const info = await stat(path);
  if (!info.isFile()) {
    throw new Error(`${path} must be a file`);
  }
}

async function fileExists(path) {
  try {
    await access(path, fsConstants.F_OK);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function parseInstanceConfig(rawArgs) {
  const args = [];
  let configPath;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--config') {
      configPath = resolve(requireNext(rawArgs, index, '--config'));
      index += 1;
      continue;
    }
    if (arg.startsWith('--config=')) {
      configPath = resolve(arg.slice('--config='.length));
      continue;
    }
    args.push(arg);
  }
  const subcommand = args[0];
  if (subcommand === undefined || subcommand === '-h' || subcommand === '--help') {
    return { configPath, args };
  }
  if (!instanceCommands.has(subcommand)) {
    return { configPath, args };
  }
  if (!configPath) {
    const positionalConfig = args[1];
    if (!positionalConfig || positionalConfig.startsWith('-')) {
      throw new Error(`skiff instance ${subcommand} requires <config>`);
    }
    configPath = resolve(positionalConfig);
    args.splice(1, 1);
  }
  return { configPath, args };
}

function parseFlags(rawArgs, spec) {
  const flags = new Set();
  const positionals = [];
  for (const arg of rawArgs) {
    if (spec.flags.has(arg)) {
      flags.add(arg);
      continue;
    }
    if (arg.startsWith('-')) {
      throw new Error(`unknown option ${arg}`);
    }
    positionals.push(arg);
  }
  return { flags, positionals };
}

function requireNext(args, index, optionName) {
  const value = args[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${optionName} requires a value`);
  }
  return value;
}

function capture(command, args, options = {}) {
  return new Promise((resolvePromise) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', (error) => {
      resolvePromise({ code: null, stdout, stderr, error: error.message });
    });
    child.on('exit', (code, signal) => {
      resolvePromise({ code: code ?? -1, signal, stdout, stderr });
    });
  });
}

function run(command, args, cwd, env = process.env) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd,
      env,
      stdio: 'inherit',
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolvePromise();
        return;
      }
      reject(new Error(`${command} exited with ${signal ?? code}`));
    });
  });
}
