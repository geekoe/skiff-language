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
  profile,
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
  const configPath = join(instanceRoot, 'instance.yml');
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
    encryptedStorageLiveInstanceYml({
      repoRoot,
      profile,
      ports: portLease.ports,
    }),
    'utf8',
  );
  return { paths, portLease };
}

export function encryptedStorageLiveInstanceYml({
  repoRoot,
  profile,
  ports,
}) {
  const httpPort = ports.base;
  const controlPort = ports.base + 1;
  const devHome = join('dev-home');
  const pidDir = join(devHome, 'pids');
  const logDir = join(devHome, 'logs');
  return [
    'schemaVersion: skiff-instance-v1',
    `profile: ${profile}`,
    `devHome: ${JSON.stringify(devHome)}`,
    `artifactRoot: ${JSON.stringify(join(devHome, 'artifacts'))}`,
    `pidDir: ${JSON.stringify(pidDir)}`,
    `logDir: ${JSON.stringify(logDir)}`,
    `mongoDbPath: ${JSON.stringify(join(devHome, 'mongo-data'))}`,
    `activationUrl: ${JSON.stringify(`http://127.0.0.1:${controlPort}/__skiff/activate-assembly`)}`,
    'processes:',
    '  - name: mongo',
    '    command: mongod',
    '    args:',
    '      - --dbpath',
    `      - ${JSON.stringify(join(devHome, 'mongo-data'))}`,
    '      - --port',
    `      - ${JSON.stringify(String(ports.mongo))}`,
    '      - --replSet',
    '      - rs0',
    '      - --bind_ip',
    '      - 127.0.0.1',
    `    cwd: ${JSON.stringify(devHome)}`,
    `    ports: [${ports.mongo}]`,
    '    healthUrl: null',
    '  - name: router',
    `    command: ${JSON.stringify(join(repoRoot, 'build', 'runtime-stack', 'bin', 'skiff-router'))}`,
    '    args:',
    `      - ${JSON.stringify(join(devHome, 'router.yml'))}`,
    `    cwd: ${JSON.stringify(devHome)}`,
    `    ports: [${httpPort}, ${controlPort}]`,
    `    healthUrl: ${JSON.stringify(`http://127.0.0.1:${controlPort}/__router/health`)}`,
    '  - name: runtime',
    `    command: ${JSON.stringify(join(repoRoot, 'build', 'runtime-stack', 'bin', 'skiff-runtime'))}`,
    '    args:',
    `      - ${JSON.stringify(join(devHome, 'runtime.yml'))}`,
    `    cwd: ${JSON.stringify(devHome)}`,
    '    ports: []',
    '    healthUrl: null',
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
  const signalProcess = (pid, signal) => {
    try {
      killProcess(pid, signal);
    } catch (error) {
      if (error.code !== 'ESRCH') {
        throw error;
      }
    }
  };
  const processAlive = (pid) => {
    try {
      killProcess(pid, 0);
      return true;
    } catch (error) {
      return error.code !== 'ESRCH';
    }
  };
  const waitForProcesses = async (pids, attempts) => {
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      if (!pids.some(processAlive)) {
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
    const pids = [];
    for (const entry of entries) {
      if (!entry.endsWith('.pid')) {
        continue;
      }
      const raw = await readTextFile(
        join(instanceRoot, 'pids', entry),
        'utf8',
      );
      const pid = Number(raw.trim());
      if (!Number.isSafeInteger(pid) || pid <= 1) {
        throw new Error(`refusing to stop invalid process metadata ${entry}`);
      }
      pids.push(pid);
    }
    onValidated(pids);
    for (const pid of pids) {
      signalProcess(pid, 'SIGTERM');
    }
    await waitForProcesses(pids, 40);
    for (const pid of pids.filter(processAlive)) {
      signalProcess(pid, 'SIGKILL');
    }
    await waitForProcesses(pids, 20);
    const survivors = pids.filter(processAlive);
    if (survivors.length > 0) {
      throw new Error(
        `owned processes did not stop: ${survivors.join(', ')}`,
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
