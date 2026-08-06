import { randomInt } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { assertPortsClosed, leaseLocalPorts } from './local-port-lease.mjs';
import { createMongoshCommand } from './mongosh-json-command.mjs';

const PORT_MIN = 45000;
const PORT_MAX = 45999;
const FORBIDDEN_PORTS = new Set([
  27017,
  ...range(4000, 4007),
  ...range(44000, 44999),
]);
const REPLICA_SET_NAME = 'rs0';

/**
 * Temporary single-node Mongo replica set for managed live checks.
 *
 * Follows the repository's existing live harness conventions
 * (`encrypted-storage-live-*`): leased port in 45000-45999, mktemp dbPath,
 * mongosh-driven `rs.initiate`, and guaranteed cleanup (SIGTERM -> SIGKILL,
 * temp root removal, port lease release, port-closed assertion). Never touches
 * the stable Mongo on 27017 or the stable Skiff instance. The Router no longer
 * connects to Mongo; the replica set exists so the Runtime's serviceDb still
 * has a real endpoint.
 */
export class MongodLiveHarness {
  static async create({ repoRoot = process.cwd() } = {}) {
    const { port, release } = await leaseProbePort();
    const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-mongod-live-'));
    const dbPath = join(tempRoot, 'db');
    await mkdir(dbPath, { recursive: true });
    return new MongodLiveHarness({
      repoRoot,
      port,
      tempRoot,
      dbPath,
      releaseLease: release,
    });
  }

  constructor({ repoRoot, port, tempRoot, dbPath, releaseLease }) {
    this.repoRoot = repoRoot;
    this.port = port;
    this.tempRoot = tempRoot;
    this.dbPath = dbPath;
    this.releaseLease = releaseLease;
    this.mongoUrl =
      `mongodb://127.0.0.1:${port}/?directConnection=true&replicaSet=${REPLICA_SET_NAME}&retryWrites=false`;
    this.mongosh = createMongoshCommand();
    this.mongod = undefined;
    this.mongodExited = undefined;
    this.cleaned = false;
  }

  async start() {
    const logPath = join(this.tempRoot, 'mongod.log');
    const child = spawnMongodProcess({
      port: this.port,
      dbPath: this.dbPath,
      logPath,
    });
    this.mongod = child;
    this.mongodExited = new Promise((resolve) => {
      child.once('exit', (code, signal) => resolve({ code, signal }));
    });
    child.once('error', (error) => {
      throw new Error(`failed to start temporary mongod: ${error.message}`);
    });
    await this.waitUntilReady();
    await this.initializeReplicaSet();
    console.log(`live-check mongod ready on 127.0.0.1:${this.port}`);
  }

  async cleanup() {
    if (this.cleaned) {
      return;
    }
    this.cleaned = true;
    const errors = [];
    try {
      await this.stopMongod();
    } catch (error) {
      errors.push(error);
    }
    try {
      await rm(this.tempRoot, { recursive: true, force: true });
    } catch (error) {
      errors.push(error);
    }
    try {
      await assertPortsClosed([this.port]);
    } catch (error) {
      errors.push(error);
    }
    try {
      await this.releaseLease();
    } catch (error) {
      errors.push(error);
    }
    if (errors.length > 0) {
      throw new AggregateError(errors, 'live-check mongod cleanup failed');
    }
  }

  async waitUntilReady() {
    for (let attempt = 0; attempt < 100; attempt += 1) {
      try {
        const hello = await this.mongosh.json(
          {
            url: `mongodb://127.0.0.1:${this.port}/admin?directConnection=true`,
            expression: 'db.hello().ok === 1',
            cwd: this.repoRoot,
          },
        );
        if (hello === true) {
          return;
        }
      } catch {
        // mongod is still starting; retry.
      }
      await delay(100);
    }
    throw new Error(`temporary mongod did not become ready on port ${this.port}`);
  }

  async initializeReplicaSet() {
    const url = `mongodb://127.0.0.1:${this.port}/admin?directConnection=true`;
    const initiate = `try { rs.status(); } catch (error) { rs.initiate({_id:${JSON.stringify(
      REPLICA_SET_NAME,
    )},members:[{_id:0,host:'127.0.0.1:${this.port}'}]}); }`;
    await this.mongosh.run([url, '--quiet', '--eval', initiate], {
      cwd: this.repoRoot,
    });
    for (let attempt = 0; attempt < 120; attempt += 1) {
      const writable = await this.mongosh.json(
        {
          url: `mongodb://127.0.0.1:${this.port}/admin?directConnection=true`,
          expression: 'db.hello().isWritablePrimary === true',
          cwd: this.repoRoot,
        },
      );
      if (writable === true) {
        return;
      }
      await delay(100);
    }
    throw new Error(`replica set ${REPLICA_SET_NAME} did not become writable`);
  }

  async stopMongod() {
    const child = this.mongod;
    if (child === undefined || child.exitCode !== null || child.signalCode !== null) {
      return;
    }
    child.kill('SIGTERM');
    const outcome = await Promise.race([
      this.mongodExited,
      delay(5000).then(() => undefined),
    ]);
    if (outcome === undefined) {
      child.kill('SIGKILL');
      await Promise.race([
        this.mongodExited,
        delay(5000).then(() => undefined),
      ]);
    }
  }
}

function spawnMongodProcess({ port, dbPath, logPath }) {
  // child-process-owner: mongod-live-spawn
  return spawn(
    'mongod',
    [
      '--port',
      String(port),
      '--dbpath',
      dbPath,
      '--bind_ip',
      '127.0.0.1',
      '--replSet',
      REPLICA_SET_NAME,
      '--quiet',
      '--logpath',
      logPath,
    ],
    { stdio: ['ignore', 'ignore', 'ignore'] },
  );
}

async function leaseProbePort() {
  for (let attempt = 0; attempt < 500; attempt += 1) {
    const port = PORT_MIN + randomInt(PORT_MAX - PORT_MIN + 1);
    if (FORBIDDEN_PORTS.has(port)) {
      continue;
    }
    try {
      const lease = await leaseLocalPorts([port]);
      return {
        port,
        release: () => lease.release(),
      };
    } catch {
      // Another owner holds this port; try another candidate.
    }
  }
  throw new Error(`no isolated port available in ${PORT_MIN}-${PORT_MAX}`);
}

function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}
