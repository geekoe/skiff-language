import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

const DEFAULT_STOP_TIMEOUT_MS = 5_000;

export async function runOwnedCommand(command, args, {
  cwd,
  env = process.env,
  signal,
  stdio = 'inherit',
  stopTimeoutMs = DEFAULT_STOP_TIMEOUT_MS,
} = {}) {
  signal?.throwIfAborted();
  const ownsProcessGroup = process.platform !== 'win32';
  const child = spawn(command, args, {
    cwd,
    env,
    stdio,
    detached: ownsProcessGroup,
  });
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
    const interruption = signal?.reason ?? new Error(`${command} interrupted`);
    try {
      await termination;
    } catch (terminationError) {
      throw new Error(
        `${errorMessage(interruption)}; owned command cleanup failed: ${errorMessage(terminationError)}`,
        { cause: new AggregateError([interruption, terminationError]) },
      );
    }
    throw interruption;
  }
  if (outcome.error !== undefined) {
    throw outcome.error;
  }
  if (outcome.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} exited with ${outcome.signal ?? outcome.code}`);
  }
}

function childCompletion(child) {
  return new Promise((resolvePromise) => {
    let spawnError;
    child.once('error', (error) => {
      spawnError = error;
    });
    child.once('close', (code, signal) => {
      resolvePromise({ code, signal, error: spawnError });
    });
  });
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

function errorMessage(error) {
  return error?.message || String(error);
}
