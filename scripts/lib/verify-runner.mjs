import { mkdir, rm } from 'node:fs/promises';
import { join, relative, resolve } from 'node:path';

import { captureOwnedCommand } from './owned-command.mjs';

const TASK_PRIVATE_ROOT_ENV = 'SKIFF_VERIFY_TASK_PRIVATE_ROOT';
const VERIFY_TASKS_RELATIVE_ROOT = ['var', 'verify', 'tasks'];

export function printVerifyPlan(plan, root) {
  console.log(`selectors: ${plan.selectors.join(', ')}`);
  console.log(`tasks: ${plan.tasks.length}`);
  for (const task of plan.tasks) {
    const cwd = relative(root, task.cwd) || '.';
    const execution = task.preconditionError !== undefined
      ? `[blocked: ${task.preconditionError}]`
      : formatCommand(task);
    console.log(
      `- ${task.id} | ${task.kind} | cwd=${cwd} | ${execution}`,
    );
  }
}

export async function runVerifyPlan(plan, root, {
  jobs = 1,
  signal,
} = {}) {
  const tasks = plan?.tasks;
  if (!Array.isArray(tasks) || tasks.length === 0) {
    throw new Error('verify run requires a plan with at least one task');
  }
  if (!Number.isInteger(jobs) || jobs < 1) {
    throw new Error('verify jobs must be a positive integer');
  }
  const budget = jobs;
  for (const task of tasks) {
    const slots = task.slots ?? 1;
    if (!Number.isInteger(slots) || slots < 1) {
      throw new Error(`verify task ${task.id} has invalid slots ${JSON.stringify(slots)}`);
    }
    if (task.preconditionError === undefined && slots > budget) {
      throw new Error(
        `verify task ${task.id} requires ${slots} slots but the jobs budget is ${budget}`,
      );
    }
  }

  const results = new Map();
  const running = new Map();
  let dispatched = 0;
  let aborted = signal?.aborted === true;
  const onAbort = () => {
    aborted = true;
  };
  signal?.addEventListener('abort', onAbort, { once: true });

  try {
    while (results.size < tasks.length && !aborted) {
      while (!aborted && dispatched < tasks.length) {
        const task = tasks[dispatched];
        if (task.preconditionError !== undefined) {
          dispatched += 1;
          recordBlocked(task, results, root);
          continue;
        }
        const slots = task.slots ?? 1;
        const exclusive = task.exclusive === true || task.tier === 'live/manual';
        const used = usedSlots(running);
        if (exclusive) {
          if (running.size > 0) {
            break;
          }
        } else if (hasExclusiveRunning(running) || used + slots > budget) {
          break;
        }
        dispatched += 1;
        startTask(task, running, { root, signal, budget });
      }
      if (running.size === 0) {
        break;
      }
      const settled = await Promise.race(
        [...running.values()].map((entry) => entry.completion),
      );
      running.delete(settled.task.id);
      recordSettled(settled, results, root);
    }

    if (aborted) {
      const pending = [...running.values()];
      await Promise.allSettled(pending.map((entry) => entry.completion));
      for (const entry of pending) {
        running.delete(entry.task.id);
        recordSettled(entry, results, root);
      }
      for (const task of tasks) {
        if (!results.has(task.id)) {
          results.set(task.id, {
            status: 'interrupted',
            reason: 'verify interrupted before task dispatch',
          });
        }
      }
    }
  } finally {
    signal?.removeEventListener('abort', onAbort);
  }

  const orderedResults = tasks.map((task) => ({
    id: task.id,
    ...results.get(task.id),
  }));
  printSummary(tasks, results);
  if (orderedResults.every((result) => result.status === 'passed')) {
    console.log('\nAll selected Skiff verification tasks passed.');
  }
  return { results: orderedResults };
}

function startTask(task, running, context) {
  const entry = {
    task,
    completion: settleTask(task, context),
  };
  running.set(task.id, entry);
  return entry;
}

async function settleTask(task, { root, signal }) {
  const preflightFailures = await runPreflight(task);
  if (preflightFailures.length > 0) {
    return {
      task,
      status: 'failed',
      reason: preflightFailures.join('; '),
    };
  }
  if (signal?.aborted) {
    return {
      task,
      status: 'interrupted',
      reason: 'verify interrupted before task spawn',
    };
  }

  let privateState;
  try {
    privateState = await createTaskPrivateState(task, root);
    const outcome = await captureOwnedCommand(task.command, task.args, {
      cwd: task.cwd,
      env: privateState === undefined
        ? process.env
        : { ...process.env, ...privateState.env },
      signal,
    });
    return settleOutcome(task, outcome, signal);
  } catch (error) {
    return {
      task,
      status: 'failed',
      reason: errorText(error),
    };
  } finally {
    if (privateState !== undefined) {
      try {
        await rm(privateState.privateRoot, { recursive: true, force: true });
      } catch (error) {
        console.error(
          `warning: failed to remove verify task private root ${privateState.privateRoot}: ${errorText(error)}`,
        );
      }
    }
  }
}

async function runPreflight(task) {
  if (task.executionPreflight === undefined) {
    return [];
  }
  try {
    return normalizePreflightResult(await task.executionPreflight(), task.id);
  } catch (error) {
    return [errorText(error)];
  }
}

function normalizePreflightResult(result, taskId) {
  if (result === undefined) {
    return [];
  }
  if (typeof result === 'string' && result.trim().length > 0) {
    return [result];
  }
  if (
    Array.isArray(result)
    && result.every((reason) => typeof reason === 'string' && reason.trim().length > 0)
  ) {
    return result;
  }
  throw new Error(`${taskId} executionPreflight returned an invalid result`);
}

function settleOutcome(task, outcome, signal) {
  const completed = outcome.error === null
    && outcome.code === 0
    && outcome.signal === null;
  if (signal?.aborted && !completed) {
    return {
      task,
      status: 'interrupted',
      reason: errorText(outcome.error) || `${task.command} interrupted`,
      code: outcome.code,
      signal: outcome.signal,
      stdout: outcome.stdout,
      stderr: outcome.stderr,
    };
  }
  if (outcome.error !== null) {
    return {
      task,
      status: 'failed',
      reason: errorText(outcome.error),
      code: outcome.code,
      signal: outcome.signal,
      stdout: outcome.stdout,
      stderr: outcome.stderr,
    };
  }
  if (outcome.code !== 0 || outcome.signal !== null) {
    return {
      task,
      status: 'failed',
      reason: `${task.command} exited with ${outcome.signal ?? outcome.code}`,
      code: outcome.code,
      signal: outcome.signal,
      stdout: outcome.stdout,
      stderr: outcome.stderr,
    };
  }
  return {
    task,
    status: 'passed',
    stdout: outcome.stdout,
    stderr: outcome.stderr,
  };
}

async function createTaskPrivateState(task, root) {
  if (task.mutation === undefined) {
    return undefined;
  }
  const privateRoot = resolve(
    root,
    ...VERIFY_TASKS_RELATIVE_ROOT,
    sanitizeTaskId(task.id),
  );
  const env = {};
  for (const [name, relativePath] of Object.entries(task.mutation.redirect)) {
    const target = join(privateRoot, relativePath);
    await mkdir(target, { recursive: true });
    env[name] = target;
  }
  env[TASK_PRIVATE_ROOT_ENV] = privateRoot;
  return { privateRoot, env };
}

function sanitizeTaskId(id) {
  const sanitized = id
    .replace(/[^A-Za-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return sanitized.length > 0 ? sanitized : 'task';
}

function usedSlots(running) {
  let total = 0;
  for (const entry of running.values()) {
    total += entry.task.slots ?? 1;
  }
  return total;
}

function hasExclusiveRunning(running) {
  for (const entry of running.values()) {
    if (entry.task.exclusive === true || entry.task.tier === 'live/manual') {
      return true;
    }
  }
  return false;
}

function recordBlocked(task, results, root) {
  printTaskBlock(task, { status: 'blocked', reason: task.preconditionError }, root);
  results.set(task.id, { status: 'blocked', reason: task.preconditionError });
}

function recordSettled(settled, results, root) {
  const { task, ...result } = settled;
  printTaskBlock(task, result, root);
  results.set(task.id, result);
}

function printTaskBlock(task, result, root) {
  const cwd = relative(root, task.cwd) || '.';
  console.log(`\n==> ${task.id} (${cwd})`);
  if (task.preconditionError !== undefined) {
    console.log(`[blocked: ${task.preconditionError}]`);
  } else {
    console.log(`$ ${formatCommand(task)}`);
    writeCaptured(result.stdout, process.stdout);
    writeCaptured(result.stderr, process.stderr);
  }
  const statusLine = result.status === 'passed'
    ? 'passed'
    : `${result.status}${result.reason !== undefined ? `: ${result.reason}` : ''}`;
  console.log(`result: ${statusLine}`);
}

function writeCaptured(value, stream) {
  if (typeof value !== 'string' || value.length === 0) {
    return;
  }
  stream.write(value.endsWith('\n') ? value : `${value}\n`);
}

function printSummary(tasks, results) {
  const counts = { passed: 0, failed: 0, blocked: 0, interrupted: 0 };
  const lines = [];
  for (const task of tasks) {
    const result = results.get(task.id);
    counts[result.status] += 1;
    const suffix = result.status === 'passed' ? '' : `: ${result.reason ?? ''}`;
    lines.push(`- ${task.id}: ${result.status}${suffix}`);
  }
  console.log('');
  console.log(
    `Summary (tasks: ${tasks.length} | passed: ${counts.passed} | failed: ${counts.failed} | blocked: ${counts.blocked} | interrupted: ${counts.interrupted})`,
  );
  for (const line of lines) {
    console.log(line);
  }
}

function formatCommand(task) {
  return [task.command, ...(task.displayArgs ?? task.args)].map(quoteForDisplay).join(' ');
}

function quoteForDisplay(value) {
  if (/^[A-Za-z0-9_./:=+@%-]+$/.test(value)) {
    return value;
  }
  return JSON.stringify(value);
}

function errorText(error) {
  if (error instanceof Error) {
    return error.message;
  }
  if (error !== null && typeof error === 'object' && typeof error.message === 'string') {
    return error.message;
  }
  return String(error);
}
