import assert from 'node:assert/strict';
import { symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';

import { checkPhase0Evidence } from '../lib/bytecode-vm-phase-0-evidence.mjs';
import {
  COMMIT,
  TREE,
  withEvidenceBundle,
} from './bytecode-vm-phase-0-gate-fixture.mjs';

test('checker accepts exact commands on one clean candidate', async () => {
  await withEvidenceBundle({}, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'PASS');
    assert.deepEqual(bundle.manifest.counts.commands, { total: 17, passed: 17, failed: 0 });
    assert.equal(bundle.manifest.counts.tests.declared, 11);
    assert.equal((await check(bundle)).verdict, 'PASS');
  });
});

for (const fixture of [
  { name: 'dirty candidate', options: { dirty: true }, code: 'candidate.dirty' },
  { name: 'stale candidate', options: { stale: true }, code: 'candidate.stale' },
  { name: 'missing command', options: { missingId: 'request-scalar-regression' }, code: 'command.missing' },
  { name: 'interrupted command', options: { interruptedId: 'host-production-composition' }, code: 'command.interrupted' },
  { name: 'candidate changed before closure', options: { closureChanged: true }, code: 'candidate.stale' },
]) {
  test(`checker derives FAIL for ${fixture.name}`, async () => {
    await withEvidenceBundle(fixture.options, async (bundle) => {
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
  test(`checker rejects ${fixture.name} Node test completion`, async () => {
    await withEvidenceBundle({ nodeCounts: fixture.nodeCounts }, async (bundle) => {
      assert.equal(bundle.manifest.verdict, 'FAIL');
      assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.test-count'), true);
    });
  });
}

test('checker rejects a command log changed after evidence closure', async () => {
  await withEvidenceBundle({}, async (bundle) => {
    await writeFile(join(bundle.outputDir, 'commands', 'request-scalar-regression.stdout.log'),
      'tampered\n');
    await assert.rejects(check(bundle), /file hash closure/);
  });
});

test('checker rejects actual child environment drift from the bound command snapshot', async () => {
  await withEvidenceBundle({ environmentDriftId: 'request-scalar-regression' }, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'FAIL');
    assert.equal(
      bundle.manifest.failures.some(({ code }) => code === 'command.identity'),
      true,
    );
    assert.equal((await check(bundle)).verdict, 'FAIL');
  });
});

test('checker rejects every internal symlink instead of following or ignoring it', async () => {
  await withEvidenceBundle({}, async (bundle) => {
    await symlink('/dev/null', join(bundle.outputDir, 'internal-link'));
    await assert.rejects(check(bundle), /contains symlink/);
  });
});

function check(bundle) {
  return checkPhase0Evidence(bundle.outputDir, {
    repoRoot: bundle.repoRoot,
    expectedCommit: COMMIT,
    expectedTree: TREE,
    directoryIdentities: bundle.directoryIdentities,
    commandEnvironments: bundle.commandEnvironments,
  });
}
