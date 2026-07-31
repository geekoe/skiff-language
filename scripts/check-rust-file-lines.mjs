#!/usr/bin/env node

// No-exception gate: every .rs file in the repository must stay at or below
// MAX_FILE_LINES. There is deliberately no baseline/allowlist; if a file
// exceeds the limit it must be split, not exempted.

import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MAX_FILE_LINES = 6533; // current maximum; no exceptions

function runFileLineGate() {
  // child-process-owner: rust-file-line-gate
  const files = execFileSync(
    'rg',
    ['--files', '--glob', '*.rs'],
    { cwd: root, encoding: 'utf8' },
  )
    .trim()
    .split('\n')
    .filter(Boolean);

  // child-process-owner: rust-file-line-gate
  const wc = execFileSync('wc', ['-l', ...files], { cwd: root, encoding: 'utf8' });
  const failures = [];
  for (const line of wc.trimEnd().split('\n')) {
    if (/^\s*\d+\s+total$/.test(line)) continue;
    const match = line.match(/^\s*(\d+)\s+(.+)$/);
    if (!match) continue;
    const count = Number(match[1]);
    const file = match[2];
    if (count > MAX_FILE_LINES) {
      failures.push(`${file}: ${count} lines (limit ${MAX_FILE_LINES})`);
    }
  }

  if (failures.length > 0) {
    for (const failure of failures) {
      console.error(`FAIL ${failure}`);
    }
    console.error(
      'HINT Do not just split the file to get under the limit — that only hides the problem. ' +
        'An oversized file usually signals missing abstraction, duplicated code, or unclear ' +
        'responsibilities; investigate deeper and address the underlying issue first.',
    );
    process.exit(1);
  }
  console.log(`Rust file line gate passed: ${files.length} files, limit ${MAX_FILE_LINES} lines.`);
}

runFileLineGate();
