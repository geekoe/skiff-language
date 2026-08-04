import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import { cargoTargetDir } from './cargo-target-dir.mjs';
import {
  captureAttachedCommand,
  runAttachedCommand,
} from './command-execution.mjs';
import {
  assertRouterProcessSpec,
  resolveRouterProcessSpec,
  routerProcessInvocation,
} from './dev-runtime-paths.mjs';
import { installManagedBinary } from './managed-binary.mjs';

const IDENTITY_LINE_PATTERN = /^skiff-router ([a-f0-9]{64})$/;

export async function runRouterProcessSmoke({ root, env = process.env }) {
  if (!root) {
    throw new Error('router process smoke requires the repository root');
  }
  const devHome = join(root, '.stack', 'dev-home');
  const rustSpec = assertRouterProcessSpec(
    resolveRouterProcessSpec({ devHome, repoRoot: root }),
  );
  const rustInvocation = routerProcessInvocation(rustSpec);

  assertInvocation(rustInvocation, {
    command: rustSpec.rust_binary_path,
    args: [rustSpec.config_path],
  });

  const targetDir = cargoTargetDir(root, env);
  await runAttachedCommand(
    'cargo',
    ['build', '--manifest-path', join(root, 'router', 'Cargo.toml'), '--bin', 'skiff-router'],
    {
      cwd: root,
      env: { ...env, CARGO_TARGET_DIR: targetDir },
    },
  );
  const sourceBinary = join(
    targetDir,
    'debug',
    process.platform === 'win32' ? 'skiff-router.exe' : 'skiff-router',
  );
  await installManagedBinary(sourceBinary, rustSpec.rust_binary_path);

  const identity = await captureAttachedCommand(
    rustSpec.rust_binary_path,
    ['--identity'],
    { cwd: root, env },
  );
  assertCommandOutcome(identity, 'skiff-router --identity');
  const identityLine = identity.stdout.trim();
  const identityMatch = IDENTITY_LINE_PATTERN.exec(identityLine);
  if (identityMatch === null) {
    throw new Error(
      `router process smoke: unexpected identity output ${JSON.stringify(identityLine)}`,
    );
  }
  const binarySha256 = await sha256File(rustSpec.rust_binary_path);
  if (identityMatch[1] !== binarySha256) {
    throw new Error(
      `router process smoke: binary identity mismatch (binary reports ${identityMatch[1]}, file sha256 is ${binarySha256})`,
    );
  }

  const bare = await captureAttachedCommand(rustSpec.rust_binary_path, [], {
    cwd: root,
    env,
  });
  assertCommandOutcome(bare, 'skiff-router');
  if (!bare.stderr.includes('no listener bound')) {
    throw new Error(
      'router process smoke: bare invocation must report the no-listener skeleton state',
    );
  }

  return {
    implementation: 'rust',
    rust: { spec: rustSpec, invocation: rustInvocation },
    binary: {
      path: rustSpec.rust_binary_path,
      sha256: binarySha256,
      identity: identityLine,
    },
    lifecycle: { code: bare.code },
  };
}

function assertCommandOutcome(outcome, label) {
  if (outcome.error !== null || outcome.signal !== null || outcome.code !== 0) {
    throw new Error(
      `router process smoke: ${label} failed (${outcome.signal ?? outcome.code ?? outcome.error?.code ?? 'spawn'})`,
    );
  }
}

function assertInvocation(actual, expected) {
  if (
    actual.command !== expected.command
    || actual.args.join('\0') !== expected.args.join('\0')
  ) {
    throw new Error(
      `router process smoke: unexpected process invocation ${JSON.stringify(actual)}`,
    );
  }
}

async function sha256File(path) {
  return createHash('sha256').update(await readFile(path)).digest('hex');
}
