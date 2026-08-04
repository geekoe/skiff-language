import { join } from 'node:path';

import { runAttachedCommand } from './command-execution.mjs';
import { loadStackConfig } from './stack-config.mjs';

export async function buildStack({
  configDir,
  skiffRoot,
  runCommand = runAttachedCommand,
}) {
  const stack = await loadStackConfig(configDir, { skiffRoot, files: ['build.yml'] });
  const invocation = buildStackInvocation({ stack, skiffRoot });
  await runCommand(invocation.command, invocation.args, {
    cwd: skiffRoot,
    env: invocation.env,
  });
  return {
    configDir: stack.configDir,
    target: stack.build.target,
    buildRoot: stack.paths.buildRoot,
    units: stack.build.units,
  };
}

export function buildStackInvocation({ stack, skiffRoot }) {
  const args = [
    join(skiffRoot, 'scripts', 'build-runtime-stack.mjs'),
    '--target',
    stack.build.target,
    '--zig-dir',
    stack.build.zigDir,
    '--build-root',
    stack.paths.buildRoot,
  ];
  if (stack.build.units.length > 0) {
    args.push('--only', stack.build.units.join(','));
  }
  return {
    command: process.execPath,
    args,
    env: {
      ...process.env,
      CARGO_TARGET_DIR: stack.paths.cargoTargetDir,
    },
  };
}
