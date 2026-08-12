import assert from 'node:assert/strict';
import { mkdtemp, mkdir, realpath, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  parsePhase0GateArgs,
  runPhase0Gate,
} from '../lib/bytecode-vm-phase-0-gate-runner.mjs';
import { COMMIT, TREE } from './bytecode-vm-phase-0-gate-fixture.mjs';

test('runner argument parser requires explicit caller-designated identities', () => {
  assert.deepEqual(parsePhase0GateArgs([
    '--output-dir', '/tmp/evidence', '--candidate', COMMIT, '--tree', TREE,
  ], { env: {} }), {
    help: false,
    outputDir: '/tmp/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner refuses existing and symlink output paths before any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase0-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const existing = join(temp, 'existing');
  const linked = join(temp, 'linked');
  try {
    await mkdir(repoRoot);
    await mkdir(existing);
    await symlink(existing, linked);
    for (const outputDir of [existing, linked]) {
      let calls = 0;
      await assert.rejects(runPhase0Gate({
        outputDir, expectedCommit: COMMIT, expectedTree: TREE,
      }, {
        repoRoot,
        capture: async () => { calls += 1; },
      }), /must not already exist/);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});
