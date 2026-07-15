import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { captureAttachedCommand } from './command-execution.mjs';
import {
  analyzeClippyRun,
  assertTooManyLinesBaselineMatches,
  parseTooManyLinesBaseline,
} from './rust-clippy-baseline.mjs';

export const RUST_CLIPPY_BASELINE_ARGS = Object.freeze([
  'clippy',
  '--workspace',
  '--all-targets',
  '--no-deps',
  '--message-format=json',
]);

export const RUST_CLIPPY_BASELINE_PATH = Object.freeze([
  'scripts',
  'rust-clippy-too-many-lines-baseline.json',
]);

export async function runRustClippyBaselineCheck({
  root,
  baselinePath,
  captureCommand = captureAttachedCommand,
  env = process.env,
} = {}) {
  if (!root) {
    throw new Error('Rust Clippy baseline check requires the repository root');
  }
  const resolvedBaselinePath = baselinePath ?? join(root, ...RUST_CLIPPY_BASELINE_PATH);
  const baseline = parseTooManyLinesBaseline(await readFile(resolvedBaselinePath, 'utf8'));
  const outcome = await captureCommand('cargo', [...RUST_CLIPPY_BASELINE_ARGS], {
    cwd: root,
    env,
  });
  const report = analyzeClippyRun(outcome, { root });
  assertTooManyLinesBaselineMatches(report.findings, baseline.entries);
  return report;
}

export function formatRustClippyBaselineSummary(report) {
  const advisorySummary = report.advisoryCount === 0
    ? 'no advisory warnings'
    : `${report.advisoryCount} advisory warning(s) across ${report.advisoryCounts.length} lint code(s)`;
  return [
    `Rust Clippy structural baseline passed: ${report.findings.length} clippy::too_many_lines finding(s).`,
    `Other Clippy diagnostics remain advisory: ${advisorySummary}.`,
  ].join('\n');
}
