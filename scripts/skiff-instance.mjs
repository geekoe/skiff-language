#!/usr/bin/env node
// `skiff instance` — lightweight local process supervisor.
//
// Reads the generated instance.yml from the runtime-stack artifact directory
// (produced by `skiff stack build --profile debug`) and only maintains the
// processes it describes. It never reads a configDir, never renders configs,
// never builds binaries, and never manages watch.

import { spawn as spawnManagedChild } from 'node:child_process';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { connect } from 'node:net';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { parseStackYaml } from './lib/stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');
const DEFAULT_RUNTIME_DIR = join(skiffRoot, 'build', 'runtime-stack');
const STARTUP_TIMEOUT_MS = 30_000;

const usage = `usage:
  skiff instance <up|restart|status|down|supervise|repair> [--runtime <dir>] [--startup-gate <file>] [--startup-ready <file>] [component]`;

try {
  await main(process.argv.slice(2));
} catch (error) {
  console.error(`error: ${error?.message || String(error)}`);
  process.exitCode = 1;
}

async function main(rawArgs) {
  const command = rawArgs.shift();
  if (!command || command === '-h' || command === '--help') {
    console.log(usage);
    return;
  }
  const parsed = parseArgs(rawArgs);
  const spec = await loadInstanceSpec(parsed.runtimeDir);
  switch (command) {
    case 'up':
      await upInstance(spec);
      return;
    case 'restart':
      await restartInstance(spec, parsed.component);
      return;
    case 'status':
      console.log(JSON.stringify(await instanceStatus(spec), null, 2));
      return;
    case 'down':
      await downInstance(spec, parsed.component);
      return;
    case 'repair':
      await repairInstance(spec);
      return;
    case 'supervise':
      await superviseInstance(spec, parsed.startupGate, parsed.startupReady);
      return;
    default:
      throw new Error(`unknown instance command ${command}\n${usage}`);
  }
}

function parseArgs(rawArgs) {
  let runtimeDir = DEFAULT_RUNTIME_DIR;
  let component;
  let startupGate;
  let startupReady;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    if (argument === '--runtime') {
      const value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--runtime requires a directory path');
      }
      runtimeDir = resolve(value);
      index += 1;
    } else if (argument.startsWith('--runtime=')) {
      runtimeDir = resolve(argument.slice('--runtime='.length));
    } else if (argument === '--startup-gate') {
      startupGate = resolve(requireValue(rawArgs, ++index, argument));
    } else if (argument.startsWith('--startup-gate=')) {
      startupGate = resolve(argument.slice('--startup-gate='.length));
    } else if (argument === '--startup-ready') {
      startupReady = resolve(requireValue(rawArgs, ++index, argument));
    } else if (argument.startsWith('--startup-ready=')) {
      startupReady = resolve(argument.slice('--startup-ready='.length));
    } else if (component === undefined) {
      component = argument;
    } else {
      throw new Error(`unexpected argument ${argument}\n${usage}`);
    }
  }
  return { runtimeDir, component, startupGate, startupReady };
}

function requireValue(rawArgs, index, label) {
  const value = rawArgs[index];
  if (!value || value.startsWith('--')) {
    throw new Error(`${label} requires a value`);
  }
  return value;
}

async function loadInstanceSpec(runtimeDir) {
  const file = join(runtimeDir, 'instance.yml');
  let source;
  try {
    source = await readFile(file, 'utf8');
  } catch (error) {
    throw new Error(
      `instance.yml not found at ${file}; run "skiff stack build --configDir <dir> --profile debug" first`,
      { cause: error },
    );
  }
  const spec = parseStackYaml(source, 'instance.yml');
  if (spec.schemaVersion !== 'skiff-instance-v1') {
    throw new Error(`instance.yml schemaVersion must be skiff-instance-v1`);
  }
  if (!Array.isArray(spec.processes) || spec.processes.length === 0) {
    throw new Error('instance.yml processes must be a non-empty array');
  }
  for (const process of spec.processes) {
    if (
      typeof process?.name !== 'string'
      || typeof process?.command !== 'string'
      || !Array.isArray(process?.args)
    ) {
      throw new Error('instance.yml process must declare name, command, and args');
    }
  }
  return {
    ...spec,
    runtimeDir,
    pidDir: resolve(spec.pidDir),
    logDir: resolve(spec.logDir),
  };
}

async function upInstance(spec, { only } = {}) {
  const targets = only === undefined
    ? spec.processes
    : spec.processes.filter((process) => process.name === only);
  if (targets.length === 0) {
    throw new Error(`unknown instance component ${only}`);
  }
  for (const process of targets) {
    if (await isProcessRunning(spec, process)) {
      continue;
    }
    await startProcess(spec, process);
  }
  await waitForRouterHealth(spec);
}

async function restartInstance(spec, component) {
  await downInstance(spec, component);
  await upInstance(spec, { only: component });
}

async function downInstance(spec, component) {
  const targets = component === undefined
    ? [...spec.processes].reverse()
    : spec.processes.filter((process) => process.name === component).reverse();
  for (const process of targets) {
    await stopProcess(spec, process);
  }
}

async function repairInstance(spec) {
  for (const process of spec.processes) {
    if (!await isProcessRunning(spec, process)) {
      await startProcess(spec, process);
    }
  }
  const router = spec.processes.find((process) => process.name === 'router');
  if (router !== undefined && !await routerHealthy(spec, router)) {
    await stopProcess(spec, router);
    await startProcess(spec, router);
  }
  await waitForRouterHealth(spec);
}

async function superviseInstance(spec, startupGate, startupReady) {
  const gate = startupGate;
  const ready = startupReady;
  const preRouter = spec.processes.filter((process) => process.name !== 'router' && process.name !== 'runtime');
  const routerAndRuntime = spec.processes.filter((process) => process.name === 'router' || process.name === 'runtime');
  for (const process of preRouter) {
    if (!await isProcessRunning(spec, process)) {
      await startProcess(spec, process);
    }
  }
  if (ready !== undefined) {
    await mkdir(dirname(ready), { recursive: true });
    await writeFile(ready, 'ready\n');
  }
  if (gate !== undefined) {
    await waitForFile(gate, STARTUP_TIMEOUT_MS, `startup gate ${gate}`);
  }
  for (const process of routerAndRuntime) {
    if (!await isProcessRunning(spec, process)) {
      await startProcess(spec, process);
    }
  }
  await waitForRouterHealth(spec);
  console.log('instance supervising; Ctrl-C to stop');
  let stopping = false;
  process.on('SIGINT', async () => {
    if (stopping) {
      return;
    }
    stopping = true;
    await downInstance(spec);
    process.exit(0);
  });
  for (;;) {
    await sleep(5000);
    for (const process of spec.processes) {
      if (!await isProcessRunning(spec, process)) {
        console.error(`[instance] ${process.name} exited; restarting`);
        await startProcess(spec, process);
      }
    }
  }
}

async function waitForFile(file, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      await readFile(file);
      return;
    } catch {
      await sleep(100);
    }
  }
  throw new Error(`timed out waiting for ${label}`);
}

async function instanceStatus(spec) {
  const processes = [];
  for (const process of spec.processes) {
    const pid = await readPid(spec, process.name);
    const alive = pid !== null && isAlive(pid);
    const ports = [];
    for (const port of process.ports ?? []) {
      ports.push({ port, open: await isPortOpen(port) });
    }
    processes.push({
      name: process.name,
      pid,
      alive,
      ports,
    });
  }
  const router = spec.processes.find((process) => process.name === 'router');
  let health = null;
  if (router !== undefined) {
    health = await routerHealth(spec, router);
  }
  return {
    schemaVersion: 'skiff-instance-status-v1',
    profile: spec.profile,
    runtimeDir: spec.runtimeDir,
    processes,
    routerHealth: health,
  };
}

async function startProcess(spec, entry) {
  await mkdir(spec.pidDir, { recursive: true });
  await mkdir(spec.logDir, { recursive: true });
  const { open } = await import('node:fs/promises');
  const out = await open(join(spec.logDir, `${entry.name}.out.log`), 'a');
  const err = await open(join(spec.logDir, `${entry.name}.err.log`), 'a');
  // child-process-owner: instance-managed-component
  const child = spawnManagedChild(entry.command, entry.args, {
    cwd: entry.cwd ?? spec.runtimeDir,
    env: { ...globalThis.process.env, ...(spec.env ?? {}) },
    stdio: ['ignore', out.fd, err.fd],
  });
  child.unref();
  out.close().catch(() => {});
  err.close().catch(() => {});
  await writeFile(pidPath(spec, entry.name), String(child.pid));
  child.on('exit', () => {
    rm(pidPath(spec, entry.name), { force: true }).catch(() => {});
  });
  console.error(`[instance] ${entry.name} started (pid ${child.pid})`);
}

async function stopProcess(spec, entry) {
  const pid = await readPid(spec, entry.name);
  if (pid === null || !isAlive(pid)) {
    await rm(pidPath(spec, entry.name), { force: true });
    return;
  }
  try {
    process.kill(pid, 'SIGTERM');
  } catch {
    return;
  }
  for (let attempt = 0; attempt < 30; attempt += 1) {
    if (!isAlive(pid)) {
      await rm(pidPath(spec, entry.name), { force: true });
      console.error(`[instance] ${entry.name} stopped`);
      return;
    }
    await sleep(100);
  }
  try {
    process.kill(pid, 'SIGKILL');
  } catch {
    // already gone
  }
  await rm(pidPath(spec, entry.name), { force: true });
  console.error(`[instance] ${entry.name} killed`);
}

async function waitForRouterHealth(spec) {
  const router = spec.processes.find((process) => process.name === 'router');
  if (router === undefined) {
    return;
  }
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await routerHealthy(spec, router)) {
      return;
    }
    await sleep(500);
  }
  throw new Error(`router did not become healthy within ${STARTUP_TIMEOUT_MS}ms`);
}

async function routerHealthy(spec, router) {
  const health = await routerHealth(spec, router);
  return health?.ok === true && health?.activeAssembly?.profile === spec.profile;
}

async function routerHealth(spec, router) {
  if (router.healthUrl === undefined || router.healthUrl === null) {
    return null;
  }
  try {
    const response = await fetch(router.healthUrl);
    if (!response.ok) {
      return null;
    }
    return await response.json();
  } catch {
    return null;
  }
}

async function isProcessRunning(spec, process) {
  const pid = await readPid(spec, process.name);
  return pid !== null && isAlive(pid);
}

function pidPath(spec, name) {
  return join(spec.pidDir, `${name}.pid`);
}

async function readPid(spec, name) {
  try {
    const value = Number((await readFile(pidPath(spec, name), 'utf8')).trim());
    return Number.isSafeInteger(value) && value > 0 ? value : null;
  } catch {
    return null;
  }
}

function isAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function isPortOpen(port) {
  return new Promise((resolvePromise) => {
    const socket = connect({ port, host: '127.0.0.1' });
    const done = (value) => {
      socket.destroy();
      resolvePromise(value);
    };
    socket.once('connect', () => done(true));
    socket.once('error', () => done(false));
    socket.setTimeout(500, () => done(false));
  });
}

function sleep(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}
