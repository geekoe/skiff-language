#!/usr/bin/env node
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { access, mkdir, open, readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { parseSimpleYamlObject } from './lib/simple-yaml.mjs';

export const ACTIONS = ['start', 'stop', 'restart', 'status', 'logs'];

const skiffRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const START_PID_POLL_MS = 2_000;
const STOP_WAIT_MS = 5_000;
const STOP_KILL_WAIT_MS = 2_000;
const CARGO_METADATA_TIMEOUT_MS = 20_000;

export function componentInfo(component) {
  switch (component) {
    case 'router':
      return {
        crate: 'skiff-router',
        bin: 'skiff-router',
        manifest: 'router/Cargo.toml',
      };
    case 'runtime':
      return {
        crate: 'runtime',
        bin: 'runtime',
        manifest: 'runtime/Cargo.toml',
      };
    default:
      throw new Error(`unknown component ${JSON.stringify(component)}; expected router|runtime`);
  }
}

export function parseCliArgs(argv) {
  if (argv.length < 2) {
    throw new Error('missing required arguments: <component> <action> --dir <run dir>');
  }
  const [component, action] = argv;
  componentInfo(component);
  if (!ACTIONS.includes(action)) {
    throw new Error(`unknown action ${JSON.stringify(action)}; expected ${ACTIONS.join('|')}`);
  }
  let dir = null;
  for (let index = 2; index < argv.length; index += 1) {
    if (argv[index] === '--dir') {
      dir = argv[index + 1];
      index += 1;
      continue;
    }
    throw new Error(`unexpected argument ${argv[index]}`);
  }
  if (dir === null || dir.length === 0) {
    throw new Error('missing required --dir <run dir>');
  }
  return { component, action, dir };
}

export async function loadProcessYml(runDir) {
  const ymlPath = join(runDir, 'process.yml');
  const content = await readMaybe(ymlPath);
  if (content === null) {
    return null;
  }
  const parsed = parseSimpleYamlObject(content, ymlPath);
  const result = { binary: null, env: {} };
  if (typeof parsed.binary === 'string' && parsed.binary.length > 0) {
    result.binary = parsed.binary;
  }
  if (parsed.env !== undefined && parsed.env !== null && typeof parsed.env === 'object') {
    for (const [key, value] of Object.entries(parsed.env)) {
      if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
        result.env[key] = String(value);
      }
    }
  }
  return result;
}

export function mergeEnv(baseEnv, overrides = {}) {
  return { ...baseEnv, ...overrides };
}

export function pickBinaryPath(processBinary, cargoTargetBinary, buildBinary) {
  for (const candidate of [processBinary, cargoTargetBinary, buildBinary]) {
    if (typeof candidate === 'string' && candidate.length > 0) {
      return candidate;
    }
  }
  return null;
}

export async function componentProfile(runDir, component) {
  const content = await readMaybe(join(runDir, `${component}.yml`));
  if (content === null) {
    return 'debug';
  }
  try {
    const parsed = parseSimpleYamlObject(content, join(runDir, `${component}.yml`));
    if (typeof parsed.profile === 'string' && parsed.profile.length > 0) {
      return parsed.profile;
    }
  } catch {
    return 'debug';
  }
  return 'debug';
}

export function runCargoMetadata({ manifest, env }) {
  const result = spawnSync(
    'cargo',
    ['metadata', '--no-deps', '--format-version', '1', '--manifest-path', manifest],
    { encoding: 'utf8', env, timeout: CARGO_METADATA_TIMEOUT_MS },
  );
  if (result.error !== undefined || result.status !== 0) {
    return null;
  }
  try {
    return JSON.parse(result.stdout);
  } catch {
    return null;
  }
}

export async function resolveComponentBinary({
  component,
  runDir,
  skiffRoot: root = skiffRoot,
  env = process.env,
  loadMetadata = runCargoMetadata,
}) {
  const info = componentInfo(component);
  const processYml = await loadProcessYml(runDir);
  const processBinary = processYml === null ? null : processYml.binary;
  if (processBinary !== null) {
    return { binary: processBinary, source: 'process.yml' };
  }
  const runDirExists = await pathExists(runDir);
  if (runDirExists) {
    const profile = await componentProfile(runDir, component);
    const metadata = await loadMetadata({ manifest: join(root, info.manifest), env });
    const targetDirectory = metadata === null ? null : metadata.target_directory;
    if (typeof targetDirectory === 'string' && targetDirectory.length > 0) {
      const targetBinary = join(targetDirectory, profile, info.bin);
      if (await pathExists(targetBinary)) {
        return { binary: targetBinary, source: 'cargo' };
      }
    }
  }
  const buildBinary = join(root, 'build', 'bin', info.bin);
  if (await pathExists(buildBinary)) {
    return { binary: buildBinary, source: 'build' };
  }
  return null;
}

export async function readPidFile(runDir, component) {
  const content = await readMaybe(join(runDir, `${component}.pid`));
  if (content === null) {
    return null;
  }
  const pid = Number(content.trim());
  return Number.isInteger(pid) && pid > 0 ? pid : null;
}

export function pidAlive(pid) {
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

export function buildBinaryArgs(info, configPath) {
  return [configPath];
}

export function tailLines(content, maxLines) {
  return content.split(/\r?\n/).slice(-maxLines);
}

export async function sha256OfFile(filePath) {
  try {
    const hash = createHash('sha256');
    await new Promise((resolvePromise, reject) => {
      const stream = createReadStream(filePath);
      stream.on('data', (chunk) => hash.update(chunk));
      stream.on('end', resolvePromise);
      stream.on('error', reject);
    });
    return hash.digest('hex');
  } catch {
    return null;
  }
}

async function actionStart({ component, runDir, root }) {
  const info = componentInfo(component);
  const configPath = join(runDir, `${component}.yml`);
  if (!(await pathExists(configPath))) {
    throw new Error(`config file not found: ${configPath}`);
  }
  const resolved = await resolveComponentBinary({ component, runDir, skiffRoot: root });
  if (resolved === null || !(await pathExists(resolved.binary))) {
    throw new Error(
      `binary not found for ${component}; set binary: in ${join(runDir, 'process.yml')} or build via skiff build`,
    );
  }
  const existingPid = await readPidFile(runDir, component);
  if (existingPid !== null && pidAlive(existingPid)) {
    throw new Error(`already running (pid ${existingPid})`);
  }
  await mkdir(runDir, { recursive: true });
  const processYml = await loadProcessYml(runDir);
  const env = mergeEnv(process.env, processYml === null ? {} : processYml.env);
  const outLog = await open(join(runDir, `${component}.out.log`), 'a');
  const errLog = await open(join(runDir, `${component}.err.log`), 'a');
  const child = spawn(resolved.binary, buildBinaryArgs(info, configPath), {
    cwd: root,
    env,
    stdio: ['ignore', outLog.fd, errLog.fd],
    detached: true,
  });
  child.on('error', (error) => {
    console.error(`skiff-process: ${component}: spawn failed: ${error.message}`);
    process.exitCode = 1;
  });
  child.unref();
  await Promise.all([outLog.close(), errLog.close()]);
  await delay(300);
  const pid = await pollPidFile(runDir, component, START_PID_POLL_MS);
  return pid === null ? `started ${component}` : `started ${component} (pid ${pid})`;
}

async function actionStop({ component, runDir }) {
  const pid = await readPidFile(runDir, component);
  if (pid === null || !pidAlive(pid)) {
    return 'not running';
  }
  try {
    process.kill(pid, 'SIGTERM');
  } catch (error) {
    if (error?.code !== 'ESRCH') {
      throw error;
    }
  }
  if (await waitForExit(pid, STOP_WAIT_MS)) {
    return `stopped ${component} (pid ${pid})`;
  }
  try {
    process.kill(pid, 'SIGKILL');
  } catch (error) {
    if (error?.code !== 'ESRCH') {
      throw error;
    }
  }
  await waitForExit(pid, STOP_KILL_WAIT_MS);
  return `stopped ${component} (pid ${pid})`;
}

async function actionRestart({ component, runDir, root }) {
  const stopMessage = await actionStop({ component, runDir });
  const startMessage = await actionStart({ component, runDir, root });
  if (stopMessage === 'not running') {
    return startMessage;
  }
  return `${stopMessage}\n${startMessage}`;
}

async function actionStatus({ component, runDir, root }) {
  const pid = await readPidFile(runDir, component);
  const result = {
    component,
    runDir,
    pid,
    alive: pid !== null && pidAlive(pid),
  };
  const resolved = await resolveComponentBinary({ component, runDir, skiffRoot: root });
  if (resolved !== null) {
    result.binary = resolved.binary;
    const sha256 = await sha256OfFile(resolved.binary);
    if (sha256 !== null) {
      result.binarySha256 = sha256;
    }
  }
  return JSON.stringify(result, null, 2);
}

async function actionLogs({ component, runDir }) {
  const outPath = join(runDir, `${component}.out.log`);
  const errPath = join(runDir, `${component}.err.log`);
  let found = false;
  const outContent = await readMaybe(outPath);
  if (outContent !== null) {
    found = true;
    for (const line of tailLines(outContent, 200)) {
      console.log(line);
    }
  }
  const errContent = await readMaybe(errPath);
  if (errContent !== null) {
    found = true;
    const lines = tailLines(errContent, 50).filter((line) => line.length > 0);
    for (const line of lines) {
      console.log(`[err] ${line}`);
    }
  }
  if (!found) {
    return `no log files in ${runDir} (expected ${outPath})`;
  }
  return null;
}

async function pollPidFile(runDir, component, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const pid = await readPidFile(runDir, component);
    if (pid !== null) {
      return pid;
    }
    if (Date.now() >= deadline) {
      return null;
    }
    await delay(100);
  }
}

async function waitForExit(pid, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (!pidAlive(pid)) {
      return true;
    }
    if (Date.now() >= deadline) {
      return false;
    }
    await delay(100);
  }
}

async function readMaybe(filePath) {
  try {
    return await readFile(filePath, 'utf8');
  } catch (error) {
    if (error?.code === 'ENOENT') {
      return null;
    }
    throw error;
  }
}

async function pathExists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

async function main(argv) {
  let parsed;
  try {
    parsed = parseCliArgs(argv);
  } catch (error) {
    console.error(error.message);
    console.error('usage: node scripts/skiff-process.mjs <router|runtime> <start|stop|restart|status|logs> --dir <run dir>');
    process.exitCode = 1;
    return;
  }
  const { component, action, dir } = parsed;
  const runDir = resolve(dir);
  try {
    let result;
    switch (action) {
      case 'start':
        result = await actionStart({ component, runDir, root: skiffRoot });
        break;
      case 'stop':
        result = await actionStop({ component, runDir });
        break;
      case 'restart':
        result = await actionRestart({ component, runDir, root: skiffRoot });
        break;
      case 'status':
        result = await actionStatus({ component, runDir, root: skiffRoot });
        break;
      case 'logs':
        result = await actionLogs({ component, runDir });
        break;
      default:
        throw new Error(`unknown action ${JSON.stringify(action)}`);
    }
    if (result !== null) {
      console.log(result);
    }
  } catch (error) {
    console.error(`skiff-process: ${component} ${action}: ${error.message}`);
    process.exitCode = 1;
  }
}

const isDirectRun =
  process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isDirectRun) {
  main(process.argv.slice(2));
}
