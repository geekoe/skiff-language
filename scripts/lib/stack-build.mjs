import { join } from 'node:path';

import { runAttachedCommand } from './command-execution.mjs';
import { loadStackConfig } from './stack-config.mjs';
import { generateLocalInstanceSpec } from './stack-instance-spec.mjs';

export async function buildStack({
  configDir,
  skiffRoot,
  profileOverride,
  runCommand = runAttachedCommand,
  generateInstance = generateLocalInstanceSpec,
}) {
  const stack = await loadStackConfig(configDir, { skiffRoot });
  const profile = profileOverride ?? stack.build.profile;
  if (profile !== 'debug' && profile !== 'release') {
    throw new Error(`build profile must be "debug" or "release"; got ${profile}`);
  }
  const invocation = buildStackInvocation({ stack, skiffRoot, profile });
  await runCommand(invocation.command, invocation.args, {
    cwd: skiffRoot,
    env: invocation.env,
  });
  let instanceSpec = null;
  if (profile === 'debug') {
    instanceSpec = await generateInstance({
      stack: {
        ...stack,
        build: { ...stack.build, profile },
      },
      skiffRoot,
    });
  }
  return {
    configDir: stack.configDir,
    target: stack.build.target,
    profile,
    buildRoot: stack.paths.buildRoot,
    units: stack.build.units,
    instanceSpec,
  };
}

export function buildStackInvocation({ stack, skiffRoot, profile }) {
  const args = [
    join(skiffRoot, 'scripts', 'build-runtime-stack.mjs'),
    '--target',
    stack.build.target,
    '--zig-dir',
    stack.build.zigDir,
    '--build-root',
    stack.paths.buildRoot,
    '--profile',
    profile ?? stack.build.profile,
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
