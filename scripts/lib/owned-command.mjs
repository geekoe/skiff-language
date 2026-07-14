import { spawn as spawnOwnedChild } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

import { childCompletion } from './command-execution.mjs';
import {
  commandExecutionError,
  safeErrorClone,
  safeSpawnFailure,
} from './command-execution-internal.mjs';

const DEFAULT_STOP_TIMEOUT_MS = 5_000;

export async function runOwnedCommand(command, args, {
  cwd,
  env = process.env,
  signal,
  stdio = 'inherit',
  stopTimeoutMs = DEFAULT_STOP_TIMEOUT_MS,
} = {}) {
  if (signal?.aborted) {
    throw safeInterruption(command, signal.reason);
  }
  const ownsProcessGroup = process.platform !== 'win32';
  let child;
  try {
    // child-process-owner: owned-process-group
    child = spawnOwnedChild(command, args, {
      cwd,
      env,
      stdio,
      detached: ownsProcessGroup,
    });
  } catch (error) {
    throw commandExecutionError(command, {
      code: null,
      signal: null,
      error: safeSpawnFailure(command, error),
    });
  }
  const completion = childCompletion(child);
  let termination;
  const abort = () => {
    termination ??= terminateOwnedChild(child, {
      ownsProcessGroup,
      stopTimeoutMs,
    });
  };
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) {
    abort();
  }

  let outcome;
  try {
    outcome = await completion;
  } finally {
    signal?.removeEventListener('abort', abort);
  }
  if (termination !== undefined) {
    const interruption = safeInterruption(command, signal?.reason);
    try {
      await termination;
    } catch (terminationError) {
      const cleanupError = safeErrorClone(
        terminationError,
        'owned command cleanup failed',
      );
      throw new AggregateError(
        [interruption, cleanupError],
        `${interruption.message}; owned command cleanup failed: ${cleanupError.message}`,
      );
    }
    throw interruption;
  }
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    throw commandExecutionError(command, outcome);
  }
}

async function terminateOwnedChild(child, { ownsProcessGroup, stopTimeoutMs }) {
  signalOwnedChild(child, 'SIGTERM', ownsProcessGroup);
  if (await waitUntilStopped(child, ownsProcessGroup, stopTimeoutMs)) {
    return;
  }
  signalOwnedChild(child, 'SIGKILL', ownsProcessGroup);
  if (!await waitUntilStopped(child, ownsProcessGroup, stopTimeoutMs)) {
    const owner = ownsProcessGroup ? `process group ${child.pid}` : `process ${child.pid}`;
    throw new Error(`owned command ${owner} did not stop`);
  }
}

async function waitUntilStopped(child, ownsProcessGroup, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (!ownedChildAlive(child, ownsProcessGroup)) {
      return true;
    }
    await delay(50);
  }
  return !ownedChildAlive(child, ownsProcessGroup);
}

function signalOwnedChild(child, signal, ownsProcessGroup) {
  if (child.pid === undefined) {
    return;
  }
  try {
    if (ownsProcessGroup) {
      process.kill(-child.pid, signal);
    } else {
      child.kill(signal);
    }
  } catch (error) {
    if (error?.code !== 'ESRCH') {
      throw error;
    }
  }
}

function ownedChildAlive(child, ownsProcessGroup) {
  if (child.pid === undefined) {
    return false;
  }
  try {
    if (ownsProcessGroup) {
      process.kill(-child.pid, 0);
    } else {
      process.kill(child.pid, 0);
    }
    return true;
  } catch (error) {
    return error?.code !== 'ESRCH';
  }
}

function safeInterruption(command, reason) {
  return safeErrorClone(reason, `${command} interrupted`);
}
