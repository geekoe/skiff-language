// Skiff dev router/runtime supervisor (launchd-owned foreground loop).
//
// The router/runtime binaries daemonize (skiff-process.mjs spawns them
// detached and exits), so a launchd job calling `skiff <component> start`
// is one-shot and cannot supervise the daemon. This script is the launchd
// job: it polls the run-dir pidfiles and restarts a component when it is
// stale or dead, respecting dependencies (router needs Mongo, runtime
// needs the router control port).
//
// It never fights a manual `skiff <component> restart --dir`: the pidfile
// is checked before starting, so a component that is already alive is left
// alone; a stale pidfile (dead daemon) falls through to `start`.
//
// Logs go to stdout/stderr (plist StandardOutPath/StandardErrorPath).

import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { connect } from 'node:net';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const SKIFF_ROOT = process.env.SKIFF_ROOT ?? join(SCRIPT_DIR, '..');

const MONGO_HOST = '127.0.0.1';
const MONGO_PORT = 27017;
const ROUTER_HEALTH_URL = 'http://127.0.0.1:4001/__router/health';
const COMPONENT_RUN_DIRS = {
  router: process.env.SKIFF_ROUTER_DIR ?? join(SKIFF_ROOT, '.skiff-dev/router'),
  runtime: process.env.SKIFF_RUNTIME_DIR ?? join(SKIFF_ROOT, '.skiff-dev/runtime'),
};
const SUPERVISE_INTERVAL_MS = 10_000;
const PROBE_TIMEOUT_MS = 2_000;

function log(message) {
  console.log(`[skiff-supervise] ${new Date().toISOString()} ${message}`);
}

function delay(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

async function readPid(runDir, component) {
  try {
    const content = await readFile(join(runDir, `${component}.pid`), 'utf8');
    const pid = Number(content.trim());
    return Number.isInteger(pid) && pid > 0 ? pid : null;
  } catch {
    return null;
  }
}

function pidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) {
    return false;
  }
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

function probeTcp(host, port) {
  return new Promise((resolvePromise) => {
    const socket = connect({ host, port });
    const onResult = (ok) => {
      socket.destroy();
      resolvePromise(ok);
    };
    socket.setTimeout(PROBE_TIMEOUT_MS);
    socket.once('connect', () => onResult(true));
    socket.once('timeout', () => onResult(false));
    socket.once('error', () => onResult(false));
  });
}

async function waitForMongo() {
  while (!(await probeTcp(MONGO_HOST, MONGO_PORT))) {
    log(`mongo ${MONGO_HOST}:${MONGO_PORT} not reachable; retrying in 5s`);
    await delay(5_000);
  }
}

async function waitForRouterHealth() {
  while (true) {
    try {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);
      const response = await fetch(ROUTER_HEALTH_URL, { signal: controller.signal });
      clearTimeout(timer);
      if (response.ok) {
        return;
      }
    } catch {
      // retry
    }
    log(`router control ${ROUTER_HEALTH_URL} not healthy; retrying in 5s`);
    await delay(5_000);
  }
}

async function startComponent(component, runDir) {
  log(`starting ${component} (${runDir})`);
  await new Promise((resolvePromise) => {
    // child-process-owner: skiff-supervise-spawn
    const child = spawn(process.execPath, [
      join(SCRIPT_DIR, 'skiff-process.mjs'),
      component,
      'start',
      '--dir',
      runDir,
    ], {
      cwd: SKIFF_ROOT,
      stdio: 'inherit',
    });
    child.on('error', (error) => {
      log(`start ${component} failed to spawn: ${error.message}`);
      resolvePromise();
    });
    child.on('exit', (code) => {
      log(`start ${component} exited with code ${code ?? 'null'}`);
      resolvePromise();
    });
  });
}

async function ensureComponent(component) {
  const runDir = COMPONENT_RUN_DIRS[component];
  const pid = await readPid(runDir, component);
  if (pid !== null && pidAlive(pid)) {
    return true;
  }
  if (pid === null) {
    log(`${component} has no pidfile; starting`);
  } else {
    log(`${component} pidfile pid ${pid} is dead; starting`);
  }
  await startComponent(component, runDir);
  return false;
}

async function superviseLoop() {
  while (true) {
    try {
      await waitForMongo();
      await ensureComponent('router');
      await waitForRouterHealth();
      await ensureComponent('runtime');
    } catch (error) {
      log(`supervise iteration failed: ${error?.message ?? error}`);
    }
    await delay(SUPERVISE_INTERVAL_MS);
  }
}

await superviseLoop();
