import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  managedMongoOpenFileLimit,
  managedProcessSpawnInvocation,
} from '../lib/managed-process-spawn.mjs';

const mongoSpec = {
  name: 'mongo',
  command: '/opt/homebrew/bin/mongod',
  args: ['--dbpath', '/tmp/skiff mongo', '--port', '27017'],
};

test('managed Mongo raises its open-file limit before exec without changing mongod argv', () => {
  assert.equal(managedMongoOpenFileLimit, 65_536);
  assert.deepEqual(managedProcessSpawnInvocation(mongoSpec, { platform: 'darwin' }), {
    command: '/bin/sh',
    args: [
      '-c',
      'ulimit -n "$1" && shift && exec "$@"',
      'skiff-managed-mongo',
      '65536',
      '/opt/homebrew/bin/mongod',
      '--dbpath',
      '/tmp/skiff mongo',
      '--port',
      '27017',
    ],
  });
});

test('managed Mongo keeps direct spawn on Windows', () => {
  assert.deepEqual(managedProcessSpawnInvocation(mongoSpec, { platform: 'win32' }), {
    command: mongoSpec.command,
    args: mongoSpec.args,
  });
});

test('non-Mongo managed components keep direct spawn', () => {
  const runtime = {
    name: 'runtime',
    command: '/tmp/runtime',
    args: ['/tmp/runtime.yml'],
  };
  assert.deepEqual(managedProcessSpawnInvocation(runtime, { platform: 'darwin' }), {
    command: runtime.command,
    args: runtime.args,
  });
});
