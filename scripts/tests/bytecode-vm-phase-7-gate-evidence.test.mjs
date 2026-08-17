import assert from 'node:assert/strict';
import {
  mkdir,
  readFile,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';

import { checkPhase7Evidence } from '../lib/bytecode-vm-phase-7-evidence.mjs';
import { phase7ExecutionOrder } from '../lib/bytecode-vm-phase-7-contract.mjs';
import {
  COMMIT,
  CONSUMER_ID,
  PRODUCER_ID,
  REPOSITORY,
  TREE,
  withPhase7EvidenceBundle,
} from './bytecode-vm-phase-7-gate-fixture.mjs';

test('checker accepts the all-green control bundle', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'PASS');
    assert.deepEqual(bundle.manifest.counts.commands, { total: 131, passed: 131, failed: 0 });
    assert.equal(bundle.manifest.counts.tests.declared > 0, true);
    assert.equal(bundle.manifest.failures.length, 0);
    assert.equal((await check(bundle)).verdict, 'PASS');
  });
});

test('failed producer yields a BLOCKED consumer without stale consumption', async () => {
  await withPhase7EvidenceBundle({
    failedProducerId: PRODUCER_ID,
    blockedConsumerId: CONSUMER_ID,
  }, async (bundle) => {
    assert.equal(bundle.manifest.verdict, 'FAIL');
    assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.failed'), true);
    const consumer = bundle.manifest.commands.find(({ id }) => id === CONSUMER_ID);
    assert.equal(consumer.status, 'BLOCKED');
    assert.deepEqual(consumer.blockedBy, [PRODUCER_ID]);
    const producer = bundle.manifest.commands.find(({ id }) => id === PRODUCER_ID);
    assert.equal(producer.status, 'FAIL');
    assert.equal((await check(bundle)).verdict, 'FAIL');
  });
});

for (const fixture of [
  { name: 'dirty candidate', options: { dirty: true }, code: 'candidate.dirty' },
  { name: 'stale candidate', options: { stale: true }, code: 'candidate.stale' },
  {
    name: 'missing scenario receipt',
    options: { missingId: 'phase-7-gate-self-tests' },
    code: 'command.missing',
  },
  {
    name: 'missing inherited receipt',
    options: { missingId: 'phase-7-regression-phase-5-regression-phase-5-gate-self-tests' },
    code: 'command.missing',
  },
  {
    name: 'cross-epoch catalog digest',
    options: { crossEpoch: true },
    code: 'catalog.cross-epoch',
  },
  {
    name: 'environment drift',
    options: { environmentDriftId: 'phase-7-gate-self-tests' },
    code: 'command.identity',
  },
]) {
  test(`checker derives FAIL for ${fixture.name}`, async () => {
    await withPhase7EvidenceBundle(fixture.options, async (bundle) => {
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
]) {
  test(`checker rejects ${fixture.name} Node completion`, async () => {
    await withPhase7EvidenceBundle({ nodeCounts: fixture.nodeCounts }, async (bundle) => {
      assert.equal(bundle.manifest.verdict, 'FAIL');
      assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.test-count'), true);
    });
  });
}

for (const fixture of [
  { name: 'ignored', rustCounts: { ignored: 1 } },
  { name: 'zero rust', rustCounts: { passed: 0 } },
]) {
  test(`checker rejects ${fixture.name} Rust completion`, async () => {
    await withPhase7EvidenceBundle({ rustCounts: fixture.rustCounts }, async (bundle) => {
      assert.equal(bundle.manifest.verdict, 'FAIL');
      assert.equal(bundle.manifest.failures.some(({ code }) => code === 'command.test-count'), true);
    });
  });
}

test('checker rejects a reordered receipt chain position', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    await tamperReceipt(bundle, 'phase-7-gate-self-tests', (receipt) => ({
      ...receipt,
      sequence: 9999,
    }));
    const result = await check(bundle);
    assert.equal(result.verdict, 'FAIL');
    assert.equal(result.failures.some(({ code }) => code === 'command.reordered'), true);
  });
});

test('checker rejects a tampered stream log', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    const sequence = receiptSequence(bundle, 'phase-7-gate-self-tests');
    await writeFile(
      join(bundle.outputDir, 'commands', `${sequence}-phase-7-gate-self-tests.stdout.log`),
      'tampered\n',
    );
    const result = await check(bundle);
    assert.equal(result.verdict, 'FAIL');
    assert.equal(result.failures.some(({ code }) => code === 'command.stream'), true);
  });
});

test('checker rejects a broken receipt chain link', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    await tamperReceipt(bundle, CONSUMER_ID, (receipt) => ({
      ...receipt,
      priorReceiptDigest: '0'.repeat(64),
    }));
    const result = await check(bundle);
    assert.equal(result.verdict, 'FAIL');
    assert.equal(result.failures.some(({ code }) => code === 'command.chain'), true);
  });
});

test('checker rejects an unexpected receipt and an unexpected evidence file', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    await writeFile(
      join(bundle.outputDir, 'commands', '999-unknown.receipt.json'),
      '{"id":"unknown"}\n',
    );
    const receiptResult = await check(bundle);
    assert.equal(receiptResult.failures.some(({ code }) => code === 'command.unexpected'), true);
    await rm(join(bundle.outputDir, 'commands', '999-unknown.receipt.json'));
    await writeFile(join(bundle.outputDir, 'stray.json'), '{"stray":true}\n');
    const fileResult = await check(bundle);
    assert.equal(fileResult.verdict, 'FAIL');
    assert.equal(fileResult.failures.some(({ code }) => code === 'evidence.unexpected'), true);
  });
});

test('checker rejects a directory swapped for a symlink', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    await mkdir(join(bundle.outputDir, 'commands-backup'));
    await rm(join(bundle.outputDir, 'commands'), { recursive: true });
    await symlink(join(bundle.outputDir, 'commands-backup'), join(bundle.outputDir, 'commands'));
    await assert.rejects(check(bundle), /directory identity|original regular directory/);
  });
});

test('checker rejects tampered evidence via the file closure', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    const sequence = receiptSequence(bundle, 'phase-7-gate-self-tests');
    await writeFile(
      join(bundle.outputDir, 'commands', `${sequence}-phase-7-gate-self-tests.stderr.log`),
      'unexpected stderr\n',
    );
    const result = await check(bundle);
    assert.equal(result.failures.some(({ code }) => code === 'evidence.tampered'), true);
  });
});

test('checker rejects an allowed-path escape claimed by the manifest', async () => {
  await withPhase7EvidenceBundle({}, async (bundle) => {
    const manifestPath = join(bundle.outputDir, 'manifest.json');
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    manifest.evidenceFiles.push({
      path: 'escaped/outside.json',
      bytes: 2,
      sha256: '0'.repeat(64),
    });
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
    const result = await check(bundle);
    assert.equal(result.verdict, 'FAIL');
    assert.equal(result.failures.some(({ code }) => code === 'evidence.allowed'), true);
  });
});

function check(bundle) {
  return checkPhase7Evidence(bundle.outputDir, {
    repoRoot: bundle.repoRoot,
    expectedCommit: COMMIT,
    expectedTree: TREE,
    directoryIdentities: bundle.directoryIdentities,
    commandEnvironments: bundle.commandEnvironments,
  });
}

function receiptSequence(bundle, id) {
  return phase7ExecutionOrder(REPOSITORY).indexOf(id) + 1;
}

async function tamperReceipt(bundle, id, transform) {
  const sequence = receiptSequence(bundle, id);
  const path = join(bundle.outputDir, 'commands', `${sequence}-${id}.receipt.json`);
  const receipt = JSON.parse(await readFile(path, 'utf8'));
  await writeFile(path, `${JSON.stringify(transform(receipt), null, 2)}\n`);
}