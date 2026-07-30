import assert from 'node:assert/strict';
import {
  mkdtemp,
  open,
  readFile,
  readdir,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { captureAttachedCommand } from '../lib/command-execution.mjs';
import {
  matchRegistryRemovalTarget,
  runDevRegistryCommand,
} from '../lib/package-service-dev-registry.mjs';
import {
  activateDevAssembly,
  classifyAuthoringRoot,
  readDevRegistry,
  runDevSyncOnce,
  runDevWatch,
  writeDevRegistry,
} from '../skiff-dev-sync.mjs';
import { writePackageRoot } from './package-service-fixtures.mjs';

const scriptRoot = dirname(fileURLToPath(new URL('../skiff.mjs', import.meta.url)));
const skiffCli = join(scriptRoot, 'skiff.mjs');
const targetAssembly = {
  assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`,
};
const targetConfig = {
  snapshotId: `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`,
};

test('managed activation restart accepts an already committed generation one tuple', async () => {
  let posts = 0;
  const result = await activateDevAssembly({
    activationUrl: 'http://router.test:4101/custom/activation',
    environment: 'dev',
    assembly: targetAssembly,
    configSnapshot: targetConfig,
    fetchImpl: async (url, init) => {
      assert.equal(url, 'http://router.test:4101/__router/health');
      assert.equal(init.method, 'GET');
      posts += init.method === 'POST' ? 1 : 0;
      return healthResponse({
        generation: 1,
        assembly: targetAssembly,
        configSnapshot: targetConfig,
      });
    },
  });
  assert.equal(posts, 0);
  assert.equal(result.response.idempotent, true);
  assert.equal(result.response.committed.generation, 1);
});

test('managed activation treats a lost commit response as idempotent success', async () => {
  let active = activeTuple(1, '1', '2');
  let request;
  const result = await activateDevAssembly({
    activationUrl: 'http://router.test:4101/__skiff/activate-assembly',
    environment: 'dev',
    assembly: targetAssembly,
    configSnapshot: targetConfig,
    fetchImpl: async (_url, init) => {
      if (init.method === 'GET') {
        return healthResponse(active);
      }
      request = JSON.parse(init.body);
      active = {
        environment: 'dev',
        generation: 2,
        assembly: targetAssembly,
        configSnapshot: targetConfig,
      };
      throw new Error('connection reset after commit');
    },
  });
  assert.equal(request.expectedGeneration, 1);
  assert.equal(result.response.idempotent, true);
  assert.equal(result.response.committed.generation, 2);
});

test('managed activation rereads generation and retries a competing 409 with bounded backoff', async () => {
  let active = activeTuple(1, '1', '2');
  const requests = [];
  const waits = [];
  const result = await activateDevAssembly({
    activationUrl: 'http://router.test:4101/__skiff/activate-assembly',
    environment: 'dev',
    assembly: targetAssembly,
    configSnapshot: targetConfig,
    wait: async (milliseconds) => waits.push(milliseconds),
    fetchImpl: async (_url, init) => {
      if (init.method === 'GET') {
        return healthResponse(active);
      }
      const request = JSON.parse(init.body);
      requests.push(request);
      if (requests.length === 1) {
        active = activeTuple(2, '3', '4');
        return jsonResponse({
          error: { code: 'Conflict', message: 'generation changed' },
        }, 409);
      }
      active = {
        environment: 'dev',
        generation: 3,
        assembly: targetAssembly,
        configSnapshot: targetConfig,
      };
      return jsonResponse({ ok: true, committed: active });
    },
  });
  assert.deepEqual(requests.map(({ expectedGeneration }) => expectedGeneration), [1, 2]);
  assert.equal(requests[0].activationId, requests[1].activationId);
  assert.deepEqual(waits, [50]);
  assert.equal(result.response.committed.generation, 3);
});

test('registry writer atomically replaces the file and persists service identity in v2', async () => {
  const fixture = await serviceFixture('atomic', 'example.com/atomic');
  const registryPath = join(fixture.temp, 'watch.json');
  const entry = await classifyAuthoringRoot(fixture.root);
  assert.deepEqual(entry, {
    kind: 'service',
    root: fixture.root,
    serviceId: 'example.com/atomic',
  });
  await writeDevRegistry(registryPath, {
    environment: 'dev',
    roots: [entry],
  });
  const oldHandle = await open(registryPath, 'r');
  await writeDevRegistry(registryPath, {
    environment: 'staging',
    roots: [entry],
  });
  const oldBytes = await oldHandle.readFile('utf8');
  await oldHandle.close();
  const current = await readDevRegistry(registryPath);
  assert.match(oldBytes, /"environment": "dev"/);
  assert.equal(current.schemaVersion, 'skiff-package-service-dev-registry-v2');
  assert.equal(current.environment, 'staging');
  assert.deepEqual(current.roots, [{
    kind: 'service',
    root: fixture.root,
    serviceId: 'example.com/atomic',
  }]);
  assert.deepEqual(
    (await readdir(fixture.temp)).filter((name) => name.includes('.tmp-')),
    [],
  );
});

test('registry remove accepts service ID and a deleted canonical root without reading it', async () => {
  const first = await serviceFixture('remove-service', 'example.com/remove-me');
  const secondRoot = join(first.temp, 'stale-package');
  await writePackageRoot(secondRoot, { packageId: 'example.com/stale-package' });
  const registryPath = join(first.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    environment: 'dev',
    roots: [
      await classifyAuthoringRoot(first.root),
      await classifyAuthoringRoot(secondRoot),
    ],
  });
  await rm(first.root, { recursive: true, force: true });
  await runDevRegistryCommand([
    'remove',
    'example.com/remove-me',
    '--config',
    registryPath,
  ], {
    defaultConfig: registryPath,
    stdout: () => {},
  });
  await rm(secondRoot, { recursive: true, force: true });
  await runDevRegistryCommand([
    'remove',
    secondRoot,
    '--config',
    registryPath,
  ], {
    defaultConfig: registryPath,
    stdout: () => {},
  });
  assert.deepEqual((await readDevRegistry(registryPath)).roots, []);
});

test('registry remove fails closed when one token matches a service ID and another root', () => {
  const service = {
    kind: 'service',
    root: '/tmp/service-owner',
    serviceId: 'example.com/collision',
  };
  const packageEntry = {
    kind: 'package',
    root: '/tmp/package-owner',
  };
  assert.throws(
    () => matchRegistryRemovalTarget(
      [service, packageEntry],
      'example.com/collision',
      { resolveTarget: () => packageEntry.root },
    ),
    /ambiguous/,
  );
});

test('canonical registry CLI is service dev registry and old dev registry is rejected', async () => {
  const fixture = await serviceFixture('cli', 'example.com/cli');
  const registryPath = join(fixture.temp, 'watch.json');
  const canonical = await captureAttachedCommand(process.execPath, [
    skiffCli,
    'service',
    'dev',
    'registry',
    'add',
    fixture.root,
    '--config',
    registryPath,
  ], { cwd: fixture.temp });
  assert.equal(canonical.code, 0, canonical.stderr);
  assert.equal((await readDevRegistry(registryPath)).roots[0].serviceId, 'example.com/cli');

  const retired = await captureAttachedCommand(process.execPath, [
    skiffCli,
    'dev',
    'registry',
    'list',
    '--config',
    registryPath,
  ], { cwd: fixture.temp });
  assert.notEqual(retired.code, 0);
  assert.match(retired.stderr, /unknown dev command registry/);
});

test('watch reloads registry environment and sorted roots after atomic replacement', async () => {
  const first = await serviceFixture('watch-registry', 'example.com/watch-a');
  const secondRoot = join(first.temp, 'service-b');
  await writeServiceRoot(secondRoot, 'example.com/watch-b');
  const registryPath = join(first.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    environment: 'dev',
    roots: [await classifyAuthoringRoot(first.root)],
  });
  const calls = [];
  let polls = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      syncRunner: async ({ roots, environment }) => {
        calls.push({
          environment,
          services: roots.map(({ serviceId }) => serviceId),
        });
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: (error) => {
        throw error;
      },
      wait: async () => {
        polls += 1;
        if (polls === 1) {
          await writeDevRegistry(registryPath, {
            environment: 'staging',
            roots: [
              await classifyAuthoringRoot(secondRoot),
              await classifyAuthoringRoot(first.root),
            ],
          });
        } else if (polls === 2) {
          await writeDevRegistry(registryPath, {
            environment: 'staging',
            roots: [],
          });
        } else {
          throw new Error('dynamic registry sequence complete');
        }
      },
    }),
    /dynamic registry sequence complete/,
  );
  assert.deepEqual(calls, [
    { environment: 'dev', services: ['example.com/watch-a'] },
    {
      environment: 'staging',
      services: ['example.com/watch-a', 'example.com/watch-b'],
    },
    { environment: 'staging', services: [] },
  ]);
});

test('watch preserves last-known-good registry across invalid live roots, bad JSON, and ENOENT', async () => {
  const fixture = await serviceFixture('watch-lkg', 'example.com/watch-lkg');
  const registryPath = join(fixture.temp, 'watch.json');
  const entry = await classifyAuthoringRoot(fixture.root);
  await writeDevRegistry(registryPath, { environment: 'dev', roots: [entry] });
  const calls = [];
  const errors = [];
  let polls = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      syncRunner: async ({ environment }) => {
        calls.push(environment);
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: (error) => errors.push(error.message),
      wait: async () => {
        polls += 1;
        if (polls === 1) {
          await writeDevRegistry(registryPath, {
            environment: 'invalid-live-root',
            roots: [{
              kind: 'service',
              root: join(fixture.temp, 'missing-service'),
              serviceId: 'example.com/missing',
            }],
          });
        } else if (polls === 2) {
          await writeFile(registryPath, '{invalid json');
        } else if (polls === 3) {
          await rm(registryPath);
        } else if (polls === 4) {
          await writeDevRegistry(registryPath, {
            environment: 'recovered',
            roots: [entry],
          });
        } else {
          throw new Error('last-known-good sequence complete');
        }
      },
    }),
    /last-known-good sequence complete/,
  );
  assert.deepEqual(calls, ['dev', 'recovered']);
  assert.equal(
    errors.some((message) => message.includes('continuing with the last known-good')),
    true,
  );
  assert.equal(errors.some((message) => message.includes('last known-good')), true);
  assert.equal(errors.some((message) => message.includes('ENOENT')), true);
});

test('watch does not synthesize an empty activation before its first valid registry', async () => {
  const fixture = await serviceFixture('watch-first-registry', 'example.com/watch-first');
  const registryPath = join(fixture.temp, 'missing-watch.json');
  const calls = [];
  const errors = [];
  let clock = 0;
  let polls = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      now: () => clock,
      syncRunner: async ({ roots }) => {
        calls.push(roots.map(({ serviceId }) => serviceId));
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: (error) => errors.push(error.message),
      wait: async (milliseconds) => {
        clock += milliseconds;
        polls += 1;
        if (clock === 1000) {
          await writeDevRegistry(registryPath, {
            environment: 'dev',
            roots: [await classifyAuthoringRoot(fixture.root)],
          });
        } else if (polls === 3) {
          throw new Error('first-registry sequence complete');
        }
      },
    }),
    /first-registry sequence complete/,
  );
  assert.deepEqual(calls, [['example.com/watch-first']]);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /waiting for the first valid dev registry/);
});

test('watch retries failed content with exponential delay and new content replaces pending work', async () => {
  const fixture = await serviceFixture('watch-retry', 'example.com/watch-retry');
  const registryPath = join(fixture.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    environment: 'dev',
    roots: [await classifyAuthoringRoot(fixture.root)],
  });
  const attempts = [];
  let active = 0;
  let maxActive = 0;
  let clock = 0;
  let polls = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      now: () => clock,
      syncRunner: async () => {
        active += 1;
        maxActive = Math.max(maxActive, active);
        attempts.push(clock);
        active -= 1;
        if (attempts.length < 3) {
          throw new Error(`temporary failure ${attempts.length}`);
        }
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: () => {},
      wait: async (milliseconds) => {
        clock += milliseconds;
        polls += 1;
        if (polls === 3) {
          await writeFile(
            join(fixture.root, 'config.dev.yml'),
            '"example.com/watch-retry":\n  changed: true\n',
          );
        } else if (polls === 5) {
          throw new Error('retry sequence complete');
        }
      },
    }),
    /retry sequence complete/,
  );
  assert.deepEqual(attempts, [0, 1000, 1500]);
  assert.equal(maxActive, 1);
});

test('empty registry still builds the explicit empty assembly candidate', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-empty-dev-assembly-'));
  let assemblyInput;
  const result = await runDevSyncOnce({
    roots: [],
    environment: 'dev',
    artifactRoot: join(temp, 'artifacts'),
    buildOnly: true,
    compilerRunner: async (input) => {
      assemblyInput = input;
      return {
        runtimeAssemblyReceipt: {
          environment: 'dev',
          assembly: targetAssembly,
          recordPath: 'records/empty-assembly.json',
        },
      };
    },
    configSnapshotRunner: async ({ sources }) => {
      assert.deepEqual(sources, []);
      return {
        runtimeConfigSnapshotReceipt: {
          snapshot: targetConfig,
          recordPath: 'runtime-config/snapshots/empty.json',
        },
      };
    },
  });
  assert.deepEqual(assemblyInput.rootDeployments, []);
  assert.deepEqual(result.serviceDeploymentReceipts, []);
});

test('removing the final service converges watch to one exact empty assembly and snapshot pair', async () => {
  const fixture = await serviceFixture('remove-last', 'example.com/remove-last');
  const registryPath = join(fixture.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    environment: 'dev',
    roots: [await classifyAuthoringRoot(fixture.root)],
  });
  const nonemptyAssembly = {
    assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'c'.repeat(64)}`,
  };
  const emptyAssembly = {
    assemblyIdentity: `skiff-runtime-assembly-v3:sha256:${'d'.repeat(64)}`,
  };
  const nonemptyConfig = {
    snapshotId: `skiff-runtime-config-snapshot-v1:${'e'.repeat(32)}`,
  };
  const emptyConfig = {
    snapshotId: `skiff-runtime-config-snapshot-v1:${'f'.repeat(32)}`,
  };
  let active = activeTuple(0, '1', '2');
  const activationRequests = [];
  let watchWaits = 0;

  await assert.rejects(
    runDevWatch({
      ...watchOptions(registryPath),
      buildOnly: false,
    }, {
      compilerRunner: async (input) => {
        if (input.kind === 'package') {
          return {
            packageArtifactReceipt: {
              artifact: {
                packageId: 'example.com/remove-last-package',
                packageVersion: '1.0.0',
              },
            },
            serviceDeploymentReceipt: {
              deployment: {
                serviceId: 'example.com/remove-last',
                contractVersion: '1.0.0',
                deploymentRevision: 'revision-1',
                deploymentArtifactIdentity:
                  `skiff-deployment-artifact-v2:sha256:${'9'.repeat(64)}`,
              },
            },
          };
        }
        const assembly = input.rootDeployments.length === 0
          ? emptyAssembly
          : nonemptyAssembly;
        return {
          runtimeAssemblyReceipt: {
            assembly,
            recordPath: input.rootDeployments.length === 0
              ? 'records/empty-assembly.json'
              : 'records/nonempty-assembly.json',
          },
        };
      },
      configSnapshotRunner: async ({ sources }) => ({
        runtimeConfigSnapshotReceipt: {
          snapshot: sources.length === 0 ? emptyConfig : nonemptyConfig,
          recordPath: sources.length === 0
            ? 'runtime-config/snapshots/empty.json'
            : 'runtime-config/snapshots/nonempty.json',
        },
      }),
      fetchImpl: async (_url, init) => {
        if (init.method === 'GET') {
          return healthResponse(active);
        }
        const request = JSON.parse(init.body);
        activationRequests.push(request);
        active = {
          environment: 'dev',
          generation: active.generation + 1,
          assembly: request.assembly,
          configSnapshot: request.configSnapshot,
        };
        return jsonResponse({ ok: true, committed: active });
      },
      printResult: () => {},
      wait: async () => {
        watchWaits += 1;
        if (watchWaits === 1) {
          await runDevRegistryCommand([
            'remove',
            'example.com/remove-last',
            '--config',
            registryPath,
          ], {
            defaultConfig: registryPath,
            stdout: () => {},
          });
          return;
        }
        throw new Error('remove-last sequence complete');
      },
    }),
    /remove-last sequence complete/,
  );

  assert.deepEqual(
    activationRequests.map((request) => ({
      expectedGeneration: request.expectedGeneration,
      assembly: request.assembly,
      configSnapshot: request.configSnapshot,
    })),
    [
      {
        expectedGeneration: 0,
        assembly: nonemptyAssembly,
        configSnapshot: nonemptyConfig,
      },
      {
        expectedGeneration: 1,
        assembly: emptyAssembly,
        configSnapshot: emptyConfig,
      },
    ],
  );
  assert.deepEqual((await readDevRegistry(registryPath)).roots, []);
});

function watchOptions(registryPath) {
  return {
    roots: [],
    config: registryPath,
    artifactRoot: join(dirname(registryPath), 'artifacts'),
    activationUrl: 'http://router.test:4101/__skiff/activate-assembly',
    activationId: undefined,
    environment: undefined,
    pollIntervalMs: 500,
    watch: true,
    buildOnly: true,
    json: true,
  };
}

async function serviceFixture(name, serviceId) {
  const temp = await mkdtemp(join(tmpdir(), `skiff-${name}-`));
  const root = join(temp, 'service');
  await writeServiceRoot(root, serviceId);
  return { temp, root };
}

async function writeServiceRoot(root, serviceId) {
  await writePackageRoot(root, { packageId: `${serviceId}-package` });
  await writeFile(join(root, 'service.yml'), `id: ${serviceId}\n`);
}

function activeTuple(generation, assemblyDigit, snapshotDigit) {
  return {
    environment: 'dev',
    generation,
    assembly: {
      assemblyIdentity:
        `skiff-runtime-assembly-v3:sha256:${assemblyDigit.repeat(64)}`,
    },
    configSnapshot: {
      snapshotId:
        `skiff-runtime-config-snapshot-v1:${snapshotDigit.repeat(32)}`,
    },
  };
}

function healthResponse(active) {
  return jsonResponse({
    ok: true,
    activeAssembly: {
      environment: active.environment ?? 'dev',
      generation: active.generation,
      assemblyIdentity: active.assembly.assemblyIdentity,
      configSnapshotId: active.configSnapshot.snapshotId,
    },
  });
}

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}
