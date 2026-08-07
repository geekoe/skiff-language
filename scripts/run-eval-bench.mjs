#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const skiffRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const targetRoot = join(skiffRoot, 'test-services', 'eval-bench');
const artifactRoot = mkdtempSync(join(tmpdir(), 'eval-bench-'));
const skiffCli = join(skiffRoot, 'scripts', 'skiff.mjs');

const runs = 3;
const wallMs = [];

function main() {
try {
  for (let i = 0; i < runs; i += 1) {
    const started = process.hrtime.bigint();
    // child-process-owner: eval-bench-spawn
    const result = execFileSync(
      process.execPath,
      [skiffCli, 'test', targetRoot, '--artifact-root', artifactRoot, '--deny-skips', '--require-tests'],
      { cwd: targetRoot, stdio: 'inherit' },
    );
    wallMs.push(Number(process.hrtime.bigint() - started) / 1e6);
    if (result.status !== 0) {
      process.exit(result.status ?? 1);
    }
  }
} finally {
  rmSync(artifactRoot, { recursive: true, force: true });
}

wallMs.sort((a, b) => a - b);
console.log(
  `eval-bench wall time: ${wallMs.map((ms) => `${ms.toFixed(0)}ms`).join(', ')} `
  + `min=${wallMs[0].toFixed(0)}ms median=${wallMs[1].toFixed(0)}ms`,
);
}

main();
