import { createHash } from 'node:crypto';
import { isAbsolute, join } from 'node:path';

const ROUTER_DIRECTORY = 'router';
const LOCAL_TSX = join(ROUTER_DIRECTORY, 'node_modules', '.bin', 'tsx');

export async function prepareOwnedRouterNodeDependencies({
  root,
  runCommand,
  signal,
}) {
  if (!isAbsolute(root)) throw new Error('owned Router dependency root must be absolute');
  const evidence = {
    status: 'RUNNING',
    phase: 'router-dependencies',
    root,
    install: null,
    tsxExecutable: null,
  };
  try {
    evidence.install = await captureStep(runCommand, 'pnpm', [
      '--dir', ROUTER_DIRECTORY,
      'install',
      '--frozen-lockfile',
      '--offline',
    ], {
      cwd: root,
      signal,
      commandForEvidence: 'pnpm',
    });
    assertStepPassed('install', evidence.install);

    evidence.tsxExecutable = await captureStep(
      runCommand,
      join(root, LOCAL_TSX),
      ['--version'],
      {
        cwd: root,
        signal,
        commandForEvidence: LOCAL_TSX,
      },
    );
    assertStepPassed('tsx-executable', evidence.tsxExecutable);
    evidence.status = 'PASS';
    return evidence;
  } catch (error) {
    evidence.status = 'FAIL';
    const wrapped = new Error(error instanceof Error ? error.message : String(error), {
      cause: error,
    });
    wrapped.nodeDependencyEvidence = evidence;
    throw wrapped;
  }
}

async function captureStep(runCommand, command, args, {
  cwd,
  signal,
  commandForEvidence,
}) {
  let outcome;
  try {
    outcome = await runCommand(command, args, { cwd, signal });
  } catch (error) {
    return {
      status: 'FAIL',
      command: commandForEvidence,
      args: [...args],
      cwd,
      code: null,
      signal: null,
      spawnError: boundedSpawnError(error),
      stdoutBytes: null,
      stdoutSha256: null,
      stderrBytes: null,
      stderrSha256: null,
    };
  }
  const stdout = outcome.stdout ?? '';
  const stderr = outcome.stderr ?? '';
  const passed = outcome.code === 0 && outcome.signal === null && outcome.error == null;
  return {
    status: passed ? 'PASS' : 'FAIL',
    command: commandForEvidence,
    args: [...args],
    cwd,
    code: outcome.code ?? null,
    signal: outcome.signal ?? null,
    spawnError: boundedSpawnError(outcome.error),
    stdoutBytes: Buffer.byteLength(stdout),
    stdoutSha256: sha256(stdout),
    stderrBytes: Buffer.byteLength(stderr),
    stderrSha256: sha256(stderr),
  };
}

function assertStepPassed(step, outcome) {
  if (outcome.status === 'PASS') return;
  const exit = outcome.signal ?? outcome.code ?? outcome.spawnError?.code ?? 'spawn';
  throw new Error(`Gate Router dependency phase ${step} failed (${exit})`);
}

function boundedSpawnError(error) {
  if (error == null) return null;
  return {
    name: typeof error.name === 'string' ? error.name.slice(0, 80) : 'Error',
    code: typeof error.code === 'string' ? error.code.slice(0, 80) : null,
  };
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}
