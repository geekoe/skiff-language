import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  defaultBuildStatusPath,
  formatBuildStatusSuffix,
  readBuildStatus,
  summarizeBuildError,
  writeBuildStatus,
} from '../lib/dev-sync-build-status.mjs';
import { runDevSyncOnce } from '../skiff-dev-sync.mjs';
import { writePackageRoot } from './package-service-fixtures.mjs';

test('build status file round-trips ok and failed states', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-build-status-'));
  try {
    const path = join(temp, 'last-build.json');
    assert.equal(await readBuildStatus(path), null);

    await writeBuildStatus({
      path,
      state: 'failed',
      updatedAt: '2026-08-03T02:00:00.000Z',
      nextRetryAt: '2026-08-03T02:00:20.000Z',
      error: 'package compile failed',
      attempt: 3,
    });
    const failed = await readBuildStatus(path);
    assert.equal(failed.state, 'failed');
    assert.equal(failed.error, 'package compile failed');
    assert.equal(failed.attempt, 3);
    assert.equal(failed.nextRetryAt, '2026-08-03T02:00:20.000Z');

    await writeBuildStatus({
      path,
      state: 'ok',
      updatedAt: '2026-08-03T02:05:00.000Z',
      attempt: 0,
    });
    const ok = await readBuildStatus(path);
    assert.equal(ok.state, 'ok');
    assert.equal(ok.error, null);
    assert.equal(ok.nextRetryAt, null);
    const raw = JSON.parse(await readFile(path, 'utf8'));
    assert.equal(raw.schemaVersion, 1);
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});

test('build status reader rejects malformed files', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-build-status-invalid-'));
  try {
    const path = join(temp, 'last-build.json');
    await writeBuildStatus({ path, state: 'ok', updatedAt: 'x' });
    await (await import('node:fs/promises')).writeFile(path, '{not json');
    assert.equal(await readBuildStatus(path), null);
    await (await import('node:fs/promises')).writeFile(
      path,
      `${JSON.stringify({ schemaVersion: 1, state: 'unknown' })}\n`,
    );
    assert.equal(await readBuildStatus(path), null);
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});

test('build status writer rejects unknown states', async () => {
  await assert.rejects(
    writeBuildStatus({ path: '/tmp/unused.json', state: 'unknown', updatedAt: 'x' }),
    /invalid dev sync build status state/,
  );
});

test('build error summary keeps the first line and truncates', () => {
  assert.equal(
    summarizeBuildError(new Error('first line\nsecond line')),
    'first line',
  );
  const long = `x${'y'.repeat(300)}`;
  const summary = summarizeBuildError(long);
  assert.equal(summary.length, 240);
  assert.ok(summary.endsWith('...'));
});

test('build status suffix formats for status output', () => {
  const now = Date.parse('2026-08-03T02:00:10.000Z');
  assert.equal(formatBuildStatusSuffix(null, now), '');
  assert.equal(
    formatBuildStatusSuffix({ state: 'ok' }, now),
    ' build=ok',
  );
  assert.equal(
    formatBuildStatusSuffix(
      {
        state: 'failed',
        nextRetryAt: '2026-08-03T02:00:20.000Z',
      },
      now,
    ),
    ' build=failed retryIn=10s',
  );
  assert.equal(
    formatBuildStatusSuffix({ state: 'failed', nextRetryAt: null }, now),
    ' build=failed',
  );
  assert.equal(
    formatBuildStatusSuffix({ state: 'building' }, now),
    ' build=building',
  );
});

test('one-shot dev sync rejects on package compile failure', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'skiff-dev-sync-one-shot-'));
  try {
    const root = join(temp, 'provider');
    await writePackageRoot(root, { packageId: 'example.com/provider' });
    await assert.rejects(
      runDevSyncOnce({
        roots: [{ kind: 'package', root }],
        environment: 'dev',
        artifactRoot: join(temp, 'artifacts'),
        compilerRunner: async ({ kind }) => {
          if (kind === 'package') {
            throw new Error('package compile failed');
          }
          throw new Error(`unexpected kind ${kind}`);
        },
      }),
      /package compile failed/,
    );
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});

test('default build status path sits next to the watch config', () => {
  assert.equal(
    defaultBuildStatusPath('/home/dev/watch.json'),
    '/home/dev/last-build.json',
  );
});
