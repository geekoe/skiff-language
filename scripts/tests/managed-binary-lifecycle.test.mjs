import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { Readable } from 'node:stream';
import test from 'node:test';

import {
  binaryIdentity,
  installManagedBinary,
} from '../lib/managed-binary.mjs';

test('binary identity hashes through a path stream and closes it', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-binary-identity-'));
  const path = join(root, 'binary');
  let closeCount = 0;
  class CountingReadable extends Readable {
    _read() {
      this.push(Buffer.from('payload\n'));
      this.push(null);
    }
    _destroy(error, callback) {
      closeCount += 1;
      callback(error);
    }
  }
  try {
    await writeFile(path, 'payload\n');
    const streams = [];
    const identity = await binaryIdentity(path, {
      stat: async (candidate, options) => {
        assert.equal(candidate, path);
        assert.deepEqual(options, { bigint: true });
        return {
          isFile: () => true,
          dev: 1n,
          ino: 2n,
          size: 8n,
          mtimeNs: 3n,
          ctimeNs: 4n,
        };
      },
      createReadStream: (candidate) => {
        assert.equal(candidate, path);
        streams.push(candidate);
        return new CountingReadable();
      },
    });

    assert.deepEqual(streams, [path]);
    assert.equal(closeCount, 1);
    assert.equal(
      identity.digest,
      createHash('sha256').update('payload\n').digest('hex'),
    );
    assert.equal(identity.size, 8);
    assert.deepEqual(identity.file, {
      device: '1',
      inode: '2',
      size: '8',
      modifiedNs: '3',
      changedNs: '4',
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('same-content managed install repairs executable mode atomically', {
  skip: process.platform === 'win32',
}, async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-managed-mode-'));
  const source = join(root, 'source');
  const destination = join(root, 'bin', 'skiff-compiler');
  try {
    await mkdir(dirname(destination), { recursive: true });
    await writeFile(source, '#!/usr/bin/env node\n');
    await writeFile(destination, '#!/usr/bin/env node\n');
    await chmod(source, 0o755);
    await chmod(destination, 0o644);

    await installManagedBinary(source, destination);

    assert.equal((await stat(destination)).mode & 0o7777, 0o755);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
