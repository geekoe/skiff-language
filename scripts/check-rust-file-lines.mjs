#!/usr/bin/env node

// No-exception gate: every .rs file in the repository must stay at or below
// MAX_FILE_LINES. There is deliberately no baseline/allowlist; if a file
// exceeds the limit it must be split, not exempted.

import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MAX_FILE_LINES = 8084; // current maximum; no exceptions

const files = execFileSync(
  'rg',
  ['--files', '--glob', '*.rs'],
  { cwd: root, encoding: 'utf8' },
)
  .trim()
  .split('\n')
  .filter(Boolean);

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
  process.exit(1);
}
console.log(`Rust file line gate passed: ${files.length} files, limit ${MAX_FILE_LINES} lines.`);
