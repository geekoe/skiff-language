import { relative } from 'node:path';

import { runAttachedCommand } from './command-execution.mjs';

export function printVerifyPlan(plan, root) {
  console.log(`selectors: ${plan.selectors.join(', ')}`);
  console.log(`phases: ${plan.phases.length}`);
  for (const phase of plan.phases) {
    const cwd = relative(root, phase.cwd) || '.';
    const execution = phase.preconditionError !== undefined
      ? `[blocked: ${phase.preconditionError}]`
      : formatCommand(phase);
    console.log(
      `- ${phase.id} | ${phase.kind} | cwd=${cwd} | ${execution}`,
    );
  }
}

export async function runVerifyPlan(plan, root) {
  await assertExecutionPreconditions(plan);
  for (const phase of plan.phases) {
    const cwd = relative(root, phase.cwd) || '.';
    console.log(`\n==> ${phase.id} (${cwd})`);
    console.log(`$ ${formatCommand(phase)}`);
    await runPhase(phase);
  }
  console.log('\nAll selected Skiff verification phases passed.');
}

async function assertExecutionPreconditions(plan) {
  const failures = [];
  for (const phase of plan.phases) {
    if (phase.preconditionError !== undefined) {
      failures.push({ id: phase.id, reason: phase.preconditionError });
    }
  }
  for (const phase of plan.phases) {
    if (phase.executionPreflight === undefined) {
      continue;
    }
    try {
      const result = await phase.executionPreflight();
      for (const reason of normalizePreflightResult(result, phase.id)) {
        failures.push({ id: phase.id, reason });
      }
    } catch (error) {
      failures.push({
        id: phase.id,
        reason: error instanceof Error ? error.message : String(error),
      });
    }
  }
  if (failures.length === 0) {
    return;
  }
  throw new Error([
    'verify plan preflight failed:',
    ...failures.map(({ id, reason }) => `- ${id}: ${reason}`),
  ].join('\n'));
}

function normalizePreflightResult(result, phaseId) {
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
  throw new Error(`${phaseId} executionPreflight returned an invalid result`);
}

async function runPhase(phase) {
  try {
    await runAttachedCommand(phase.command, phase.args, {
      cwd: phase.cwd,
      env: process.env,
    });
  } catch (error) {
    const status = error?.signal ?? error?.code ?? 'UNKNOWN';
    throw new Error(`${phase.id} failed with ${status}`);
  }
}

function formatCommand(phase) {
  return [phase.command, ...(phase.displayArgs ?? phase.args)].map(quoteForDisplay).join(' ');
}

function quoteForDisplay(value) {
  if (/^[A-Za-z0-9_./:=+@%-]+$/.test(value)) {
    return value;
  }
  return JSON.stringify(value);
}
