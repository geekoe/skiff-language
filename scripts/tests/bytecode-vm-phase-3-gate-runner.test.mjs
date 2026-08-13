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

import { commandEnvironmentIdentity } from '../lib/bytecode-vm-phase-3-contract.mjs';
import {
  parsePhase3GateArgs,
  runPhase3Gate,
} from '../lib/bytecode-vm-phase-3-gate-runner.mjs';
import { COMMIT, TREE, tap } from './bytecode-vm-phase-3-gate-fixture.mjs';

test('runner accepts the independent Phase 3 caller inputs', () => {
  assert.deepEqual(parsePhase3GateArgs([], { env: {
    SKIFF_BYTECODE_VM_PHASE3_EVIDENCE_DIR: '/absolute/evidence',
    SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_COMMIT: COMMIT,
    SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_TREE: TREE,
  } }), {
    help: false,
    outputDir: '/absolute/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner rejects each missing caller input before capturing any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase3-required-inputs-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    for (const { name, pattern } of [
      { name: 'SKIFF_BYTECODE_VM_PHASE3_EVIDENCE_DIR', pattern: /--output-dir/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_COMMIT', pattern: /--candidate/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_TREE', pattern: /--tree/ },
    ]) {
      const env = {
        SKIFF_BYTECODE_VM_PHASE3_EVIDENCE_DIR: outputDir,
        SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_COMMIT: COMMIT,
        SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_TREE: TREE,
      };
      delete env[name];
      let calls = 0;
      await assert.rejects(runPhase3Gate(parsePhase3GateArgs([], { env }), {
        repoRoot,
        capture: async () => { calls += 1; },
      }), pattern);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('runner receipts all forty-six commands and freezes the actual environment', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase3-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const ambient = {
    PATH: '/usr/bin:/bin',
    GIT_PAGER: 'cat',
    PHASE3_UNRECORDED_ENV: 'before',
  };
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase3Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      capture: async (command, args, { env }) => {
        observed.push(env.PHASE3_UNRECORDED_ENV);
        ambient.PHASE3_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 46, passed: 46, failed: 0 });
    const phase1 = result.manifest.commands.filter(({ id }) => id.startsWith('phase-1-regression-'));
    const phase2 = result.manifest.commands.filter(({ id }) => id.startsWith('phase-2-regression-'));
    const phase3 = result.manifest.commands.filter(({ id }) => id.startsWith('phase-3-') || id.startsWith('k3-') || id.startsWith('c3-'));
    assert.equal(phase1.length, 12);
    assert.equal(phase2.length, 9);
    assert.equal(phase3.length, 13);
    assert.equal(phase1.every(({ status }) => status === 'PASS'), true);
    assert.equal(phase2.every(({ status }) => status === 'PASS'), true);
    assert.equal(observed.length, 46);
    assert.equal(observed.every((value) => value === 'before'), true);
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', 'phase-3-gate-self-tests.receipt.json'),
      'utf8',
    ));
    assert.deepEqual(receipt.identity.environment, commandEnvironmentIdentity({
      PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PHASE3_UNRECORDED_ENV: 'before',
    }));
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

function successfulOutcome(command, args) {
  let stdout = '';
  if (command === 'git' && args[0] === 'rev-parse') {
    stdout = `${args[1] === 'HEAD' ? COMMIT : TREE}\n`;
  } else if (command === 'node') {
    stdout = tap();
  } else if (command === 'cargo') {
    const exact = args.includes('--exact');
    const passed = exact ? 1 : 3;
    stdout = `test result: ok. ${passed} passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s\n`;
  }
  return { code: 0, signal: null, error: null, stdout, stderr: '' };
}
