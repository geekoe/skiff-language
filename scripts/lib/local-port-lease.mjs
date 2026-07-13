import { randomBytes, randomInt } from 'node:crypto';
import { createConnection, createServer } from 'node:net';
import { mkdir, open, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';

const DEFAULT_LEASE_DIR = join(tmpdir(), 'skiff-local-port-leases');

export async function leaseConsecutiveLocalPorts({
  rangeStart,
  rangeEnd,
  count,
  attempts = 500,
  leaseDir = DEFAULT_LEASE_DIR,
} = {}) {
  assertPortRange(rangeStart, rangeEnd, count);
  const maximumBase = rangeEnd - count + 1;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    const base = randomInt(rangeStart, maximumBase + 1);
    const ports = Array.from({ length: count }, (_, index) => base + index);
    try {
      return await leaseLocalPorts(ports, { leaseDir });
    } catch {
      // Another owner or listener may have won this candidate. Try another block.
    }
  }
  throw new Error(`no local port block of ${count} available in ${rangeStart}-${rangeEnd}`);
}

export async function leaseLocalPorts(ports, { leaseDir = DEFAULT_LEASE_DIR } = {}) {
  const candidates = [...new Set(ports)];
  if (candidates.length !== ports.length || candidates.some((port) => !validPort(port))) {
    throw new Error('local port lease candidates must be distinct integer ports');
  }
  await mkdir(leaseDir, { recursive: true });
  const token = randomBytes(16).toString('hex');
  const metadata = `${JSON.stringify({
    schemaVersion: 'skiff-local-port-lease-v1',
    pid: process.pid,
    token,
    ports: candidates,
    createdAt: new Date().toISOString(),
  })}\n`;
  const handles = [];
  try {
    for (const port of candidates) {
      const path = join(leaseDir, `${port}.lock`);
      const handle = await openLeaseFile(path);
      handles.push({ handle, path, port });
      await handle.writeFile(metadata, 'utf8');
      await handle.sync();
      await assertPortAvailable(port);
    }
  } catch (error) {
    await releaseLeaseHandles(handles, token);
    throw error;
  }

  let released = false;
  return {
    ports: candidates,
    async release() {
      if (released) {
        return;
      }
      released = true;
      await releaseLeaseHandles(handles, token);
    },
  };
}

export async function assertPortsClosed(ports, { attempts = 40, delayMs = 100 } = {}) {
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    const openPorts = [];
    for (const port of ports) {
      if (await canConnect(port)) {
        openPorts.push(port);
      }
    }
    if (openPorts.length === 0) {
      return;
    }
    await delay(delayMs);
  }
  throw new Error(`managed process left a listener on one of: ${ports.join(', ')}`);
}

async function openLeaseFile(path) {
  try {
    return await open(path, 'wx');
  } catch (error) {
    if (error?.code !== 'EEXIST' || !await removeStaleLease(path)) {
      throw error;
    }
    return open(path, 'wx');
  }
}

async function removeStaleLease(path) {
  let metadata;
  try {
    metadata = JSON.parse(await readFile(path, 'utf8'));
  } catch {
    return false;
  }
  if (metadata?.schemaVersion !== 'skiff-local-port-lease-v1'
    || !Number.isInteger(metadata.pid)
    || metadata.pid <= 0) {
    return false;
  }
  if (processAlive(metadata.pid)) {
    return false;
  }
  await rm(path, { force: true });
  return true;
}

async function releaseLeaseHandles(handles, token) {
  const errors = [];
  for (const { handle, path } of handles.reverse()) {
    try {
      await handle.close();
    } catch (error) {
      errors.push(error);
    }
    try {
      const metadata = JSON.parse(await readFile(path, 'utf8'));
      if (metadata?.token === token) {
        await rm(path, { force: true });
      }
    } catch (error) {
      if (error?.code !== 'ENOENT') {
        errors.push(error);
      }
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'failed to release local port lease');
  }
}

function assertPortAvailable(port) {
  return new Promise((resolvePromise, reject) => {
    const server = createServer();
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => {
      server.close(resolvePromise);
    });
  });
}

function canConnect(port) {
  return new Promise((resolvePromise) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.setTimeout(100);
    socket.once('connect', () => {
      socket.destroy();
      resolvePromise(true);
    });
    socket.once('timeout', () => {
      socket.destroy();
      resolvePromise(false);
    });
    socket.once('error', () => resolvePromise(false));
  });
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === 'EPERM';
  }
}

function assertPortRange(rangeStart, rangeEnd, count) {
  if (!validPort(rangeStart) || !validPort(rangeEnd) || rangeStart > rangeEnd) {
    throw new Error('local port lease range must use valid ascending ports');
  }
  if (!Number.isInteger(count) || count < 1 || rangeStart + count - 1 > rangeEnd) {
    throw new Error('local port lease count must fit within the configured range');
  }
}

function validPort(port) {
  return Number.isInteger(port) && port > 0 && port <= 65535;
}
