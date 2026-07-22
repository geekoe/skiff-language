import assert from 'node:assert/strict';
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  installManagedPidMetadata,
  removeManagedPidMetadata,
} from '../lib/managed-pid-metadata.mjs';

test('managed PID metadata installs with nonce and inode and only its owner removes it', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-pid-owner-'));
  const path = join(root, 'runtime.pid');
  try {
    const owner = await installManagedPidMetadata(path, fixtureMetadata(101));
    const metadata = JSON.parse(await readFile(path, 'utf8'));
    assert.equal(typeof metadata.pidMetadataOwner.nonce, 'string');
    assert.equal(metadata.pidMetadataOwner.nonce, owner.nonce);
    assert.equal(metadata.pidMetadataOwner.device, owner.device);
    assert.equal(metadata.pidMetadataOwner.inode, owner.inode);
    assert.match(owner.device, /^\d+$/);
    assert.match(owner.inode, /^\d+$/);

    assert.deepEqual(await removeManagedPidMetadata(owner), {
      removed: true,
      reason: 'owned',
    });
    await assert.rejects(access(path), { code: 'ENOENT' });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('pre-existing and concurrent PID claims are no-clobber restart blockers', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-pid-no-clobber-'));
  const foreignPath = join(root, 'foreign.pid');
  const racingPath = join(root, 'racing.pid');
  try {
    await writeFile(foreignPath, 'foreign-owner\n');
    const conflict = await rejectionOf(
      installManagedPidMetadata(foreignPath, fixtureMetadata(202)),
    );
    assert.equal(conflict.code, 'EEXIST');
    assert.match(conflict.message, /refusing to replace pre-existing PID metadata/);
    assert.equal(await readFile(foreignPath, 'utf8'), 'foreign-owner\n');

    const claims = await Promise.allSettled([
      installManagedPidMetadata(racingPath, fixtureMetadata(301)),
      installManagedPidMetadata(racingPath, fixtureMetadata(302)),
    ]);
    assert.equal(claims.filter(({ status }) => status === 'fulfilled').length, 1);
    assert.equal(claims.filter(({ status }) => status === 'rejected').length, 1);
    const winner = claims.find(({ status }) => status === 'fulfilled').value;
    const loser = claims.find(({ status }) => status === 'rejected').reason;
    assert.equal(loser.code, 'EEXIST');
    assert.equal(JSON.parse(await readFile(racingPath, 'utf8')).pidMetadataOwner.nonce, winner.nonce);
    assert.deepEqual(await removeManagedPidMetadata(winner), {
      removed: true,
      reason: 'owned',
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('foreign content and inode replacement are preserved by conditional PID cleanup', async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-pid-replacement-'));
  try {
    await t.test('same-inode foreign nonce is preserved', async () => {
      const path = join(root, 'foreign-content.pid');
      const owner = await installManagedPidMetadata(path, fixtureMetadata(401));
      await writeFile(path, 'replacement-with-same-inode\n');
      assert.deepEqual(await removeManagedPidMetadata(owner), {
        removed: false,
        reason: 'foreign',
      });
      assert.equal(await readFile(path, 'utf8'), 'replacement-with-same-inode\n');
    });

    await t.test('replacement inode is preserved', async () => {
      const path = join(root, 'replacement-inode.pid');
      const owner = await installManagedPidMetadata(path, fixtureMetadata(402));
      await rm(path);
      await writeFile(path, 'replacement-with-new-inode\n');
      assert.deepEqual(await removeManagedPidMetadata(owner), {
        removed: false,
        reason: 'replacement',
      });
      assert.equal(await readFile(path, 'utf8'), 'replacement-with-new-inode\n');
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function fixtureMetadata(pid) {
  return {
    schemaVersion: 1,
    component: 'runtime',
    pid,
    pgid: pid,
  };
}

async function rejectionOf(promise) {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  assert.fail('expected PID metadata operation to reject');
}
