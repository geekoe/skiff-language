import assert from 'node:assert/strict';
import {
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { commandEnvironmentIdentity } from '../lib/bytecode-vm-phase-5-contract.mjs';
import {
  acquirePhase5CargoLease,
  parsePhase5GateArgs,
  PHASE5_CARGO_TARGET_DIR,
  PHASE5_CARRIER_ENV,
  PHASE5_RUNTIME_BIN_ENV,
  runPhase5Gate,
} from '../lib/bytecode-vm-phase-5-gate-runner.mjs';
import { COMMIT, TREE, tap } from './bytecode-vm-phase-5-gate-fixture.mjs';

test('runner accepts the independent Phase 5 caller inputs', () => {
  assert.deepEqual(parsePhase5GateArgs([], { env: {
    SKIFF_BYTECODE_VM_PHASE5_EVIDENCE_DIR: '/absolute/evidence',
    SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_COMMIT: COMMIT,
    SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_TREE: TREE,
  } }), {
    help: false,
    outputDir: '/absolute/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner rejects each missing caller input before capturing any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase5-required-inputs-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    for (const { name, pattern } of [
      { name: 'SKIFF_BYTECODE_VM_PHASE5_EVIDENCE_DIR', pattern: /--output-dir/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_COMMIT', pattern: /--candidate/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_TREE', pattern: /--tree/ },
    ]) {
      const env = {
        SKIFF_BYTECODE_VM_PHASE5_EVIDENCE_DIR: outputDir,
        SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_COMMIT: COMMIT,
        SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_TREE: TREE,
      };
      delete env[name];
      let calls = 0;
      await assert.rejects(runPhase5Gate(parsePhase5GateArgs([], { env }), {
        repoRoot,
        capture: async () => { calls += 1; },
        acquireCargoLease: fakeCargoLease,
      }), pattern);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('runner receipts all ninety-three commands and freezes the actual environment', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase5-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const ambient = {
    PATH: '/usr/bin:/bin',
    GIT_PAGER: 'cat',
    PHASE5_UNRECORDED_ENV: 'before',
  };
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase5Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      acquireCargoLease: fakeCargoLease,
      capture: async (command, args, { env }) => {
        observed.push(env.PHASE5_UNRECORDED_ENV);
        ambient.PHASE5_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 93, passed: 93, failed: 0 });
    const regression = result.manifest.commands.filter(({ id }) => id.startsWith('phase-4-regression-'));
    const phase5 = result.manifest.commands.filter(({ id }) => !id.startsWith('phase-4-regression-'));
    assert.equal(regression.length, 55);
    assert.equal(phase5.length, 38);
    assert.equal(regression.every(({ status }) => status === 'PASS'), true);
    assert.equal(observed.length, 93);
    assert.equal(observed.every((value) => value === 'before'), true);
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', 'phase-5-gate-self-tests.receipt.json'),
      'utf8',
    ));
    assert.deepEqual(receipt.identity.environment, commandEnvironmentIdentity({
      PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PHASE5_UNRECORDED_ENV: 'before',
      CARGO_TARGET_DIR: PHASE5_CARGO_TARGET_DIR,
      [PHASE5_CARRIER_ENV]: `${outputDir}.carrier`,
      [PHASE5_RUNTIME_BIN_ENV]: `${PHASE5_CARGO_TARGET_DIR}/debug/runtime`,
    }));
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('Cargo lease is exclusive and its release is idempotent', async () => {
  let held = false;
  const makeDirectory = async () => {
    if (held) {
      const error = new Error('exists');
      error.code = 'EEXIST';
      throw error;
    }
    held = true;
  };
  const removeDirectory = async () => { held = false; };
  const release = await acquirePhase5CargoLease('/lease', { makeDirectory, removeDirectory });
  await assert.rejects(
    acquirePhase5CargoLease('/lease', { makeDirectory, removeDirectory }),
    /already held/,
  );
  await release();
  await release();
  assert.equal(held, false);
});

async function fakeCargoLease() {
  return async () => {};
}

function successfulOutcome(command, args) {
  let stdout = '';
  if (command === 'git' && args[0] === 'rev-parse') {
    stdout = `${args[1] === 'HEAD' ? COMMIT : TREE}\n`;
  } else if (command === 'node') {
    stdout = tap();
  } else if (command === 'cargo') {
    if (args[0] === 'fmt' || args[0] === 'clippy') {
      stdout = '';
    } else {
      const exact = args.includes('--exact');
      const passed = exact ? 1 : 3;
      stdout = `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s\n`;
    }
  }
  return { code: 0, signal: null, error: null, stdout, stderr: '' };
}
