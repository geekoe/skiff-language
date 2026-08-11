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
  classifyAuthoringRoot,
  readDevRegistry,
  runDevSyncOnce,
  runDevWatch,
  writeDevRegistry,
} from '../skiff-dev-sync.mjs';
import { writePackageRoot } from './package-service-fixtures.mjs';

const scriptRoot = dirname(fileURLToPath(new URL('../skiff.mjs', import.meta.url)));
const skiffCli = join(scriptRoot, 'skiff.mjs');
const targetConfig = {
  snapshotId: `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`,
};

test('watch retry reuses the retained build state when the post-publish step failed', async () => {
  const fixture = await serviceFixture('watch-retry-reuse', 'example.com/watch-reuse');
  const registryPath = join(fixture.temp, 'watch.json');
  const entry = await classifyAuthoringRoot(fixture.root);
  await writeDevRegistry(registryPath, { profile: 'dev', roots: [entry] });
  const buildStates = [];
  const errors = [];
  let clock = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      syncRunner: async ({ buildState }) => {
        buildStates.push(buildState);
        if (buildStates.length === 1) {
          const error = new Error('config snapshot production failed: timed out');
          error.reusableBuildState = { marker: 'retained-build' };
          throw error;
        }
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: (error) => errors.push(error.message),
      now: () => clock,
      wait: async () => {
        clock += 1500;
        if (clock > 4500) {
          throw new Error('watch retry sequence complete');
        }
      },
    }),
    /watch retry sequence complete/,
  );
  assert.equal(buildStates[0], undefined);
  assert.deepEqual(buildStates[1], { marker: 'retained-build' });
  assert.equal(errors.length, 1);
});

test('watch does not reuse the retained build state after a source change', async () => {
  const fixture = await serviceFixture('watch-no-reuse', 'example.com/watch-no-reuse');
  const registryPath = join(fixture.temp, 'watch.json');
  const entry = await classifyAuthoringRoot(fixture.root);
  await writeDevRegistry(registryPath, { profile: 'dev', roots: [entry] });
  const buildStates = [];
  let clock = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      syncRunner: async ({ buildState }) => {
        buildStates.push(buildState);
        if (buildStates.length === 1) {
          const error = new Error('config snapshot production failed: timed out');
          error.reusableBuildState = { marker: 'retained-build' };
          throw error;
        }
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: () => {},
      now: () => clock,
      wait: async () => {
        clock += 1500;
        if (clock === 1500) {
          await writeFile(
            join(fixture.root, 'main.skiff'),
            'function health() -> string { return "changed" }\n',
          );
        }
        if (clock > 4500) {
          throw new Error('watch no-reuse sequence complete');
        }
      },
    }),
    /watch no-reuse sequence complete/,
  );
  assert.equal(buildStates[0], undefined);
  assert.equal(buildStates[1], undefined);
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
    profile: 'dev',
    roots: [entry],
  });
  const oldHandle = await open(registryPath, 'r');
  await writeDevRegistry(registryPath, {
    profile: 'staging',
    roots: [entry],
  });
  const oldBytes = await oldHandle.readFile('utf8');
  await oldHandle.close();
  const current = await readDevRegistry(registryPath);
  assert.match(oldBytes, /"profile": "dev"/);
  assert.equal(current.schemaVersion, 'skiff-package-service-dev-registry-v2');
  assert.equal(current.profile, 'staging');
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
    profile: 'dev',
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
  const packageRoot = join(fixture.temp, 'package');
  await writePackageRoot(packageRoot, { packageId: 'example.com/cli-package-dependency' });
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
  const addPackage = await captureAttachedCommand(process.execPath, [
    skiffCli,
    'service',
    'dev',
    'registry',
    'add',
    packageRoot,
    '--config',
    registryPath,
  ], { cwd: fixture.temp });
  assert.equal(addPackage.code, 0, addPackage.stderr);

  const listed = await captureAttachedCommand(process.execPath, [
    skiffCli,
    'service',
    'dev',
    'registry',
    'list',
    '--config',
    registryPath,
  ], { cwd: fixture.temp });
  assert.equal(listed.code, 0, listed.stderr);
  assert.match(listed.stdout, new RegExp(`- package ${escapeRegExp(packageRoot)}`));
  assert.match(
    listed.stdout,
    new RegExp(`- service example\\.com/cli at ${escapeRegExp(fixture.root)}`),
  );
  assert.match(
    listed.stdout,
    /only listed roots are watched; add every locally developed package dependency explicitly/,
  );

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

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

test('watch reloads registry profile and sorted roots after atomic replacement', async () => {
  const first = await serviceFixture('watch-registry', 'example.com/watch-a');
  const secondRoot = join(first.temp, 'service-b');
  await writeServiceRoot(secondRoot, 'example.com/watch-b');
  const registryPath = join(first.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    profile: 'dev',
    roots: [await classifyAuthoringRoot(first.root)],
  });
  const calls = [];
  let polls = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      syncRunner: async ({ roots, profile }) => {
        calls.push({
          profile,
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
            profile: 'staging',
            roots: [
              await classifyAuthoringRoot(secondRoot),
              await classifyAuthoringRoot(first.root),
            ],
          });
        } else if (polls === 2) {
          await writeDevRegistry(registryPath, {
            profile: 'staging',
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
    { profile: 'dev', services: ['example.com/watch-a'] },
    {
      profile: 'staging',
      services: ['example.com/watch-a', 'example.com/watch-b'],
    },
    { profile: 'staging', services: [] },
  ]);
});

test('watch preserves last-known-good registry across invalid live roots, bad JSON, and ENOENT', async () => {
  const fixture = await serviceFixture('watch-lkg', 'example.com/watch-lkg');
  const registryPath = join(fixture.temp, 'watch.json');
  const entry = await classifyAuthoringRoot(fixture.root);
  await writeDevRegistry(registryPath, { profile: 'dev', roots: [entry] });
  const calls = [];
  const errors = [];
  let polls = 0;
  await assert.rejects(
    runDevWatch(watchOptions(registryPath), {
      syncRunner: async ({ profile }) => {
        calls.push(profile);
        return {};
      },
      buildStateFromResult: () => ({}),
      printResult: () => {},
      reportError: (error) => errors.push(error.message),
      wait: async () => {
        polls += 1;
        if (polls === 1) {
          await writeDevRegistry(registryPath, {
            profile: 'invalid-live-root',
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
            profile: 'recovered',
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

test('watch does not synthesize a sync before its first valid registry', async () => {
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
            profile: 'dev',
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
    profile: 'dev',
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

test('empty registry still publishes the explicit empty snapshot', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-empty-dev-snapshot-'));
  const result = await runDevSyncOnce({
    roots: [],
    profile: 'dev',
    artifactRoot: join(temp, 'artifacts'),
    buildOnly: true,
    compilerRunner: async () => {
      throw new Error('empty sync must not invoke compiler');
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
  assert.deepEqual(result.serviceDeploymentReceipts, []);
});

test('removing the final service converges watch to an empty deployment closure and snapshot without coordination', async () => {
  const fixture = await serviceFixture('remove-last', 'example.com/remove-last');
  const registryPath = join(fixture.temp, 'watch.json');
  await writeDevRegistry(registryPath, {
    profile: 'dev',
    roots: [await classifyAuthoringRoot(fixture.root)],
  });
  const nonemptyConfig = {
    snapshotId: `skiff-runtime-config-snapshot-v1:${'e'.repeat(32)}`,
  };
  const emptyConfig = {
    snapshotId: `skiff-runtime-config-snapshot-v1:${'f'.repeat(32)}`,
  };
  let watchWaits = 0;
  let networkCalls = 0;

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
        throw new Error('dev watch must not invoke assembly authoring');
      },
      configSnapshotRunner: async ({ sources }) => ({
        runtimeConfigSnapshotReceipt: {
          snapshot: sources.length === 0 ? emptyConfig : nonemptyConfig,
          recordPath: sources.length === 0
            ? 'runtime-config/snapshots/empty.json'
            : 'runtime-config/snapshots/nonempty.json',
        },
      }),
      fetchImpl: async () => {
        networkCalls += 1;
        return jsonResponse({});
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

  assert.equal(networkCalls, 0);
  assert.deepEqual((await readDevRegistry(registryPath)).roots, []);
});

function watchOptions(registryPath) {
  return {
    roots: [],
    config: registryPath,
    artifactRoot: join(dirname(registryPath), 'artifacts'),
    profile: undefined,
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

function jsonResponse(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}
