import assert from 'node:assert/strict';
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  symlink,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { commandEnvironmentIdentity } from '../lib/bytecode-vm-phase-0-contract.mjs';
import { checkPhase0Evidence } from '../lib/bytecode-vm-phase-0-evidence.mjs';
import { PHASE0_DIRECTORY_IDENTITY_FILE } from '../lib/bytecode-vm-phase-0-evidence-root.mjs';
import {
  parsePhase0GateArgs,
  runPhase0Gate,
} from '../lib/bytecode-vm-phase-0-gate-runner.mjs';
import { COMMIT, TREE, tap } from './bytecode-vm-phase-0-gate-fixture.mjs';

test('runner argument parser requires explicit caller-designated identities', () => {
  assert.deepEqual(parsePhase0GateArgs([
    '--output-dir', '/tmp/evidence', '--candidate', COMMIT, '--tree', TREE,
  ], { env: {} }), {
    help: false,
    outputDir: '/tmp/evidence',
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test('runner refuses existing and symlink output paths before any command', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase0-runner-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const existing = join(temp, 'existing');
  const linked = join(temp, 'linked');
  try {
    await mkdir(repoRoot);
    await mkdir(existing);
    await symlink(existing, linked);
    for (const outputDir of [existing, linked]) {
      let calls = 0;
      await assert.rejects(runPhase0Gate({
        outputDir, expectedCommit: COMMIT, expectedTree: TREE,
      }, {
        repoRoot,
        capture: async () => { calls += 1; },
      }), /must not already exist/);
      assert.equal(calls, 0);
    }
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

for (const target of ['root', 'commands']) {
  test(`runner and checker fail closed when the evidence ${target} is replaced by a symlink`, async () => {
    const created = await mkdtemp(join(tmpdir(), `skiff-phase0-${target}-race-`));
    const temp = await realpath(created);
    const repoRoot = join(temp, 'repo');
    const outputDir = join(temp, 'evidence');
    const preserved = join(temp, `preserved-${target}`);
    const replacement = join(temp, `replacement-${target}`);
    await mkdir(repoRoot);
    await mkdir(replacement);
    let replaced = false;
    try {
      await assert.rejects(runPhase0Gate({
        outputDir, expectedCommit: COMMIT, expectedTree: TREE,
      }, {
        repoRoot,
        env: { PATH: '/usr/bin:/bin' },
        capture: async (command, args) => {
          if (!replaced) {
            replaced = true;
            const victim = target === 'root' ? outputDir : join(outputDir, 'commands');
            await rename(victim, preserved);
            await symlink(replacement, victim);
          }
          return successfulOutcome(command, args);
        },
      }), /evidence directory|symbolic link|ELOOP/);
      assert.deepEqual(await readdir(replacement), []);

      const identityRoot = target === 'root' ? preserved : outputDir;
      const directoryIdentities = JSON.parse(
        await readFile(join(identityRoot, PHASE0_DIRECTORY_IDENTITY_FILE), 'utf8'),
      );
      await assert.rejects(checkPhase0Evidence(outputDir, {
        repoRoot,
        expectedCommit: COMMIT,
        expectedTree: TREE,
        directoryIdentities,
        commandEnvironments: new Map(),
      }), /evidence directory|symbolic link|canonical path/);
    } finally {
      await rm(created, { recursive: true, force: true });
    }
  });
}

test('runner snapshots and receipts the complete actual environment before ambient drift', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-phase0-env-race-'));
  const temp = await realpath(created);
  const repoRoot = join(temp, 'repo');
  const outputDir = join(temp, 'evidence');
  const ambient = { PATH: '/usr/bin:/bin', P0_UNRECORDED_ENV: 'before' };
  const observed = [];
  try {
    await mkdir(repoRoot);
    const result = await runPhase0Gate({
      outputDir, expectedCommit: COMMIT, expectedTree: TREE,
    }, {
      repoRoot,
      env: ambient,
      capture: async (command, args, { env }) => {
        observed.push(env.P0_UNRECORDED_ENV);
        ambient.P0_UNRECORDED_ENV = 'after';
        return successfulOutcome(command, args);
      },
    });
    assert.equal(result.manifest.verdict, 'PASS');
    assert.equal(result.checkerError, null);
    assert.deepEqual(result.manifest.counts.commands, { total: 20, passed: 20, failed: 0 });
    assert.equal(observed.length, 20);
    assert.equal(observed.every((value) => value === 'before'), true);
    const receipt = JSON.parse(await readFile(
      join(outputDir, 'commands', 'gate-self-tests.receipt.json'),
      'utf8',
    ));
    assert.deepEqual(
      receipt.identity.environment,
      commandEnvironmentIdentity({ PATH: '/usr/bin:/bin', P0_UNRECORDED_ENV: 'before' }),
    );
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
    stdout = 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.01s\n';
  }
  return { code: 0, signal: null, error: null, stdout, stderr: '' };
}
