import { spawn } from 'node:child_process';
import { relative } from 'node:path';

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
  for (const phase of plan.phases) {
    if (phase.preconditionError !== undefined) {
      throw new Error(`${phase.id} cannot run: ${phase.preconditionError}`);
    }
    const cwd = relative(root, phase.cwd) || '.';
    console.log(`\n==> ${phase.id} (${cwd})`);
    console.log(`$ ${formatCommand(phase)}`);
    await runPhase(phase);
  }
  console.log('\nAll selected Skiff verification phases passed.');
}

function runPhase(phase) {
  return new Promise((resolve, reject) => {
    const child = spawn(phase.command, phase.args, {
      cwd: phase.cwd,
      env: process.env,
      stdio: 'inherit',
    });
    child.once('error', reject);
    child.once('exit', (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${phase.id} failed with ${signal ?? code}`));
    });
  });
}

function formatCommand(phase) {
  return [phase.command, ...phase.args].map(quoteForDisplay).join(' ');
}

function quoteForDisplay(value) {
  if (/^[A-Za-z0-9_./:=+@%-]+$/.test(value)) {
    return value;
  }
  return JSON.stringify(value);
}
