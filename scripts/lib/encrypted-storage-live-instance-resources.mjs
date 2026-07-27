import { randomInt } from 'node:crypto';
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { leaseLocalPorts } from './local-port-lease.mjs';

const PORT_MIN = 45000;
const PORT_MAX = 45999;
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);

export async function createEncryptedStorageLiveInstanceResources({
  repoRoot,
  environment,
  randomPort = randomInt,
  leasePorts = leaseLocalPorts,
  makeTempDirectory = mkdtemp,
  makeDirectory = mkdir,
  writeTextFile = writeFile,
  temporaryDirectory = tmpdir(),
} = {}) {
  const portLease = await leaseIsolatedPorts({ randomPort, leasePorts });
  const tempRoot = await makeTempDirectory(
    join(temporaryDirectory, 'skiff-encrypted-storage-live-'),
  );
  const instanceRoot = join(tempRoot, 'instance');
  const configPath = join(instanceRoot, 'config.yml');
  const paths = {
    tempRoot,
    instanceRoot,
    configPath,
    devHome: join(instanceRoot, 'dev-home'),
    artifactRoot: join(instanceRoot, 'dev-home', 'artifacts'),
    keyring: join(
      instanceRoot,
      'dev-home',
      'secrets',
      'service-db-keyring.json',
    ),
    runtimeLog: join(instanceRoot, 'logs', 'runtime.log'),
    runtimeErrorLog: join(instanceRoot, 'logs', 'runtime.err.log'),
    routerLog: join(instanceRoot, 'logs', 'router.log'),
    routerErrorLog: join(instanceRoot, 'logs', 'router.err.log'),
    fixtureRoot: join(repoRoot, 'runtime', 'encrypted-storage-live'),
  };
  await makeDirectory(instanceRoot, { recursive: true });
  await writeTextFile(
    configPath,
    encryptedStorageLiveInstanceConfigText({
      repoRoot,
      environment,
      ports: portLease.ports,
    }),
    'utf8',
  );
  return { paths, portLease };
}

export function encryptedStorageLiveInstanceConfigText({
  repoRoot,
  environment,
  ports,
}) {
  return [
    `environment: ${environment}`,
    'devHome: dev-home',
    `cargoTargetDir: ${JSON.stringify(join(repoRoot, 'build', 'cargo-target'))}`,
    'ports:',
    `  base: ${ports.base}`,
    `  mongo: ${ports.mongo}`,
    'http:',
    '  maxRequestBytes: 67108864',
    '  maxResponseBytes: 8388608',
    'components:',
    '  telemetry: disabled',
    '  mongo: managed',
    '  watch: disabled',
    'telemetry:',
    '  memory: true',
    'mongo:',
    '  binary: mongod',
    '  dbPath: service-db',
    'watch:',
    '  config: watch.json',
    '',
  ].join('\n');
}

export function isEncryptedStorageLivePortForbidden(port) {
  return port < PORT_MIN || port > PORT_MAX || FORBIDDEN_PORTS.has(port);
}

export function createEncryptedStorageLiveOwnedProcessGroupStopper({
  readDirectory = readdir,
  readTextFile = readFile,
  killProcess = process.kill,
  wait = delay,
} = {}) {
  const signalProcessGroup = (pgid, signal) => {
    try {
      killProcess(-pgid, signal);
    } catch (error) {
      if (error.code !== 'ESRCH') {
        throw error;
      }
    }
  };
  const processGroupAlive = (pgid) => {
    try {
      killProcess(-pgid, 0);
      return true;
    } catch (error) {
      return error.code !== 'ESRCH';
    }
  };
  const waitForProcessGroups = async (groups, attempts) => {
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      if (!groups.some(processGroupAlive)) {
        return;
      }
      await wait(100);
    }
  };

  return async function stopEncryptedStorageLiveOwnedProcessGroups({
    instanceRoot,
    configPath,
    onValidated,
  }) {
    let entries;
    try {
      entries = await readDirectory(join(instanceRoot, 'pids'));
    } catch (error) {
      if (error.code === 'ENOENT') {
        onValidated(undefined);
        return;
      }
      throw error;
    }
    const groups = [];
    for (const entry of entries) {
      if (!entry.endsWith('.pid')) {
        continue;
      }
      const raw = await readTextFile(
        join(instanceRoot, 'pids', entry),
        'utf8',
      );
      const metadata = JSON.parse(raw);
      if (
        metadata.configPath !== configPath
        || metadata.instanceRoot !== instanceRoot
        || !Number.isInteger(metadata.pgid)
        || metadata.pgid <= 1
      ) {
        throw new Error(`refusing to stop unowned process metadata ${entry}`);
      }
      groups.push(metadata.pgid);
    }
    onValidated(groups);
    for (const pgid of groups) {
      signalProcessGroup(pgid, 'SIGTERM');
    }
    await waitForProcessGroups(groups, 40);
    for (const pgid of groups.filter(processGroupAlive)) {
      signalProcessGroup(pgid, 'SIGKILL');
    }
    await waitForProcessGroups(groups, 20);
    const survivors = groups.filter(processGroupAlive);
    if (survivors.length > 0) {
      throw new Error(
        `owned process groups did not stop: ${survivors.join(', ')}`,
      );
    }
  };
}

export const stopEncryptedStorageLiveOwnedProcessGroups =
  createEncryptedStorageLiveOwnedProcessGroupStopper();

async function leaseIsolatedPorts({ randomPort, leasePorts }) {
  for (let attempt = 0; attempt < 500; attempt += 1) {
    const base = 45000 + randomPort(0, 400);
    const mongo = 45500 + randomPort(0, 500);
    const candidates = [base, base + 1, base + 2, mongo];
    if (
      new Set(candidates).size !== candidates.length
      || candidates.some(isEncryptedStorageLivePortForbidden)
    ) {
      continue;
    }
    try {
      const lease = await leasePorts(candidates);
      return {
        ports: { base, mongo },
        release: () => lease.release(),
      };
    } catch {
      // Try another disjoint port set.
    }
  }
  throw new Error(`no isolated ports available in ${PORT_MIN}-${PORT_MAX}`);
}

function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}
