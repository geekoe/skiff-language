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
  commandEnvironmentIdentity,
  phase6WorkloadSpecs,
} from '../lib/bytecode-vm-phase-6-contract.mjs';
import {
  acquirePhase6CargoLease,
  parsePhase6GateArgs,
  PHASE6_CARGO_TARGET_DIR,
  PHASE6_CARRIER_ENV,
  PHASE6_RUNTIME_BIN_ENV,
  runPhase6Gate,
} from '../lib/bytecode-vm-phase-6-gate-runner.mjs';
import {
  PHASE5_CARRIER_ENV,
  PHASE5_RUNTIME_BIN_ENV,
} from '../lib/bytecode-vm-phase-5-gate-runner.mjs';
import { COMMIT, TREE, tap } from './bytecode-vm-phase-6-gate-fixture.mjs';

test('runner accepts the independent Phase 6 caller inputs', () => {
  assert.deepEqual(parsePhase6GateArgs([], { env: {
    SKIFF_BYTECODE_VM_PHASE6_EVIDENCE_DIR: '/absolute/evidence',
    SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT: COMMIT,
    SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_TREE: TREE,
  } }), {
    help: false,
    outputDir: '/absolute/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner rejects missing caller input before capturing any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase6-required-inputs-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    for (const { name, pattern } of [
      { name: 'SKIFF_BYTECODE_VM_PHASE6_EVIDENCE_DIR', pattern: /--output-dir/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT', pattern: /--candidate/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_TREE', pattern: /--tree/ },
    ]) {
      const env = {
        SKIFF_BYTECODE_VM_PHASE6_EVIDENCE_DIR: outputDir,
        SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT: COMMIT,
        SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_TREE: TREE,
      };
      delete env[name];
      let calls = 0;
      await assert.rejects(runPhase6Gate(parsePhase6GateArgs([], { env }), {
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

test('runner receipts all one hundred twenty three commands and freezes provenance', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase6-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const ambient = {
    PATH: '/usr/bin:/bin',
    GIT_PAGER: 'cat',
    PHASE6_UNRECORDED_ENV: 'before',
  };
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase6Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      acquireCargoLease: fakeCargoLease,
      capture: async (command, args, { env }) => {
        observed.push(env.PHASE6_UNRECORDED_ENV);
        ambient.PHASE6_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 123, passed: 123, failed: 0 });
    const regression = result.manifest.commands.filter(({ id }) => id.startsWith('phase-5-regression-'));
    const phase6 = result.manifest.commands.filter(({ id }) => !id.startsWith('phase-5-regression-'));
    assert.equal(regression.length, 95);
    assert.equal(phase6.length, 28);
    assert.equal(observed.length, 123);
    assert.equal(observed.every((value) => value === 'before'), true);
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', 'phase-6-gate-self-tests.receipt.json'),
      'utf8',
    ));
    assert.deepEqual(receipt.identity.environment, commandEnvironmentIdentity({
      PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PHASE6_UNRECORDED_ENV: 'before',
      CARGO_TARGET_DIR: PHASE6_CARGO_TARGET_DIR,
      [PHASE6_CARRIER_ENV]: `${outputDir}.carrier`,
      [PHASE6_RUNTIME_BIN_ENV]: `${PHASE6_CARGO_TARGET_DIR}/debug/runtime`,
      [PHASE5_CARRIER_ENV]: `${outputDir}.carrier`,
      [PHASE5_RUNTIME_BIN_ENV]: `${PHASE6_CARGO_TARGET_DIR}/debug/runtime`,
    }));
    assert.equal(receipt.identity.sourcePhase, 6);
    assert.equal(receipt.identity.originChain.at(-1).id, 'phase-6-gate-self-tests');
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('runner records every later receipt after one expected-red workload', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase6-no-fail-fast-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase6Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: { PATH: '/usr/bin:/bin' },
      acquireCargoLease: fakeCargoLease,
      capture: async (command, args, { env }) => {
        observed.push([command, ...args].join(' '));
        if (args.includes('service_')) {
          return {
            code: 101,
            signal: null,
            error: null,
            stdout: 'test result: FAILED. 0 passed; 6 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.01s\n',
            stderr: 'Phase 6 service admission gap\n',
          };
        }
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'FAIL');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 123, passed: 122, failed: 1 });
    assert.equal(observed.length, 123, 'one red workload must not truncate the Gate matrix');
    assert.equal(
      result.manifest.commands.find(({ id }) => id === 'p6-service-matrix')?.status,
      'FAIL',
    );
    assert.equal(
      result.manifest.commands.find(({ id }) => id === 'p6-actor-router-matrix')?.status,
      'PASS',
      'the Router matrix must still execute after an earlier host red',
    );
    assert.equal(
      result.manifest.commands.find(({ id }) => id === 'fresh-status')?.status,
      'PASS',
      'candidate closure probes must still execute after workload reds',
    );
    await assert.rejects(lstat(`${outputDir}.carrier`), { code: 'ENOENT' });
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
  const release = await acquirePhase6CargoLease('/lease', { makeDirectory, removeDirectory });
  await assert.rejects(
    acquirePhase6CargoLease('/lease', { makeDirectory, removeDirectory }),
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
    const expected = phase6WorkloadSpecs('/candidate')
      .find((spec) => JSON.stringify(spec.args) === JSON.stringify(args))?.expectedTests;
    stdout = tap({ total: expected ?? 14, passed: expected ?? 14 });
  } else if (command === 'cargo') {
    if (args[0] === 'fmt' || args[0] === 'clippy') {
      stdout = '';
    } else {
      const exact = args.includes('--exact');
      const expected = phase6WorkloadSpecs('/candidate')
        .find((spec) => JSON.stringify(spec.args) === JSON.stringify(args))?.expectedTests;
      const passed = exact ? 1 : expected ?? 3;
      stdout = `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s\n`;
    }
  }
  return { code: 0, signal: null, error: null, stdout, stderr: '' };
}
