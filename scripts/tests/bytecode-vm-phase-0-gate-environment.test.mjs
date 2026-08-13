import assert from 'node:assert/strict';
import {
  lstat,
  mkdtemp,
  realpath,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { assertBytecodeVmGateEnvironment } from '../lib/bytecode-vm-gate-environment.mjs';
import { runPhase0Gate } from '../lib/bytecode-vm-phase-0-gate-runner.mjs';
import { runPhase1Gate } from '../lib/bytecode-vm-phase-1-gate-runner.mjs';
import { captureOwnedCommand } from '../lib/owned-command.mjs';

const REPOSITORY_CONTROL_VARIABLES = [
  'GIT_DIR',
  'GIT_WORK_TREE',
  'GIT_COMMON_DIR',
  'GIT_INDEX_FILE',
  'GIT_OBJECT_DIRECTORY',
  'GIT_ALTERNATE_OBJECT_DIRECTORIES',
  'GIT_CONFIG',
  'GIT_CONFIG_GLOBAL',
  'GIT_CONFIG_SYSTEM',
  'GIT_CONFIG_NOSYSTEM',
  'GIT_CONFIG_COUNT',
  'GIT_CONFIG_KEY_0',
  'GIT_CONFIG_VALUE_0',
  'GIT_CONFIG_PARAMETERS',
  'GIT_CEILING_DIRECTORIES',
  'GIT_DISCOVERY_ACROSS_FILESYSTEM',
  'GIT_NAMESPACE',
  'GIT_SHALLOW_FILE',
  'GIT_NO_REPLACE_OBJECTS',
  'GIT_REPLACE_REF_BASE',
  'GIT_GRAFT_FILE',
  'GIT_ATTR_NOSYSTEM',
  'GIT_OPTIONAL_LOCKS',
  'GIT_EXEC_PATH',
  'GIT_FUTURE_REPOSITORY_REDIRECT',
];

test('common Gate environment rejects every repository-control Git class', () => {
  for (const name of REPOSITORY_CONTROL_VARIABLES) {
    assert.throws(
      () => assertBytecodeVmGateEnvironment({ PATH: '/usr/bin:/bin', [name]: 'controlled' }),
      new RegExp(`repository-control environment variable\\(s\\): ${name}`),
      name,
    );
  }
  const safe = { PATH: '/usr/bin:/bin', GIT_PAGER: 'cat', PROOF_INPUT: 'kept' };
  assert.equal(assertBytecodeVmGateEnvironment(safe), safe);
});

test('Phase 0 and Phase 1 reject a real two-worktree redirect before evidence or capture', async () => {
  const created = await mkdtemp(join(tmpdir(), 'skiff-gate-git-env-'));
  const temp = await realpath(created);
  const primary = join(temp, 'primary');
  const redirectedWorktree = join(temp, 'redirected');
  const gitEnvironment = { PATH: process.env.PATH ?? '/usr/bin:/bin' };
  try {
    await git(['init', primary], temp, gitEnvironment);
    await git(['-C', primary, 'config', 'user.name', 'Gate Test'], temp, gitEnvironment);
    await git(['-C', primary, 'config', 'user.email', 'gate@example.invalid'], temp, gitEnvironment);
    await git(['-C', primary, 'config', 'commit.gpgSign', 'false'], temp, gitEnvironment);
    await writeFile(join(primary, 'candidate.txt'), 'primary\n');
    await git(['-C', primary, 'add', 'candidate.txt'], temp, gitEnvironment);
    await git(['-C', primary, 'commit', '--no-verify', '-m', 'primary'], temp, gitEnvironment);
    await git(
      ['-C', primary, 'worktree', 'add', '--detach', redirectedWorktree, 'HEAD'],
      temp,
      gitEnvironment,
    );
    await writeFile(join(redirectedWorktree, 'redirected.txt'), 'redirected\n');
    await git(['-C', redirectedWorktree, 'add', 'redirected.txt'], temp, gitEnvironment);
    await git(
      ['-C', redirectedWorktree, 'commit', '--no-verify', '-m', 'redirected'],
      temp,
      gitEnvironment,
    );

    const repoRoot = await realpath(primary);
    const primaryCommit = await git(['-C', repoRoot, 'rev-parse', 'HEAD'], temp, gitEnvironment);
    const primaryTree = await git(['-C', repoRoot, 'rev-parse', 'HEAD^{tree}'], temp, gitEnvironment);
    const redirectedCommit = await git(
      ['-C', redirectedWorktree, 'rev-parse', 'HEAD'],
      temp,
      gitEnvironment,
    );
    const redirectedGitDir = await realpath(await git(
      ['-C', redirectedWorktree, 'rev-parse', '--absolute-git-dir'],
      temp,
      gitEnvironment,
    ));
    assert.notEqual(primaryCommit, redirectedCommit);

    const hostileEnvironment = {
      ...gitEnvironment,
      GIT_DIR: redirectedGitDir,
      GIT_WORK_TREE: redirectedWorktree,
    };
    assert.equal(
      await git(['rev-parse', 'HEAD'], repoRoot, hostileEnvironment),
      redirectedCommit,
      'control: inherited Git variables redirect probes away from cwd',
    );

    let captureCalls = 0;
    for (const [phase, runGate] of [['phase0', runPhase0Gate], ['phase1', runPhase1Gate]]) {
      const outputDir = join(temp, `${phase}-evidence`);
      await assert.rejects(
        runGate({
          outputDir,
          expectedCommit: primaryCommit,
          expectedTree: primaryTree,
        }, {
          repoRoot,
          env: hostileEnvironment,
          capture: async () => { captureCalls += 1; },
        }),
        /GIT_DIR, GIT_WORK_TREE; unset them before invocation/,
      );
      await assert.rejects(lstat(outputDir), (error) => error?.code === 'ENOENT');
    }
    assert.equal(captureCalls, 0);
  } finally {
    await rm(created, { recursive: true, force: true });
  }
});

async function git(args, cwd, env) {
  const outcome = await captureOwnedCommand('git', args, { cwd, env });
  assert.equal(outcome.code, 0, `${args.join(' ')}\n${outcome.stderr}`);
  assert.equal(outcome.signal, null);
  assert.equal(outcome.error, null);
  return outcome.stdout.trim();
}
