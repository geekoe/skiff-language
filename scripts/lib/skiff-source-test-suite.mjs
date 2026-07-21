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

export function skiffSourceTestRunnerCargoArgs({ skiffRoot, root, artifactRoot }) {
  return [
    'run',
    '--quiet',
    '--manifest-path',
    join(skiffRoot, 'test-runner', 'Cargo.toml'),
    '--',
    root,
    '--artifact-root',
    artifactRoot,
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
    runTest: async (isolatedEnv, signal, stack) => {
      if (stack?.sourceArtifactRoot === undefined) {
        throw new Error('isolated runtime owner omitted the canonical source artifact root');
      }
      for (const [index, entry] of plan.entries()) {
        log(`[skiff-tests] running ${entry.id}: ${entry.root}`);
        await runCommand(
          'cargo',
          skiffSourceTestRunnerCargoArgs({
            skiffRoot,
            root: entry.absoluteRoot,
            artifactRoot: stack.sourceArtifactRoot,
          }),
          {
            cwd: skiffRoot,
            env: {
              ...isolatedEnv,
              SKIFF_TEST_EXPECTED_GENERATION: String(index),
            },
            signal,
          },
        );
      }
    },
  });
  return plan;
}
