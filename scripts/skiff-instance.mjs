#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { constants as fsConstants } from 'node:fs';
import { access, chmod, copyFile, mkdir, open, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
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
const allProcessNames = ['mongo', 'telemetry', 'router', 'runtime', 'watch'];
const instanceCommands = new Set(['init', 'paths', 'status', 'build', 'up', 'run', 'down', 'reload', 'sync', 'watch']);
const usage = `usage:
  skiff instance init <config> [--force]
  skiff instance paths <config> [--json]
  skiff instance status <config> [--json]
  skiff instance build <config>
  skiff instance up <config>
  skiff instance run <config>
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
    case 'build':
      await buildInstance(args, configPath);
      return;
    case 'up':
      await upInstance(args, configPath);
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
    const pid = processStatus.pid === null ? '' : ` pid=${processStatus.pid}`;
    console.log(`${processStatus.name}: ${processStatus.running ? 'running' : 'stopped'}${pid}`);
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
  const args = parseFlags(rawArgs, { flags: new Set() });
  if (args.positionals.length !== 0) {
    throw new Error(`unexpected argument ${args.positionals[0]}`);
  }
  const config = await loadInstance(configPath);
  await ensureInstanceDirs(config.paths);
  await writeRuntimeConfigs(config, true);
  await buildComponentBinaries(config);

  for (const spec of managedProcessSpecs(config)) {
    await startManagedProcess(config, spec);
  }

  console.log(JSON.stringify(await instanceStatus(config), null, 2));
}

async function runInstance(rawArgs, configPath) {
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
  for (const name of [...allProcessNames].reverse()) {
    stopped.push(await stopManagedProcess(config, name));
  }
  console.log(JSON.stringify({ stopped }, null, 2));
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
    serviceDbMongoUrl: 'mongodb://127.0.0.1:27017/?directConnection=true&replicaSet=rs0&retryWrites=false',
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
  if (config.components.runtime === 'stable') {
    await copyBinary(config.stable.runtimeBinary, config.paths.runtimeBinary);
  } else {
    await buildRustBinary({
      manifest: join(skiffRoot, 'runtime', 'Cargo.toml'),
      bin: 'runtime',
      source: join(config.paths.cargoTargetDir, 'debug', process.platform === 'win32' ? 'runtime.exe' : 'runtime'),
      destination: config.paths.runtimeBinary,
      config,
    });
  }

  if (config.components.identityCli === 'stable') {
    await copyBinary(config.stable.identityCli, config.paths.identityCli);
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
      }]
      : []),
    ...(config.components.telemetry === 'disabled'
      ? []
      : [{
        name: 'telemetry',
        command: 'pnpm',
        args: ['--dir', join(skiffRoot, 'telemetry'), 'dev', '--config', config.paths.telemetryConfig],
        cwd: skiffRoot,
      }]),
    {
      name: 'router',
      command: 'pnpm',
      args: ['--dir', join(skiffRoot, 'router'), 'dev', '--config', config.paths.routerConfig],
      cwd: skiffRoot,
    },
    {
      name: 'runtime',
      command: config.paths.runtimeBinary,
      args: [config.paths.runtimeConfig],
      cwd: skiffRoot,
    },
    ...(config.components.watch === 'managed'
      ? [{
        name: 'watch',
        command: 'node',
        args: [join(scriptDir, 'skiff-dev-sync.mjs'), '--watch', '--config', config.paths.watchConfig],
        cwd: skiffRoot,
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

async function startManagedProcess(config, spec) {
  const current = await readPid(config, spec.name);
  if (current !== null && isProcessAlive(current)) {
    console.log(`[skiff-instance] ${spec.name} already running pid=${current}`);
    return;
  }
  await rm(pidPath(config, spec.name), { force: true });
  const out = await open(join(config.paths.logDir, `${spec.name}.log`), 'a');
  const err = await open(join(config.paths.logDir, `${spec.name}.err.log`), 'a');
  const child = spawn(spec.command, spec.args, {
    cwd: spec.cwd,
    env: processEnv(config),
    detached: true,
    stdio: ['ignore', out.fd, err.fd],
  });
  child.unref();
  await writeFile(pidPath(config, spec.name), `${child.pid}\n`);
  await delay(250);
  if (!isProcessAlive(child.pid)) {
    throw new Error(`${spec.name} exited after start; see ${join(config.paths.logDir, `${spec.name}.err.log`)}`);
  }
  console.log(`[skiff-instance] started ${spec.name} pid=${child.pid}`);
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
      const entry = running.get(spec.name);
      if (entry) {
        entry.child.kill('SIGTERM');
      } else {
        await stopManagedProcess(config, spec.name);
      }
    }
    await delay(500);
    for (const spec of [...specs].reverse()) {
      const entry = running.get(spec.name);
      if (entry && isProcessAlive(entry.child.pid)) {
        entry.child.kill('SIGKILL');
      }
      await rm(pidPath(config, spec.name), { force: true });
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
    await rm(pidPath(config, spec.name), { force: true });
    const out = await open(join(config.paths.logDir, `${spec.name}.log`), 'a');
    const err = await open(join(config.paths.logDir, `${spec.name}.err.log`), 'a');
    const child = spawn(spec.command, spec.args, {
      cwd: spec.cwd,
      env: processEnv(config),
      detached: false,
      stdio: ['ignore', out.fd, err.fd],
    });
    running.set(spec.name, { child, out, err });
    await writeFile(pidPath(config, spec.name), `${child.pid}\n`);
    console.log(`[skiff-instance] supervised ${spec.name} pid=${child.pid}`);
    child.on('exit', (code, signal) => {
      const entry = running.get(spec.name);
      running.delete(spec.name);
      void entry?.out.close();
      void entry?.err.close();
      void rm(pidPath(config, spec.name), { force: true });
      if (!stopping) {
        console.warn(`[skiff-instance] ${spec.name} exited with ${signal ?? code}; restarting`);
        setTimeout(() => {
          void start(spec);
        }, 1000);
      }
    });
  };

  for (const spec of specs) {
    await start(spec);
  }
  console.log(JSON.stringify(await instanceStatus(config), null, 2));
  await new Promise(() => {});
}

async function stopManagedProcess(config, name) {
  const pid = await readPid(config, name);
  if (pid === null) {
    return { name, pid: null, stopped: false, reason: 'missing-pid' };
  }
  if (!isProcessAlive(pid)) {
    await rm(pidPath(config, name), { force: true });
    return { name, pid, stopped: false, reason: 'not-running' };
  }
  process.kill(pid, 'SIGTERM');
  for (let attempt = 0; attempt < 20; attempt += 1) {
    await delay(100);
    if (!isProcessAlive(pid)) {
      await rm(pidPath(config, name), { force: true });
      return { name, pid, stopped: true };
    }
  }
  process.kill(pid, 'SIGKILL');
  await rm(pidPath(config, name), { force: true });
  return { name, pid, stopped: true, forced: true };
}

async function instanceStatus(config) {
  return {
    configPath: config.paths.configPath,
    instanceRoot: config.paths.instanceRoot,
    urls: {
      routerHttp: config.urls.routerHttp,
      routerControl: config.urls.routerControl,
      routerReload: config.urls.routerReload,
      telemetry: config.urls.telemetry,
    },
    processes: await Promise.all(managedProcessSpecs(config).map(async ({ name }) => {
      const pid = await readPid(config, name);
      return {
        name,
        pid,
        running: pid !== null && isProcessAlive(pid),
        pidPath: pidPath(config, name),
        logPath: join(config.paths.logDir, `${name}.log`),
        errorLogPath: join(config.paths.logDir, `${name}.err.log`),
      };
    })),
  };
}

function pidPath(config, name) {
  return join(config.paths.pidDir, `${name}.pid`);
}

async function readPid(config, name) {
  try {
    const text = await readFile(pidPath(config, name), 'utf8');
    const pid = Number(text.trim());
    return Number.isInteger(pid) && pid > 0 ? pid : null;
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  }
}

function isProcessAlive(pid) {
  try {
    process.kill(pid, 0);
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
