import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  captureIsolatedRuntimeLogEvidence,
  ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY,
  ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES,
  renderIsolatedRuntimeLogEvidence,
} from '../lib/isolated-test-runtime-log-evidence.mjs';
import { runInIsolatedTestRuntime } from '../lib/isolated-test-runtime.mjs';

test('isolated failure log evidence hashes raw bytes and bounds redacted tails', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-log-evidence-'));
  const logDir = join(root, 'instance', 'logs');
  const secret = 'F51B_SECRET_SENTINEL';
  const privatePath = `/private/var/tmp/${secret}/runtime.skiff`;
  const runtimeLog = Buffer.from(
    `${'界'.repeat(ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES)}\ntoken=${secret} at ${privatePath}`,
  );
  try {
    await mkdir(logDir, { recursive: true });
    await writeFile(join(logDir, 'runtime.log'), runtimeLog);
    await writeFile(join(logDir, 'runtime.err.log'), '');

    const evidence = await captureIsolatedRuntimeLogEvidence(root);
    const runtime = evidence.logs.find(
      (entry) => entry.component === 'runtime' && entry.stream === 'stdout',
    );
    assert.equal(runtime.bytes, runtimeLog.length);
    assert.equal(runtime.sha256, sha256(runtimeLog));
    assert.equal(runtime.truncated, true);
    assert.ok(Buffer.byteLength(runtime.sanitizedTail) <= ISOLATED_RUNTIME_LOG_TAIL_MAX_BYTES);
    assert.match(runtime.sanitizedTail, /<REDACTED_SECRET>/);
    assert.match(runtime.sanitizedTail, /<PATH>/);
    assert.doesNotMatch(runtime.sanitizedTail, new RegExp(secret));
    assert.doesNotMatch(runtime.sanitizedTail, /\uFFFD/);

    const empty = evidence.logs.find(
      (entry) => entry.component === 'runtime' && entry.stream === 'stderr',
    );
    assert.deepEqual(
      pickLogFacts(empty),
      { missing: false, bytes: 0, sha256: sha256(''), truncated: false, sanitizedTail: '' },
    );
    for (const missing of evidence.logs.filter((entry) => entry.component === 'router')) {
      assert.deepEqual(
        pickLogFacts(missing),
        { missing: true, bytes: 0, sha256: sha256(''), truncated: false, sanitizedTail: '' },
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('failed isolated test keeps evidence after cleanup and combines cleanup failure', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-log-cleanup-'));
  const actions = [];
  const primary = new Error('original test failure');
  let removed = false;
  const dependencies = {
    leasePorts: async () => ({
      ports: [46000, 46001, 46002, 46003],
      release: async () => actions.push('lease'),
    }),
    makeTempRoot: async () => root,
    claimWorkspace: async () => ({ root: { path: root }, nonce: 'owned' }),
    createSourceArtifactRoot: (path) => mkdir(path, { recursive: true }),
    initializeInstance: async () => {
      await mkdir(join(root, 'instance', 'logs'), { recursive: true });
    },
    seedBootstrap: async () => ({}),
    spawnMongo: () => ({ pid: 4241 }),
    waitMongoPrimary: async () => {},
    spawnRouter: async () => {
      await writeFile(
        join(root, 'instance', 'logs', 'router.err.log'),
        'authorization=DO_NOT_RETAIN at /private/owned/config.yml',
      );
      return { pid: 4242 };
    },
    spawnRuntime: () => ({ pid: 4243 }),
    waitReady: async () => {},
    stopProcesses: async () => actions.push('stop'),
    assertPortsClosed: async () => {
      actions.push('ports');
      throw new Error('ports cleanup failed');
    },
    removeOwnedWorkspace: async () => {
      actions.push('workspace');
      removed = true;
      await rm(root, { recursive: true, force: true });
    },
  };
  try {
    await assert.rejects(
      runInIsolatedTestRuntime({
        skiffRoot: '/checkout/skiff',
        baseEnv: { PATH: '/bin' },
        signalTarget: new EventEmitter(),
        dependencies,
        runTest: async () => {
          throw primary;
        },
      }),
      (error) => {
        assert.ok(error.cause instanceof AggregateError);
        assert.strictEqual(error.cause.errors[0], primary);
        assert.match(error.message, /^original test failure;/);
        assert.strictEqual(
          error[ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY],
          primary[ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY],
        );
        return true;
      },
    );
    const property = Object.getOwnPropertyDescriptor(
      primary,
      ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY,
    );
    assert.equal(property.enumerable, true);
    const routerError = property.value.logs.find(
      (entry) => entry.component === 'router' && entry.stream === 'stderr',
    );
    assert.match(routerError.sanitizedTail, /<REDACTED_SECRET>/);
    assert.match(routerError.sanitizedTail, /<PATH>/);
    assert.equal(JSON.stringify(primary).includes('DO_NOT_RETAIN'), false);
    assert.deepEqual(actions, ['stop', 'ports', 'lease']);
    assert.equal(removed, false);
    assert.equal(
      (await readFile(join(root, 'instance', 'logs', 'router.err.log'), 'utf8')).length > 0,
      true,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('isolated failure evidence renderer exposes only non-empty sanitized tails', () => {
  const error = new Error('failed');
  Object.defineProperty(error, ISOLATED_RUNTIME_LOG_EVIDENCE_PROPERTY, {
    value: {
      schemaVersion: 'skiff-isolated-runtime-failure-log-evidence-v1',
      logs: [
        {
          component: 'router',
          stream: 'stderr',
          sanitizedTail: '',
          truncated: false,
        },
        {
          component: 'runtime',
          stream: 'stdout',
          sanitizedTail: 'link failed at <PATH>\n',
          truncated: true,
        },
      ],
    },
  });
  assert.equal(
    renderIsolatedRuntimeLogEvidence(error),
    '[isolated runtime stdout (tail, truncated)]\nlink failed at <PATH>',
  );
  assert.equal(renderIsolatedRuntimeLogEvidence(new Error('plain')), '');
});

function pickLogFacts(log) {
  const { missing, bytes, sha256: digest, truncated, sanitizedTail } = log;
  return { missing, bytes, sha256: digest, truncated, sanitizedTail };
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
