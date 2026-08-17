import assert from 'node:assert/strict';
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import {
  phase7CandidateSpecs,
  phase7EffectiveTestCount,
  phase7WorkloadProvenance,
  phase7WorkloadSpecs,
  commandEnvironmentIdentity,
} from '../lib/bytecode-vm-phase-7-contract.mjs';
import {
  PHASE7_CARGO_LEASE_DIR,
  PHASE7_CARGO_TARGET_DIR,
  acquirePhase7CargoLease,
  assertPhase7CargoLeaseFree,
  parsePhase7GateArgs,
  removeStalePhase7CargoLease,
  runPhase7Gate,
} from '../lib/bytecode-vm-phase-7-gate-runner.mjs';
import { PHASE7_CARRIER_ENV } from '../lib/bytecode-vm-phase-7-whole-system-harness.mjs';
import { CONSUMER_ID, COMMIT, PRODUCER_ID, TREE, tap } from './bytecode-vm-phase-7-gate-fixture.mjs';

test('runner accepts the independent Phase 7 caller inputs', () => {
  assert.deepEqual(parsePhase7GateArgs([], { env: {
    SKIFF_BYTECODE_VM_PHASE7_EVIDENCE_DIR: '/absolute/evidence',
    SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_COMMIT: COMMIT,
    SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_TREE: TREE,
  } }), {
    help: false,
    outputDir: '/absolute/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
  assert.deepEqual(parsePhase7GateArgs([
    '--candidate', COMMIT,
    '--tree', TREE,
    '--output-dir', '/absolute/other',
  ], { env: {} }), {
    help: false,
    outputDir: '/absolute/other',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
  assert.deepEqual(parsePhase7GateArgs(['--help'], { env: {} }), { help: true });
});

test('runner rejects missing caller input before capturing any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-required-inputs-'));
  const temp = await realpath(created);
  const repoRoot = process.cwd();
  const outputDir = join(temp, 'evidence');
  try {
    for (const { name, pattern } of [
      { name: 'SKIFF_BYTECODE_VM_PHASE7_EVIDENCE_DIR', pattern: /--output-dir/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_COMMIT', pattern: /--candidate/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_TREE', pattern: /--tree/ },
    ]) {
      const env = {
        SKIFF_BYTECODE_VM_PHASE7_EVIDENCE_DIR: outputDir,
        SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_COMMIT: COMMIT,
        SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_TREE: TREE,
      };
      delete env[name];
      let calls = 0;
      await assert.rejects(runPhase7Gate(parsePhase7GateArgs([], { env }), {
        repoRoot,
        capture: async () => { calls += 1; },
        assertCargoLeaseFree: async () => {},
        acquireCargoLease: fakeCargoLease,
      }), pattern);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('runner receipts every command and freezes the Phase 6 provenance chain', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-runner-'));
  const temp = await realpath(created);
  const repoRoot = process.cwd();
  const outputDir = join(temp, 'evidence');
  const ambient = {
    PATH: '/usr/bin:/bin',
    GIT_PAGER: 'cat',
    PHASE7_UNRECORDED_ENV: 'before',
  };
  const observed = [];
  try {
    const result = await runPhase7Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      assertCargoLeaseFree: async () => {},
      acquireCargoLease: fakeCargoLease,
      capture: async (command, args, { env }) => {
        observed.push(env.PHASE7_UNRECORDED_ENV);
        ambient.PHASE7_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 128, passed: 128, failed: 0 });
    assert.equal(observed.length, 128);
    assert.equal(observed.every((value) => value === 'before'), true);
    assert.equal(result.manifest.chain.receipts.length, 129);
    assert.equal(/^[a-f0-9]{64}$/.test(result.manifest.chain.head), true);
    assert.equal(/^[a-f0-9]{64}$/.test(result.manifestSha256), true);
    const inherited = result.manifest.commands.filter(({ sourcePhase }) => sourcePhase < 7);
    assert.equal(inherited.length, 111);
    assert.equal(
      result.manifest.commands.filter(({ status }) => status === 'PASS').length,
      128,
    );
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', '1-preflight-head.receipt.json'),
      'utf8',
    ));
    assert.equal(receipt.sequence, 1);
    assert.equal(/^[a-f0-9]{64}$/.test(receipt.priorReceiptDigest), true);
    const genesis = JSON.parse(await readFile(
      join(outputDir, 'commands', '0-genesis.receipt.json'),
      'utf8',
    ));
    assert.equal(genesis.specCatalogDigest, result.manifest.catalogDigest);
    assert.equal(
      genesis.candidate.commit === COMMIT && genesis.candidate.tree === TREE,
      true,
    );
    assert.deepEqual(
      receipt.identity.environment,
      commandEnvironmentIdentity({
        PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PHASE7_UNRECORDED_ENV: 'before',
        CARGO_TARGET_DIR: PHASE7_CARGO_TARGET_DIR,
        [PHASE7_CARRIER_ENV]: `${outputDir}.carrier`,
        SKIFF_BYTECODE_VM_PHASE6_CARRIER_ROOT: `${outputDir}.carrier`,
        SKIFF_BYTECODE_VM_PHASE6_RUNTIME_BIN: `${PHASE7_CARGO_TARGET_DIR}/debug/runtime`,
        SKIFF_BYTECODE_VM_PHASE5_CARRIER_ROOT: `${outputDir}.carrier`,
        SKIFF_BYTECODE_VM_PHASE5_RUNTIME_BIN: `${PHASE7_CARGO_TARGET_DIR}/debug/runtime`,
      }),
    );
    const provenance = phase7WorkloadProvenance(repoRoot);
    assert.equal(provenance.length, 116);
    assert.equal(
      result.manifest.commands.find(({ id }) => id === CONSUMER_ID)?.blockedBy,
      null,
    );
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('one early red workload does not truncate later commands or the fresh probe', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-no-fail-fast-'));
  const temp = await realpath(created);
  const repoRoot = process.cwd();
  const outputDir = join(temp, 'evidence');
  const observed = [];
  try {
    const result = await runPhase7Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: { PATH: '/usr/bin:/bin' },
      assertCargoLeaseFree: async () => {},
      acquireCargoLease: fakeCargoLease,
      capture: async (command, args) => {
        observed.push([command, ...args].join(' '));
        if (args.join(' ').includes('assertPhase7Catalog')) {
          return {
            code: 1,
            signal: null,
            error: null,
            stdout: '',
            stderr: 'Phase 7 fixture early failure\n',
          };
        }
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'FAIL');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 128, passed: 127, failed: 1 });
    assert.equal(observed.length, 128, 'one red workload must not truncate the Gate matrix');
    const failed = result.manifest.commands.find(({ id }) => id === 'phase-7-catalog-binding');
    assert.equal(failed.status, 'FAIL');
    const later = result.manifest.commands.find(({ id }) => id === CONSUMER_ID);
    assert.equal(later.status, 'PASS', 'later independent commands must still execute');
    assert.equal(
      result.manifest.commands.find(({ id }) => id === 'fresh-status')?.status,
      'PASS',
      'candidate fresh probes must still execute after workload reds',
    );
    await assert.rejects(lstat(`${outputDir}.carrier`), { code: 'ENOENT' });
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('a failed producer BLOCKs its consumer without executing it', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-blocked-'));
  const temp = await realpath(created);
  const repoRoot = process.cwd();
  const outputDir = join(temp, 'evidence');
  const executed = [];
  try {
    const result = await runPhase7Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: { PATH: '/usr/bin:/bin' },
      assertCargoLeaseFree: async () => {},
      acquireCargoLease: fakeCargoLease,
      capture: async (command, args) => {
        executed.push([command, ...args].join(' '));
        if (args.join(' ').includes('whole-system-harness') && args.at(-1) === 'producer') {
          return {
            code: 1,
            signal: null,
            error: null,
            stdout: '',
            stderr: 'Phase 7 producer failure\n',
          };
        }
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'FAIL');
    assert.equal(result.checkerError, null);
    const consumer = result.manifest.commands.find(({ id }) => id === CONSUMER_ID);
    assert.equal(consumer.status, 'BLOCKED');
    assert.deepEqual(consumer.blockedBy, [PRODUCER_ID]);
    assert.equal(
      executed.some((line) => line.includes('whole-system-harness') && line.endsWith('consumer')),
      false,
      'the BLOCKED consumer must never execute against a stale shared artifact',
    );
    const fresh = result.manifest.commands.find(({ id }) => id === 'fresh-status');
    assert.equal(fresh.status, 'PASS');
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', `${receiptNumber(result, CONSUMER_ID)}-${CONSUMER_ID}.receipt.json`),
      'utf8',
    ));
    assert.equal(receipt.outcome.status, 'BLOCKED');
    assert.deepEqual(receipt.outcome.blockedBy, [PRODUCER_ID]);
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('lease acquisition waits politely, refuses contention, and releases idempotently', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-lease-'));
  const temp = await realpath(created);
  const leaseDir = join(temp, 'lease');
  try {
    await assertPhase7CargoLeaseFree(leaseDir);
    const release = await acquirePhase7CargoLease(leaseDir, {
      delayMs: 5,
      timeoutMs: 10_000,
    });
    await assert.rejects(
      assertPhase7CargoLeaseFree(leaseDir),
      /already held/,
    );
    await assert.rejects(
      acquirePhase7CargoLease(leaseDir, { delayMs: 5, timeoutMs: 0 }),
      /stayed held past the wait budget/,
    );
    await release();
    await release();
    await assertPhase7CargoLeaseFree(leaseDir);
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('unsafe stale-lease removal is refused while an owning process remains', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase7-stale-lease-'));
  const temp = await realpath(created);
  const leaseDir = join(temp, 'lease');
  const interruptedEvidencePath = join(temp, 'interrupted-bundle');
  try {
    await mkdir(leaseDir);
    await assert.rejects(
      removeStalePhase7CargoLease(leaseDir, {
        owningProcessAlive: true,
        interruptedEvidencePath,
      }),
      /refusing unsafe stale-lease removal/,
    );
    assert.equal(await pathExists(leaseDir), true, 'the lease must stay intact');
    const removed = await removeStalePhase7CargoLease(leaseDir, {
      owningProcessAlive: false,
      interruptedEvidencePath,
    });
    assert.deepEqual(removed, { leaseDir, interruptedEvidencePath });
    assert.equal(await pathExists(leaseDir), false);
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

function receiptNumber(result, id) {
  return result.manifest.chain.receipts.find((entry) => entry.id === id).sequence;
}

async function fakeCargoLease() {
  return async () => {};
}

function successfulOutcome(command, args) {
  let stdout = '';
  if (command === 'git' && args[0] === 'rev-parse') {
    stdout = `${args[1] === 'HEAD' ? COMMIT : TREE}\n`;
  } else if (command === 'node' && args.includes('--test')) {
    const spec = phase7WorkloadSpecs('/candidate')
      .find((candidate) => JSON.stringify(candidate.args) === JSON.stringify(args));
    const expected = phase7EffectiveTestCount(spec ?? {});
    stdout = tap({ total: expected ?? 14, passed: expected ?? 14 });
  } else if (command === 'node' && args.join(' ').includes('whole-system-harness')
    && args.at(-1) === 'consumer') {
    stdout = tap({ total: 1, passed: 1 });
  } else if (command === 'cargo') {
    if (args[0] === 'fmt' || args[0] === 'clippy') {
      stdout = '';
    } else {
      const spec = phase7WorkloadSpecs('/candidate')
        .find((candidate) => JSON.stringify(candidate.args) === JSON.stringify(args));
      const expected = phase7EffectiveTestCount(spec ?? {});
      const passed = args.includes('--exact') ? 1 : expected ?? 3;
      stdout = `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s\n`;
    }
  }
  return { code: 0, signal: null, error: null, stdout, stderr: '' };
}

async function pathExists(path) {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    return false;
  }
}