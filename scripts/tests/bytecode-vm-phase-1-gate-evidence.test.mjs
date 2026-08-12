import assert from 'node:assert/strict';
import { symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';

import { checkPhase1Evidence } from '../lib/bytecode-vm-phase-1-evidence.mjs';
import {
  COMMIT,
  TREE,
  withPhase1EvidenceBundle,
} from './bytecode-vm-phase-1-gate-fixture.mjs';

test('checker accepts exactly twenty receipts on one clean candidate', async () => {
  await withPhase1EvidenceBundle({}, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'PASS');
    assert.deepEqual(bundle.manifest.counts.commands, { total: 20, passed: 20, failed: 0 });
    assert.equal(bundle.manifest.counts.tests.declared > 0, true);
    assert.equal((await check(bundle)).verdict, 'PASS');
  });
});

for (const fixture of [
  { name: 'dirty candidate', options: { dirty: true }, code: 'candidate.dirty' },
  { name: 'stale candidate', options: { stale: true }, code: 'candidate.stale' },
  { name: 'missing K0 receipt', options: { missingId: 'k0b-tc-production-contract' }, code: 'command.missing' },
  { name: 'missing fresh receipt', options: { missingId: 'fresh-head' }, code: 'command.missing' },
  { name: 'interrupted V1 command', options: { interruptedId: 'tr-v1-production-proof' }, code: 'command.interrupted' },
  { name: 'candidate changed before closure', options: { closureChanged: true }, code: 'candidate.stale' },
]) {
  test(`checker derives FAIL for ${fixture.name}`, async () => {
    await withPhase1EvidenceBundle(fixture.options, async (bundle) => {
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
    await withPhase1EvidenceBundle({ nodeCounts: fixture.nodeCounts }, async (bundle) => {
      assert.equal(bundle.manifest.verdict, 'FAIL');
      assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.test-count'), true);
    });
  });
}

test('checker rejects a command log changed after evidence closure', async () => {
  await withPhase1EvidenceBundle({}, async (bundle) => {
    await writeFile(join(bundle.outputDir, 'commands', 'k0a-compiler-containment.stdout.log'),
      'tampered\n');
    await assert.rejects(check(bundle), /file hash closure/);
  });
});

test('checker rejects a tampered final fresh receipt', async () => {
  await withPhase1EvidenceBundle({}, async (bundle) => {
    await writeFile(join(bundle.outputDir, 'commands', 'fresh-status.receipt.json'), '{}\n');
    await assert.rejects(check(bundle), /file hash closure/);
  });
});

test('checker rejects actual child environment drift from the bound snapshot', async () => {
  await withPhase1EvidenceBundle({ environmentDriftId: 'k0c-request-containment' }, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'FAIL');
    assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.identity'), true);
  });
});

test('checker rejects every internal symlink', async () => {
  await withPhase1EvidenceBundle({}, async (bundle) => {
    await symlink('/dev/null', join(bundle.outputDir, 'internal-link'));
    await assert.rejects(check(bundle), /contains symlink/);
  });
});

function check(bundle) {
  return checkPhase1Evidence(bundle.outputDir, {
    repoRoot: bundle.repoRoot,
    expectedCommit: COMMIT,
    expectedTree: TREE,
    directoryIdentities: bundle.directoryIdentities,
    commandEnvironments: bundle.commandEnvironments,
  });
}
