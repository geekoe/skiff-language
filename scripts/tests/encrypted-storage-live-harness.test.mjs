import assert from 'node:assert/strict';
import { isAbsolute } from 'node:path';
import test from 'node:test';

import {
  encryptedStorageTestRunnerArgs,
  repoRoot,
} from '../lib/encrypted-storage-live-harness.mjs';

test('encrypted-storage runner receives the module-owned platform root once', () => {
  const args = encryptedStorageTestRunnerArgs({
    testFile: '/tmp/encrypted.live.test.skiff',
    configPath: '/tmp/test-runner-live.json',
  });
  const indexes = args
    .map((value, index) => (value === '--platform-source-root' ? index : -1))
    .filter((index) => index >= 0);
  assert.equal(indexes.length, 1);
  assert.equal(args[indexes[0] + 1], repoRoot);
  assert.equal(isAbsolute(args[indexes[0] + 1]), true);
});
