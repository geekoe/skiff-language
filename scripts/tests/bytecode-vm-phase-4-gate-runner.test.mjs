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

import { commandEnvironmentIdentity } from '../lib/bytecode-vm-phase-4-contract.mjs';
import {
  parsePhase4GateArgs,
  runPhase4Gate,
} from '../lib/bytecode-vm-phase-4-gate-runner.mjs';
import { COMMIT, TREE, tap } from './bytecode-vm-phase-4-gate-fixture.mjs';

test('runner accepts the independent Phase 4 caller inputs', () => {
  assert.deepEqual(parsePhase4GateArgs([], { env: {
    SKIFF_BYTECODE_VM_PHASE4_EVIDENCE_DIR: '/absolute/evidence',
    SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_COMMIT: COMMIT,
    SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_TREE: TREE,
  } }), {
    help: false,
    outputDir: '/absolute/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner rejects each missing caller input before capturing any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase4-required-inputs-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  try {
    await mkdir(repoRoot);
    for (const { name, pattern } of [
      { name: 'SKIFF_BYTECODE_VM_PHASE4_EVIDENCE_DIR', pattern: /--output-dir/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_COMMIT', pattern: /--candidate/ },
      { name: 'SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_TREE', pattern: /--tree/ },
    ]) {
      const env = {
        SKIFF_BYTECODE_VM_PHASE4_EVIDENCE_DIR: outputDir,
        SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_COMMIT: COMMIT,
        SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_TREE: TREE,
      };
      delete env[name];
      let calls = 0;
      await assert.rejects(runPhase4Gate(parsePhase4GateArgs([], { env }), {
        repoRoot,
        capture: async () => { calls += 1; },
      }), pattern);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

test('runner receipts all sixty-seven commands and freezes the actual environment', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase4-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const ambient = {
    PATH: '/usr/bin:/bin',
    GIT_PAGER: 'cat',
    PHASE4_UNRECORDED_ENV: 'before',
  };
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase4Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      capture: async (command, args, { env }) => {
        observed.push(env.PHASE4_UNRECORDED_ENV);
        ambient.PHASE4_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 67, passed: 67, failed: 0 });
    const regression = result.manifest.commands.filter(({ id }) => id.startsWith('phase-3-regression-'));
    const phase4 = result.manifest.commands.filter(({ id }) => !id.startsWith('phase-3-regression-'));
    assert.equal(regression.length, 34);
    assert.equal(phase4.length, 33);
    assert.equal(regression.every(({ status }) => status === 'PASS'), true);
    assert.equal(observed.length, 67);
    assert.equal(observed.every((value) => value === 'before'), true);
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', 'phase-4-gate-self-tests.receipt.json'),
      'utf8',
    ));
    assert.deepEqual(receipt.identity.environment, commandEnvironmentIdentity({
      PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PHASE4_UNRECORDED_ENV: 'before',
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
