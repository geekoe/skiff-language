import assert from 'node:assert/strict';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  buildIsolatedActivationState,
  isolatedConfigSnapshotRecordPath,
} from '../lib/isolated-test-activation-seed.mjs';

test('activation seed is v2 and resolves the exact secure config snapshot record', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-activation-seed-v2-'));
  const artifactRoot = join(root, 'artifacts');
  const profile = 'isolated-test';
  const assemblyIdentity =
    `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
  const snapshotId =
    `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`;
  const configSnapshot = { snapshotId };
  const bootstrap = bootstrapReceipt({
    profile,
    assemblyIdentity,
    configSnapshot,
  });
  try {
    const recordPath = await writeSecureSnapshotRecord(
      artifactRoot,
      profile,
      configSnapshot,
    );
    assert.equal(
      recordPath,
      isolatedConfigSnapshotRecordPath(artifactRoot, configSnapshot),
    );
    assert.deepEqual(
      await buildIsolatedActivationState({
        artifactRoot,
        profile,
        bootstrap,
      }),
      {
        schemaVersion: 'skiff-profile-activation-state-v1',
        profile,
        committed: {
          generation: 0,
          assembly: { assemblyIdentity },
          configSnapshot,
        },
        pending: null,
      },
    );

    const mismatched = JSON.parse(await readFile(recordPath, 'utf8'));
    mismatched.snapshot.snapshotId =
      `skiff-runtime-config-snapshot-v1:${'c'.repeat(32)}`;
    await writeFile(recordPath, JSON.stringify(mismatched));
    await assert.rejects(
      buildIsolatedActivationState({ artifactRoot, profile, bootstrap }),
      /does not match its exact reference/,
    );

    await writeFile(recordPath, JSON.stringify({
      schemaVersion: 'skiff-runtime-config-snapshot-record-v3',
      profile,
      snapshot: configSnapshot,
      deployments: [],
    }));
    await chmod(recordPath, 0o644);
    await assert.rejects(
      buildIsolatedActivationState({ artifactRoot, profile, bootstrap }),
      /0600 regular file/,
    );
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test('activation seed rejects fake refs and receipts without a stored snapshot', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skiff-activation-seed-missing-'));
  const artifactRoot = join(root, 'artifacts');
  const profile = 'isolated-test';
  const bootstrap = bootstrapReceipt({
    profile,
    assemblyIdentity:
      `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`,
    configSnapshot: {
      snapshotId: `skiff-runtime-config-snapshot-v1:${'b'.repeat(32)}`,
    },
  });
  try {
    await mkdir(join(artifactRoot, 'runtime-config', 'snapshots'), {
      mode: 0o700,
      recursive: true,
    });
    await chmod(join(artifactRoot, 'runtime-config'), 0o700);
    await chmod(join(artifactRoot, 'runtime-config', 'snapshots'), 0o700);
    await assert.rejects(
      buildIsolatedActivationState({ artifactRoot, profile, bootstrap }),
      { code: 'ENOENT' },
    );

    await assert.rejects(
      buildIsolatedActivationState({
        artifactRoot,
        profile: 'other',
        bootstrap,
      }),
      /profile or receipt schema is invalid/,
    );
    const v1 = structuredClone(bootstrap);
    v1.schemaVersion = 'skiff-package-service-bootstrap-v1';
    await assert.rejects(
      buildIsolatedActivationState({ artifactRoot, profile, bootstrap: v1 }),
      /profile or receipt schema is invalid/,
    );
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

function bootstrapReceipt({ profile, assemblyIdentity, configSnapshot }) {
  return {
    schemaVersion: 'skiff-package-service-bootstrap-v2',
    profile,
    bootstrap: {
      assembly: { assemblyIdentity },
      configSnapshot,
      generation: 0,
      std: {},
    },
  };
}

async function writeSecureSnapshotRecord(
  artifactRoot,
  profile,
  configSnapshot,
) {
  const snapshotsRoot = join(artifactRoot, 'runtime-config', 'snapshots');
  await mkdir(snapshotsRoot, { mode: 0o700, recursive: true });
  await chmod(join(artifactRoot, 'runtime-config'), 0o700);
  await chmod(snapshotsRoot, 0o700);
  const recordPath =
    isolatedConfigSnapshotRecordPath(artifactRoot, configSnapshot);
  await writeFile(recordPath, JSON.stringify({
    schemaVersion: 'skiff-runtime-config-snapshot-record-v3',
    profile,
    snapshot: configSnapshot,
    deployments: [],
  }), { mode: 0o600 });
  await chmod(recordPath, 0o600);
  return recordPath;
}
