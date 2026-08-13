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

import { commandEnvironmentIdentity } from '../lib/bytecode-vm-phase-1-contract.mjs';
import {
  parsePhase1GateArgs,
  runPhase1Gate,
} from '../lib/bytecode-vm-phase-1-gate-runner.mjs';
import { COMMIT, TREE, tap } from './bytecode-vm-phase-1-gate-fixture.mjs';

test('runner accepts the independent Phase 1 caller inputs', () => {
  assert.deepEqual(parsePhase1GateArgs([], { env: {
    SKIFF_BYTECODE_VM_PHASE1_EVIDENCE_DIR: '/absolute/evidence',
    SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_COMMIT: COMMIT,
    SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_TREE: TREE,
  } }), {
    help: false,
    outputDir: '/absolute/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner rejects each missing caller input before capturing any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase1-required-inputs-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    for (const { name, pattern } of [
      { name: 'SKIFF_BYTECODE_VM_PHASE1_EVIDENCE_DIR', pattern: /--output-dir/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_COMMIT', pattern: /--candidate/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_TREE', pattern: /--tree/ },
    ]) {
      const env = {
        SKIFF_BYTECODE_VM_PHASE1_EVIDENCE_DIR: outputDir,
        SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_COMMIT: COMMIT,
        SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_TREE: TREE,
      };
      delete env[name];
      let calls = 0;
      await assert.rejects(runPhase1Gate(parsePhase1GateArgs([], { env }), {
        repoRoot,
        capture: async () => { calls += 1; },
      }), pattern);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('runner receipts all twenty-four commands and freezes the actual environment', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase1-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const ambient = {
    PATH: '/usr/bin:/bin',
    GIT_PAGER: 'cat',
    PHASE1_UNRECORDED_ENV: 'before',
  };
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase1Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      capture: async (command, args, { env }) => {
        observed.push(env.PHASE1_UNRECORDED_ENV);
        ambient.PHASE1_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 24, passed: 24, failed: 0 });
    for (const id of ['k0a-compiler-admission', 'k0a-emission-admission']) {
      assert.deepEqual(result.manifest.commands.find((command) => command.id === id)?.testSummary, {
        format: 'rust', total: 3, passed: 3, failed: 0,
        ignored: 0, measured: 0, filtered: 42, valid: true,
      });
    }
    assert.equal(observed.length, 24);
    assert.equal(observed.every((value) => value === 'before'), true);
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', 'gate-self-tests.receipt.json'),
      'utf8',
    ));
    assert.deepEqual(receipt.identity.environment, commandEnvironmentIdentity({
      PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PHASE1_UNRECORDED_ENV: 'before',
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
