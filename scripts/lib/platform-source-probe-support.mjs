import { createHash, randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { constants as fsConstants } from 'node:fs';
import {
  access,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  stat,
  statfs,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, isAbsolute, join, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';

import { assertPortsClosed } from './local-port-lease.mjs';

const LEASE_DIR = join(tmpdir(), 'skiff-local-port-leases');

export function createProbeDependencies(overrides) {
  return {
    signalTarget: process,
    runCommand: captureOwnedCommand,
    makeTempRoot: (prefix) => mkdtemp(prefix),
    mkdir,
    remove: (path) => rm(path, { recursive: true, force: true }),
    exists: pathExists,
    canonicalPath: realpath,
    availableBytes: diskAvailableBytes,
    allocatedBytes: allocatedDirectoryBytes,
    snapshotArtifacts,
    loadRegistry,
    readText: (path) => readFile(path, 'utf8'),
    readLedger: async (path) => JSON.parse(await readFile(path, 'utf8')),
    writeLedger: atomicWriteLedger,
    assertExecutables,
    ...overrides,
  };
}

async function assertExecutables(names, env, cwd) {
  const missing = [];
  for (const name of names) {
    let found = false;
    for (const directory of (env.PATH ?? '').split(delimiter)) {
      const root = isAbsolute(directory) ? directory : resolve(cwd, directory || '.');
      const candidate = join(root, name);
      try {
        if ((await stat(candidate)).isFile()) {
          await access(candidate, fsConstants.X_OK);
          found = true;
          break;
        }
      } catch {}
    }
    if (!found) missing.push(name);
  }
  if (missing.length > 0) {
    throw new Error(`probe is missing required executable(s): ${missing.join(', ')}`);
  }
}

export function finalizeProbeDigest(ledger) {
  const { ledgerDigest: _old, ...body } = ledger;
  return { ...body, ledgerDigest: probeDigest(body) };
}

export function probeDigest(value) {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

export function commandText(outcome) {
  return `${outcome.stdout ?? ''}\n${outcome.stderr ?? ''}`;
}

export function commandFailure(command, outcome) {
  return new Error(
    `${command} failed (${outcome.signal ?? outcome.code ?? 'spawn'}): ${commandText(outcome).trim()}`,
  );
}

export function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

async function snapshotArtifacts(targetRoot) {
  const debugRoot = join(targetRoot, 'debug');
  const files = await walkFiles(debugRoot);
  const selected = files.filter((path) => artifactTraits(path) !== null);
  const result = [];
  for (const path of selected.sort()) {
    const traits = artifactTraits(path);
    const metadata = await stat(path);
    const contents = await readFile(path);
    result.push({
      path,
      sha256: createHash('sha256').update(contents).digest('hex'),
      mtimeMs: metadata.mtimeMs,
      size: metadata.size,
      ...traits,
    });
  }
  return result;
}

function artifactTraits(path) {
  const name = path.split(sep).at(-1);
  const depInfo = name.endsWith('.d')
    && /(?:skiff[-_](?:compiler|test[-_]runner|package[-_]service[-_]smoke[-_]fixture)|package_service_contract_deployment)/
      .test(name);
  if (depInfo) return { depInfo: true, structureSubject: false, identityTest: false };
  const structureSubject = name === 'skiff-compiler'
    || name === 'skiff-test-runner'
    || name === 'skiff-package-service-smoke-fixture'
    || /^libskiff_compiler(?:_input|_source)?-[^.]+\.rlib$/.test(name);
  const identityTest = /^package_service_contract_deployment-[^.]+$/.test(name);
  return structureSubject || identityTest
    ? { depInfo: false, structureSubject, identityTest }
    : null;
}

async function walkFiles(root) {
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch (error) {
    if (error?.code === 'ENOENT') return [];
    throw error;
  }
  const files = [];
  for (const entry of entries) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...await walkFiles(path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

async function loadRegistry(root, candidate) {
  const url = pathToFileURL(join(root, 'scripts/lib/skiff-source-test-registry.mjs'));
  url.searchParams.set('candidate', candidate);
  return (await import(url.href)).canonicalSkiffSourceTestRegistry;
}

async function captureOwnedCommand(command, args, {
  cwd,
  env = process.env,
  signal,
  observePorts = false,
} = {}) {
  signal?.throwIfAborted();
  const detached = process.platform !== 'win32';
  const child = spawn(command, args, {
    cwd,
    env,
    detached,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  let running = true;
  const observedPorts = new Set();
  const observer = observePorts ? observeLeasePorts(child.pid, observedPorts, () => running) : null;
  let forceKillTimer;
  const abort = () => {
    terminate(child, detached, 'SIGTERM');
    forceKillTimer ??= setTimeout(() => terminate(child, detached, 'SIGKILL'), 5_000);
  };
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) abort();
  const { code, closeSignal, error } = await new Promise((resolvePromise) => {
    let spawnError = null;
    child.once('error', (value) => { spawnError = value; });
    child.once('close', (childCode, childSignal) => resolvePromise({
      code: childCode,
      closeSignal: childSignal,
      error: spawnError,
    }));
  });
  running = false;
  if (observer !== null) await observer;
  if (forceKillTimer !== undefined) clearTimeout(forceKillTimer);
  signal?.removeEventListener('abort', abort);
  const ports = [...observedPorts].sort((left, right) => left - right);
  const processGroupAbsent = await retireProcessGroup(child.pid, detached);
  let portsAbsent = true;
  if (ports.length > 0) {
    try { await assertPortsClosed(ports); } catch { portsAbsent = false; }
  }
  return {
    code: error === null ? code : null,
    signal: closeSignal,
    error,
    stdout,
    stderr,
    pid: child.pid,
    processGroupAbsent,
    observedPorts: ports,
    portsAbsent,
  };
}

async function observeLeasePorts(pid, ports, running) {
  while (running()) {
    let entries = [];
    try { entries = await readdir(LEASE_DIR); } catch {}
    for (const entry of entries.filter((name) => name.endsWith('.lock'))) {
      try {
        const metadata = JSON.parse(await readFile(join(LEASE_DIR, entry), 'utf8'));
        if (metadata.pid === pid && Array.isArray(metadata.ports)) {
          for (const port of metadata.ports) ports.add(port);
        }
      } catch {}
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 25));
  }
}

function terminate(child, detached, signal) {
  if (!Number.isInteger(child.pid)) return;
  try { process.kill(detached ? -child.pid : child.pid, signal); } catch {}
}

async function retireProcessGroup(pid, detached) {
  if (!processGroupAlive(pid, detached)) return true;
  for (const [signal, attempts] of [['SIGTERM', 20], ['SIGKILL', 20]]) {
    try { process.kill(detached ? -pid : pid, signal); } catch {}
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      if (!processGroupAlive(pid, detached)) return true;
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 50));
    }
  }
  return !processGroupAlive(pid, detached);
}

function processGroupAlive(pid, detached) {
  if (!Number.isInteger(pid)) return false;
  try {
    process.kill(detached ? -pid : pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function allocatedDirectoryBytes(root) {
  let metadata;
  try { metadata = await lstat(root); } catch (error) {
    if (error?.code === 'ENOENT') return undefined;
    throw error;
  }
  let total = metadata.blocks * 512;
  if (!metadata.isDirectory()) return total;
  for (const entry of await readdir(root)) {
    total += await allocatedDirectoryBytes(join(root, entry)) ?? 0;
  }
  return total;
}

async function diskAvailableBytes(path) {
  const value = await statfs(path, { bigint: true });
  const bytes = value.bavail * value.bsize;
  return bytes > BigInt(Number.MAX_SAFE_INTEGER) ? Number.MAX_SAFE_INTEGER : Number(bytes);
}

async function atomicWriteLedger(path, ledger) {
  const temporary = `${path}.${process.pid}.${randomBytes(8).toString('hex')}.tmp`;
  await writeFile(temporary, `${JSON.stringify(ledger, null, 2)}\n`, { flag: 'wx' });
  try {
    await rename(temporary, path);
  } catch (error) {
    await rm(temporary, { force: true });
    throw error;
  }
}

async function pathExists(path) {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error?.code === 'ENOENT') return false;
    throw error;
  }
}
