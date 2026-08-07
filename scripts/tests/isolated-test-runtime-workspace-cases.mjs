import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';

import { runInIsolatedTestRuntime } from '../lib/isolated-test-runtime.mjs';
import {
  assertIsolatedTestWorkspaceOwned,
  claimIsolatedTestWorkspace,
  removeOwnedIsolatedTestWorkspace,
} from '../lib/isolated-test-runtime-workspace.mjs';

test('isolated runtime carries a nonce and exact workspace identities through normal teardown', async () => {
  const fixture = await createOwnershipLifecycleDouble();
  let receipt;
  try {
    await fixture.seedForeignWorkspaces(12);
    assert.equal(await runOwnedFixture(fixture, async (_environment, _signal, stack) => {
      receipt = stack.ownershipReceipt;
      await assertIsolatedTestWorkspaceOwned(receipt);
      return 'owned-result';
    }), 'owned-result');

    assert.match(receipt.nonce, /^[0-9a-f]{32}$/);
    assert.equal(typeof receipt.root.identity.dev, 'string');
    assert.equal(typeof receipt.root.identity.ino, 'string');
    assert.equal(typeof receipt.marker.identity.ino, 'string');
    assert.equal(receipt.config, undefined);
    assert.equal(fixture.actions.filter((action) => action === 'stop:stack').length, 1);
    assert.equal(fixture.actions.filter((action) => action === 'workspace:remove').length, 1);
    await assert.rejects(lstat(receipt.root.path), { code: 'ENOENT' });
    await fixture.assertForeignWorkspacesPreserved();
  } finally {
    await fixture.dispose();
  }
});

const replacementScenarios = [
  {
    name: 'root directory replacement before temp removal',
    mutationAt: 'after-stop-stack',
    mutation: 'replace-root',
  },
  {
    name: 'root symlink replacement before temp removal',
    mutationAt: 'after-stop-stack',
    mutation: 'symlink-root',
  },
  {
    name: 'missing marker before temp removal',
    mutationAt: 'after-stop-stack',
    mutation: 'remove-marker',
  },
  {
    name: 'corrupt marker before temp removal',
    mutationAt: 'after-stop-stack',
    mutation: 'corrupt-marker',
  },
  {
    name: 'marker symlink replacement before temp removal',
    mutationAt: 'after-stop-stack',
    mutation: 'symlink-marker',
  },
  {
    name: 'marker replacement between stop and port verification',
    mutationAt: 'after-stop-stack',
    mutation: 'replace-marker',
  },
  {
    name: 'root symlink replacement between port verification and lease release',
    mutationAt: 'after-ports',
    mutation: 'symlink-root',
  },
  {
    name: 'root directory replacement between lease release and recursive removal',
    mutationAt: 'after-lease',
    mutation: 'replace-root',
  },
];

test('foreign workspace replacements fail closed at every teardown jump', async (context) => {
  assert.ok(replacementScenarios.length > 0, 'command-double teardown matrix must match cases');
  for (const scenario of replacementScenarios) {
    await context.test(scenario.name, async () => {
      const fixture = await createOwnershipLifecycleDouble(scenario);
      try {
        await assert.rejects(
          runOwnedFixture(fixture, async () => 'test-passed'),
          (error) => {
            assert.match(error.message, /isolated workspace ownership mismatch/);
            assert.match(error.message, new RegExp(fixture.receipt.nonce));
            return true;
          },
        );
        assert.equal(fixture.mutationApplied, true);
        await fixture.assertForeignPreserved();
        assert.equal(fixture.actions.includes('stop:stack'), true);
        assert.equal(fixture.actions.includes('ports:closed'), true);
        assert.equal(fixture.actions.includes('lease:release'), true);
        assert.equal(fixture.actions.includes('workspace:remove'), false);
      } finally {
        await fixture.dispose();
      }
    });
  }
});

test('primary failure stays first while every owned cleanup surface settles', async () => {
  const primary = new Error('primary test failure');
  const fixture = await createOwnershipLifecycleDouble({
    mutationAt: 'after-stop-stack',
    mutation: 'replace-root',
    stopStackError: new Error('stack stop failure'),
    portsError: new Error('ports remained open'),
    leaseError: new Error('lease release failure'),
  });
  try {
    await assert.rejects(
      runOwnedFixture(fixture, async () => {
        throw primary;
      }),
      (error) => {
        assert.ok(error.cause instanceof AggregateError);
        assert.strictEqual(error.cause.errors[0], primary);
        const cleanup = error.cause.errors[1];
        assert.ok(cleanup instanceof AggregateError);
        assert.deepEqual(
          cleanup.errors.map((entry) => entry.message.split(':')[0]),
          [
            'stop isolated stack',
            'verify ports closed',
            'release port lease',
          ],
        );
        return true;
      },
    );
    await fixture.assertForeignPreserved();
    assert.equal(fixture.actions.includes('stop:stack'), true);
    assert.equal(fixture.actions.includes('ports:closed'), true);
    assert.equal(fixture.actions.includes('lease:release'), true);
    assert.equal(fixture.actions.includes('workspace:remove'), false);
  } finally {
    await fixture.dispose();
  }
});

async function runOwnedFixture(fixture, runTest) {
  return runInIsolatedTestRuntime({
    skiffRoot: '/checkout/skiff',
    baseEnv: { PATH: '/bin' },
    signalTarget: new EventEmitter(),
    dependencies: fixture.dependencies,
    ensureRuntimeBinaries: async () => {},
    runTest,
  });
}

async function createOwnershipLifecycleDouble({
  mutationAt,
  mutation,
  stopStackError,
  portsError,
  leaseError,
} = {}) {
  const outerRoot = await mkdtemp(join(tmpdir(), 'skiff-isolated-owner-test-'));
  const actions = [];
  const foreignWorkspaces = [];
  const state = {
    foreignCheck: async () => assert.fail('replacement mutation did not install evidence'),
    mutationApplied: false,
    receipt: undefined,
  };

  const mutateOnce = async (point) => {
    if (state.mutationApplied || mutationAt !== point) {
      return;
    }
    state.mutationApplied = true;
    state.foreignCheck = await mutateOwnedPath({
      outerRoot,
      receipt: state.receipt,
      mutation,
    });
  };

  const dependencies = {
    leasePorts: async () => ({
      ports: [46000, 46001, 46002, 46003],
      release: async () => {
        actions.push('lease:release');
        await mutateOnce('after-lease');
        if (leaseError !== undefined) {
          throw leaseError;
        }
      },
    }),
    makeTempRoot: () => mkdtemp(join(outerRoot, 'skiff-test-runtime-')),
    claimWorkspace: async (path) => {
      state.receipt = await claimIsolatedTestWorkspace(path);
      return state.receipt;
    },
    createSourceArtifactRoot: (path) => mkdir(path, { recursive: true }),
    seedBootstrap: async () => ({ profile: 'skiff-test', bootstrap: {} }),
    spawnMongo: () => ({ pid: 4242 }),
    waitMongoPrimary: async () => {},
    spawnRouter: () => ({ pid: 4243 }),
    spawnRuntime: () => ({ pid: 4244 }),
    waitReady: async () => {},
    stopProcesses: async () => {
      actions.push('stop:stack');
      await mutateOnce('after-stop-stack');
      if (stopStackError !== undefined) {
        throw stopStackError;
      }
    },
    assertPortsClosed: async () => {
      actions.push('ports:closed');
      await mutateOnce('after-ports');
      if (portsError !== undefined) {
        throw portsError;
      }
    },
    removeOwnedWorkspace: async (receipt) => {
      await removeOwnedIsolatedTestWorkspace(receipt);
      actions.push('workspace:remove');
    },
  };

  return {
    actions,
    dependencies,
    dispose: () => rm(outerRoot, { force: true, recursive: true }),
    get mutationApplied() {
      return state.mutationApplied;
    },
    get receipt() {
      return state.receipt;
    },
    assertForeignPreserved: () => state.foreignCheck(),
    seedForeignWorkspaces: async (count) => {
      for (let index = 0; index < count; index += 1) {
        const evidencePath = join(
          outerRoot,
          `skiff-test-runtime-foreign-${index}`,
          'foreign.txt',
        );
        await mkdir(dirname(evidencePath), { recursive: true });
        await writeFile(evidencePath, `foreign-${index}\n`, 'utf8');
        foreignWorkspaces.push({ evidencePath, expected: `foreign-${index}\n` });
      }
    },
    assertForeignWorkspacesPreserved: async () => {
      assert.equal(foreignWorkspaces.length, 12);
      for (const { evidencePath, expected } of foreignWorkspaces) {
        assert.equal(await readFile(evidencePath, 'utf8'), expected);
      }
    },
  };
}

async function mutateOwnedPath({ outerRoot, receipt, mutation }) {
  const foreignContents = `foreign:${mutation}\n`;
  switch (mutation) {
    case 'replace-root': {
      await rm(receipt.root.path, { recursive: true });
      await mkdir(receipt.root.path);
      const evidencePath = join(receipt.root.path, 'foreign-root.txt');
      await writeFile(evidencePath, foreignContents, 'utf8');
      return async () => assert.equal(await readFile(evidencePath, 'utf8'), foreignContents);
    }
    case 'symlink-root': {
      const foreignRoot = join(outerRoot, `foreign-root-${receipt.nonce}`);
      await mkdir(foreignRoot);
      const evidencePath = join(foreignRoot, 'foreign-root.txt');
      await writeFile(evidencePath, foreignContents, 'utf8');
      await rm(receipt.root.path, { recursive: true });
      await symlink(foreignRoot, receipt.root.path, 'dir');
      return async () => {
        assert.equal((await lstat(receipt.root.path)).isSymbolicLink(), true);
        assert.equal(await readFile(evidencePath, 'utf8'), foreignContents);
      };
    }
    case 'remove-marker':
      await rm(receipt.marker.path);
      return async () => assert.rejects(lstat(receipt.marker.path), { code: 'ENOENT' });
    case 'corrupt-marker':
      await writeFile(receipt.marker.path, foreignContents, 'utf8');
      return async () => assert.equal(await readFile(receipt.marker.path, 'utf8'), foreignContents);
    case 'replace-marker':
      await rm(receipt.marker.path);
      await writeFile(receipt.marker.path, foreignContents, 'utf8');
      return async () => assert.equal(await readFile(receipt.marker.path, 'utf8'), foreignContents);
    case 'symlink-marker': {
      const foreignMarker = join(outerRoot, `foreign-marker-${receipt.nonce}.json`);
      await writeFile(foreignMarker, foreignContents, 'utf8');
      await rm(receipt.marker.path);
      await symlink(foreignMarker, receipt.marker.path);
      return async () => {
        assert.equal((await lstat(receipt.marker.path)).isSymbolicLink(), true);
        assert.equal(await readFile(foreignMarker, 'utf8'), foreignContents);
      };
    }
    default:
      throw new Error(`unknown ownership mutation: ${mutation}`);
  }
}
