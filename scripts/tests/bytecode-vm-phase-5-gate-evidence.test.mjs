import assert from 'node:assert/strict';
import { symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';

import { checkPhase5Evidence } from '../lib/bytecode-vm-phase-5-evidence.mjs';
import {
  COMMIT,
  TREE,
  withPhase5EvidenceBundle,
} from './bytecode-vm-phase-5-gate-fixture.mjs';

test('checker accepts exactly ninety-seven receipts on one clean candidate', async () => {
  await withPhase5EvidenceBundle({}, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'PASS');
    assert.deepEqual(bundle.manifest.counts.commands, { total: 97, passed: 97, failed: 0 });
    assert.equal(bundle.manifest.counts.tests.declared > 0, true);
    assert.equal((await check(bundle)).verdict, 'PASS');
  });
});

for (const fixture of [
  { name: 'dirty candidate', options: { dirty: true }, code: 'candidate.dirty' },
  { name: 'stale candidate', options: { stale: true }, code: 'candidate.stale' },
  {
    name: 'missing Phase 4 regression receipt',
    options: { missingId: 'phase-4-regression-phase-4-gate-self-tests' },
    code: 'command.missing',
  },
  {
    name: 'missing Phase 5 scenario receipt',
    options: { missingId: 'phase-5-lifecycle-race-matrix' },
    code: 'command.missing',
  },
  { name: 'missing fresh receipt', options: { missingId: 'fresh-head' }, code: 'command.missing' },
  {
    name: 'interrupted VCP command',
    options: { interruptedId: 'phase-5-vcp-production-composition' },
    code: 'command.interrupted',
  },
  { name: 'candidate changed before closure', options: { closureChanged: true }, code: 'candidate.stale' },
]) {
  test(`checker derives FAIL for ${fixture.name}`, async () => {
    await withPhase5EvidenceBundle(fixture.options, async (bundle) => {
      assert.equal(bundle.manifest.verdict, 'FAIL');
      assert.equal(bundle.manifest.failures.some(({ code }) => code === fixture.code), true);
      assert.equal((await check(bundle)).verdict, 'FAIL');
    });
  });
}

for (const fixture of [
  { name: 'zero', nodeCounts: { total: 0, passed: 0 } },
  { name: 'skip', nodeCounts: { passed: 3, skipped: 1 } },
  { name: 'todo', nodeCounts: { passed: 3, todo: 1 } },
  { name: 'cancel', nodeCounts: { passed: 3, cancelled: 1 } },
]) {
  test(`checker rejects ${fixture.name} Node completion`, async () => {
    await withPhase5EvidenceBundle({ nodeCounts: fixture.nodeCounts }, async (bundle) => {
      assert.equal(bundle.manifest.verdict, 'FAIL');
      assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.test-count'), true);
    });
  });
}

test('checker rejects a command log changed after evidence closure', async () => {
  await withPhase5EvidenceBundle({}, async (bundle) => {
    await writeFile(
      join(bundle.outputDir, 'commands', 'phase-5-gate-self-tests.stdout.log'),
      'tampered\n',
    );
    await assert.rejects(check(bundle), /file hash closure/);
  });
});

test('checker rejects a tampered final fresh receipt', async () => {
  await withPhase5EvidenceBundle({}, async (bundle) => {
    await writeFile(join(bundle.outputDir, 'commands', 'fresh-status.receipt.json'), '{}\n');
    await assert.rejects(check(bundle), /file hash closure/);
  });
});

test('checker rejects actual child environment drift from the bound snapshot', async () => {
  await withPhase5EvidenceBundle({ environmentDriftId: 'phase-5-vcp-production-composition' }, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'FAIL');
    assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.identity'), true);
  });
});

test('checker rejects every internal symlink', async () => {
  await withPhase5EvidenceBundle({}, async (bundle) => {
    await symlink('/dev/null', join(bundle.outputDir, 'internal-link'));
    await assert.rejects(check(bundle), /contains symlink/);
  });
});

function check(bundle) {
  return checkPhase5Evidence(bundle.outputDir, {
    repoRoot: bundle.repoRoot,
    expectedCommit: COMMIT,
    expectedTree: TREE,
    directoryIdentities: bundle.directoryIdentities,
    commandEnvironments: bundle.commandEnvironments,
  });
}
