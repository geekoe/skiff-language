import assert from 'node:assert/strict';
import {
  access,
  link,
  mkdir,
  mkdtemp,
  open,
  readFile,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  addOwnedWorktree,
  cleanupProbeOwnership,
  createProbeOwnership,
} from '../lib/platform-source-probe-ownership.mjs';
import {
  createProbeDependencies,
  installLedgerNoClobber,
  ledgerTemporaryPath,
} from '../lib/platform-source-probe-support.mjs';

const candidate = '1'.repeat(40);
const nonce = 'a'.repeat(32);

test('partial add cleans only a path with the matching claim and registry identity', async () => {
  const fixture = await ownershipFixture({ addMode: 'partial-owned' });
  try {
    await assert.rejects(
      addOwnedWorktree(
        fixture.owner,
        'A',
        fixture.input.aWorktree,
        fixture.checked,
      ),
      /partial add failure/,
    );
    assert.equal(fixture.owner.worktrees.length, 1);
    const cleanup = await cleanupProbeOwnership(fixture.owner, fixture.checked);
    assert.deepEqual(cleanup.errors, []);
    assert.equal(cleanup.worktrees[0].claimVerifiedBeforeRemoval, true);
    assert.equal(cleanup.worktrees[0].pathAbsent, true);
    assert.equal(cleanup.worktrees[0].registryAbsent, true);
    assert.equal(cleanup.taskRoot.absent, true);
    assert.equal(fixture.removeCalls.length, 1);
    assert.equal(fixture.removeCalls[0].includes('--force'), false);
  } finally {
    await fixture.dispose();
  }
});

test('registry-only partial add is preserved because path inode proof is absent', async () => {
  const fixture = await ownershipFixture({ addMode: 'registry-only' });
  try {
    await assert.rejects(
      addOwnedWorktree(
        fixture.owner,
        'A',
        fixture.input.aWorktree,
        fixture.checked,
      ),
      /registry-only failure/,
    );
    const cleanup = await cleanupProbeOwnership(fixture.owner, fixture.checked);
    assert.equal(fixture.owner.worktrees.length, 0);
    assert.equal(fixture.removeCalls.length, 0);
    assert.equal(cleanup.foreign.registries.length, 1);
    assert.equal(cleanup.foreign.registries[0].preserved, true);
  } finally {
    await fixture.dispose();
  }
});

test('foreign A and B paths created after the claim are preserved without registry proof', async () => {
  for (const label of ['A', 'B']) {
    const fixture = await ownershipFixture({ addMode: 'foreign-path' });
    const path = label === 'A' ? fixture.input.aWorktree : fixture.input.bWorktree;
    try {
      await assert.rejects(
        addOwnedWorktree(fixture.owner, label, path, fixture.checked),
        /foreign path failure/,
      );
      const before = await fixture.deps.pathIdentity(path);
      const cleanup = await cleanupProbeOwnership(fixture.owner, fixture.checked);
      assert.equal(fixture.removeCalls.length, 0);
      assert.deepEqual(await fixture.deps.pathIdentity(path), before);
      assert.equal(await readFile(join(path, 'foreign.txt'), 'utf8'), `${label}-foreign\n`);
      assert.equal(cleanup.foreign.preserved, true);
    } finally {
      await fixture.dispose();
    }
  }
});

test('post-add inode replacement is never passed to git worktree remove', async () => {
  const fixture = await ownershipFixture();
  try {
    await addOwnedWorktree(
      fixture.owner,
      'A',
      fixture.input.aWorktree,
      fixture.checked,
    );
    await rm(fixture.input.aWorktree, { recursive: true, force: true });
    await mkdir(fixture.input.aWorktree);
    await writeFile(join(fixture.input.aWorktree, 'foreign.txt'), 'replacement\n');
    const replacement = await fixture.deps.pathIdentity(fixture.input.aWorktree);

    const cleanup = await cleanupProbeOwnership(fixture.owner, fixture.checked);
    assert.equal(fixture.removeCalls.length, 0);
    assert.deepEqual(await fixture.deps.pathIdentity(fixture.input.aWorktree), replacement);
    assert.equal(await readFile(join(fixture.input.aWorktree, 'foreign.txt'), 'utf8'), 'replacement\n');
    assert.match(cleanup.worktrees[0].error, /ownership changed/);
    assert.equal(cleanup.foreign.preserved, true);
  } finally {
    await fixture.dispose();
  }
});

test('task-root replacement preserves both the replacement and claimed worktree', async () => {
  const fixture = await ownershipFixture();
  try {
    await addOwnedWorktree(
      fixture.owner,
      'A',
      fixture.input.aWorktree,
      fixture.checked,
    );
    await rm(fixture.taskRoot, { recursive: true, force: true });
    await mkdir(fixture.taskRoot);
    await writeFile(join(fixture.taskRoot, 'foreign.txt'), 'task-root replacement\n');
    const replacement = await fixture.deps.pathIdentity(fixture.taskRoot);

    const cleanup = await cleanupProbeOwnership(fixture.owner, fixture.checked);
    assert.equal(fixture.removeCalls.length, 0);
    assert.deepEqual(await fixture.deps.pathIdentity(fixture.taskRoot), replacement);
    assert.equal(await readFile(join(fixture.taskRoot, 'foreign.txt'), 'utf8'), 'task-root replacement\n');
    assert.notEqual(await fixture.deps.pathIdentity(fixture.input.aWorktree), null);
    assert.equal(cleanup.taskRoot.markerVerifiedBeforeRemoval, false);
    assert.equal(cleanup.foreign.preserved, true);
  } finally {
    await fixture.dispose();
  }
});

test('remove failure is reported without force or recursive fallback', async () => {
  const fixture = await ownershipFixture({ removeMode: 'fail' });
  try {
    await addOwnedWorktree(
      fixture.owner,
      'A',
      fixture.input.aWorktree,
      fixture.checked,
    );
    const cleanup = await cleanupProbeOwnership(fixture.owner, fixture.checked);
    assert.equal(fixture.removeCalls.length, 1);
    assert.equal(fixture.removeCalls[0].includes('--force'), false);
    assert.notEqual(await fixture.deps.pathIdentity(fixture.input.aWorktree), null);
    assert.match(cleanup.worktrees[0].error, /remove failure/);
    assert.equal(cleanup.taskRoot.absent, false);
    assert.equal(cleanup.taskRoot.retainedForOwnership, true);
    await access(cleanup.worktrees[0].claimPath);
  } finally {
    await fixture.dispose();
  }
});

test('ledger install flushes and closes before a no-clobber hard link', async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'skiff-ledger-order-')));
  const destination = join(root, 'ledger.json');
  const events = [];
  try {
    const evidence = await installLedgerNoClobber(destination, { status: 'PASS' }, {
      nonce,
      openFile: async (...args) => {
        const handle = await open(...args);
        return {
          stat: (...values) => handle.stat(...values),
          writeFile: async (...values) => { events.push('write'); return handle.writeFile(...values); },
          sync: async () => { events.push('flush'); return handle.sync(); },
          close: async () => { events.push('close'); return handle.close(); },
        };
      },
      linkFile: async (...args) => { events.push('link'); return link(...args); },
    });
    assert.deepEqual(events, ['write', 'flush', 'close', 'link']);
    assert.equal(evidence.ownedTemporaryAbsent, true);
    assert.equal(await absent(ledgerTemporaryPath(destination, nonce)), true);
    assert.deepEqual(JSON.parse(await readFile(destination, 'utf8')), { status: 'PASS' });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('ledger destination race preserves foreign bytes and cleans the owned temporary', async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'skiff-ledger-race-')));
  const destination = join(root, 'ledger.json');
  let failure;
  try {
    try {
      await installLedgerNoClobber(destination, { status: 'PASS' }, {
        nonce,
        linkFile: async (source, target) => {
          await writeFile(target, 'foreign ledger\n', { flag: 'wx' });
          return link(source, target);
        },
      });
    } catch (error) {
      failure = error;
    }
    assert.match(failure?.message, /EEXIST/);
    assert.equal(failure.ledgerInstallEvidence.ownedTemporaryAbsent, true);
    assert.equal(failure.ledgerInstallEvidence.foreignDestinationPreserved, true);
    assert.equal(await readFile(destination, 'utf8'), 'foreign ledger\n');
    assert.equal(await absent(ledgerTemporaryPath(destination, nonce)), true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('ledger temporary write failure removes its partial owned file', async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'skiff-ledger-write-fail-')));
  const destination = join(root, 'ledger.json');
  let failure;
  try {
    try {
      await installLedgerNoClobber(destination, { status: 'PASS' }, {
        nonce,
        openFile: async (...args) => {
          const handle = await open(...args);
          return {
            stat: (...values) => handle.stat(...values),
            writeFile: async () => {
              await handle.writeFile('partial');
              throw new Error('injected temporary write failure');
            },
            sync: () => handle.sync(),
            close: () => handle.close(),
          };
        },
      });
    } catch (error) {
      failure = error;
    }
    assert.match(failure?.message, /injected temporary write failure/);
    assert.equal(failure.ledgerInstallEvidence.ownedTemporaryAbsent, true);
    assert.equal(await absent(ledgerTemporaryPath(destination, nonce)), true);
    assert.equal(await absent(destination), true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

async function ownershipFixture({ addMode = 'success', removeMode = 'success' } = {}) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'skiff-probe-ownership-')));
  const integrationRoot = join(root, 'integration');
  await mkdir(integrationRoot);
  const taskRoot = await realpath(await mkdtemp(join(root, '.task-')));
  const input = {
    integrationRoot,
    candidate,
    aWorktree: join(root, 'a'),
    bWorktree: join(root, 'b'),
  };
  const deps = createProbeDependencies();
  const ledger = { probeNonce: nonce };
  const registry = new Map([[integrationRoot, { head: candidate, detached: false }]]);
  const removeCalls = [];
  const checked = async (_deps, command, args) => {
    assert.equal(command, 'git');
    if (args.includes('list')) {
      return successfulOutcome(registryOutput(registry));
    }
    if (args.includes('add')) {
      const path = args[args.indexOf('add') + 2];
      if (addMode === 'registry-only') {
        registry.set(path, { head: candidate, detached: true });
        throw new Error('registry-only failure');
      }
      await mkdir(path);
      if (addMode === 'foreign-path') {
        const label = path === input.aWorktree ? 'A' : 'B';
        await writeFile(join(path, 'foreign.txt'), `${label}-foreign\n`);
        throw new Error('foreign path failure');
      }
      const adminPath = join(integrationRoot, `.git-worktree-${labelForPath(path, input)}`);
      await mkdir(adminPath);
      await writeFile(join(path, '.git'), `gitdir: ${adminPath}\n`);
      registry.set(path, { head: candidate, detached: true, adminPath });
      if (addMode === 'partial-owned') throw new Error('partial add failure');
      return successfulOutcome();
    }
    if (args.includes('remove')) {
      removeCalls.push([...args]);
      const path = args[args.indexOf('remove') + 1];
      if (removeMode === 'fail') throw new Error('remove failure');
      await rm(path, { recursive: true, force: true });
      await rm(registry.get(path).adminPath, { recursive: true, force: true });
      registry.delete(path);
      return successfulOutcome();
    }
    throw new Error(`unexpected git arguments: ${args.join(' ')}`);
  };
  const owner = await createProbeOwnership({ input, deps, ledger, taskRoot });
  return {
    root,
    taskRoot,
    input,
    deps,
    owner,
    checked,
    removeCalls,
    dispose: () => rm(root, { recursive: true, force: true }),
  };
}

function labelForPath(path, input) {
  return path === input.aWorktree ? 'a' : 'b';
}

function registryOutput(registry) {
  return [...registry].map(([path, entry]) => [
    `worktree ${path}`,
    `HEAD ${entry.head}`,
    entry.detached ? 'detached' : 'branch refs/heads/integration',
    '',
  ].join('\0')).join('\0');
}

function successfulOutcome(stdout = '') {
  return { code: 0, stdout, stderr: '' };
}

async function absent(path) {
  try {
    await access(path);
    return false;
  } catch (error) {
    if (error?.code === 'ENOENT') return true;
    throw error;
  }
}
