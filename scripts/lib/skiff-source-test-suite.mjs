import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runInIsolatedTestRuntime } from './isolated-test-runtime.mjs';
import { runOwnedCommand } from './owned-command.mjs';
import {
  canonicalSkiffSourceTestRegistry,
  createCanonicalSkiffSourceTestPlan,
} from './skiff-source-test-registry.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const defaultSkiffRoot = resolve(scriptDir, '..', '..');

export function skiffSourceTestRunnerCargoArgs({ skiffRoot, root }) {
  return [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--',
    root,
    '--deny-skips',
    '--require-tests',
  ];
}

export async function runCanonicalSkiffSourceTests({
  skiffRoot = defaultSkiffRoot,
  registry = canonicalSkiffSourceTestRegistry,
  runtimeOwner = runInIsolatedTestRuntime,
  runCommand = runOwnedCommand,
  log = console.log,
} = {}) {
  const plan = createCanonicalSkiffSourceTestPlan({ skiffRoot, registry });
  await runtimeOwner({
    skiffRoot,
    runTest: async (isolatedEnv, signal) => {
      for (const entry of plan) {
        log(`[skiff-tests] running ${entry.id}: ${entry.root}`);
        await runCommand(
          'cargo',
          skiffSourceTestRunnerCargoArgs({
            skiffRoot,
            root: entry.absoluteRoot,
          }),
          {
            cwd: skiffRoot,
            env: isolatedEnv,
            signal,
          },
        );
      }
    },
  });
  return plan;
}
