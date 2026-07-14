import { spawn as spawnCommandChild } from 'node:child_process';

import {
  commandExecutionError,
  safeSpawnFailure,
} from './command-execution-internal.mjs';

const completions = new WeakMap();

export async function runAttachedCommand(command, args, {
  cwd,
  env = process.env,
} = {}) {
  const spawned = spawnAttachedChild(command, args, {
    cwd,
    env,
    detached: false,
    stdio: 'inherit',
  });
  if (spawned.error !== null) {
    throw commandExecutionError(command, {
      code: null,
      signal: null,
      error: spawned.error,
    });
  }
  const outcome = await childCompletion(spawned.child);
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    throw commandExecutionError(command, outcome);
  }
}

export async function captureAttachedCommand(command, args, {
  cwd,
  env = process.env,
} = {}) {
  const spawned = spawnAttachedChild(command, args, {
    cwd,
    env,
    detached: false,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (spawned.error !== null) {
    return {
      code: null,
      signal: null,
      stdout: '',
      stderr: '',
      error: spawned.error,
    };
  }

  const { child } = spawned;
  const completion = childCompletion(child);
  let stdout = '';
  let stderr = '';
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  const outcome = await completion;
  return {
    code: outcome.code,
    signal: outcome.signal,
    stdout,
    stderr,
    error: outcome.error,
  };
}

export async function captureCheckedCommand(command, args, options = {}) {
  const outcome = await captureAttachedCommand(command, args, options);
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    throw commandExecutionError(command, outcome, {
      stdout: outcome.stdout,
      stderr: outcome.stderr,
    });
  }
  return { stdout: outcome.stdout, stderr: outcome.stderr };
}

export function childCompletion(child) {
  const existing = completions.get(child);
  if (existing !== undefined) {
    return existing;
  }

  const completion = new Promise((resolvePromise) => {
    let spawnFailure = null;
    let settled = false;
    child.on('error', (error) => {
      if (!settled && spawnFailure === null) {
        spawnFailure = safeSpawnFailure(childCommand(child), error);
      }
    });
    child.once('close', (code, signal) => {
      settled = true;
      resolvePromise({
        code: typeof code === 'number' ? code : null,
        signal: typeof signal === 'string' ? signal : null,
        error: spawnFailure,
      });
    });
  });
  completions.set(child, completion);
  return completion;
}

function spawnAttachedChild(command, args, options) {
  try {
    // child-process-owner: attached-capture-spawn
    const child = spawnCommandChild(command, args, options);
    return { child, error: null };
  } catch (error) {
    return { child: null, error: safeSpawnFailure(command, error) };
  }
}

function childCommand(child) {
  return typeof child.spawnfile === 'string' && child.spawnfile.length > 0
    ? child.spawnfile
    : 'command';
}
