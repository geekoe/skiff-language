#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { chmod, copyFile, mkdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoBuildEnv, cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { devRuntimePaths } from './lib/dev-runtime-paths.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const skiffRoot = path.resolve(scriptDir, '..');
const cli = parseCli(process.argv.slice(2));
const paths = devRuntimePaths({ devHome: cli.devHome });
const targetDir = cargoTargetDir(skiffRoot);
const runtimeManifest = path.join(skiffRoot, 'runtime', 'Cargo.toml');
const identityManifest = path.join(skiffRoot, 'artifact-identity', 'Cargo.toml');
const cargoRuntimeBinary = path.join(
  targetDir,
  'debug',
  process.platform === 'win32' ? 'runtime.exe' : 'runtime',
);
const cargoIdentityCli = path.join(
  targetDir,
  'debug',
  process.platform === 'win32' ? 'skiff-artifact-identity.exe' : 'skiff-artifact-identity',
);

await mkdir(targetDir, { recursive: true });
await run('cargo', ['build', '--manifest-path', runtimeManifest, '--bin', 'runtime'], skiffRoot, {
  ...cargoBuildEnv(skiffRoot),
  CARGO_TARGET_DIR: targetDir,
});
await run(
  'cargo',
  ['build', '--manifest-path', identityManifest, '--bin', 'skiff-artifact-identity'],
  skiffRoot,
  {
    ...cargoBuildEnv(skiffRoot),
    CARGO_TARGET_DIR: targetDir,
  },
);

const binary = await stat(cargoRuntimeBinary);
if (!binary.isFile()) {
  throw new Error(`runtime binary was not produced at ${cargoRuntimeBinary}`);
}
const identityCliBinary = await stat(cargoIdentityCli);
if (!identityCliBinary.isFile()) {
  throw new Error(`artifact identity CLI was not produced at ${cargoIdentityCli}`);
}

await mkdir(paths.runtimeBinDir, { recursive: true });
await copyFile(cargoRuntimeBinary, paths.runtimeBinary);
await copyFile(cargoIdentityCli, paths.identityCli);
if (process.platform !== 'win32') {
  await chmod(paths.runtimeBinary, 0o755);
  await chmod(paths.identityCli, 0o755);
}

const installed = await stat(paths.runtimeBinary);
if (!installed.isFile()) {
  throw new Error(`runtime binary was not installed at ${paths.runtimeBinary}`);
}
const installedIdentityCli = await stat(paths.identityCli);
if (!installedIdentityCli.isFile()) {
  throw new Error(`artifact identity CLI was not installed at ${paths.identityCli}`);
}

console.log(JSON.stringify({
  devHome: paths.devHome,
  runtimeBinary: paths.runtimeBinary,
  identityCli: paths.identityCli,
  runtimeConfig: paths.runtimeConfig,
  runtimeHome: paths.runtimeHome,
  cargoRuntimeBinary,
  cargoIdentityCli,
  cargoTargetDir: targetDir,
}, null, 2));

function parseCli(rawArgs) {
  const result = { devHome: undefined };
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--dev-home') {
      const value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--dev-home requires a value');
      }
      result.devHome = value;
      index += 1;
      continue;
    }
    if (arg.startsWith('--dev-home=')) {
      result.devHome = arg.slice('--dev-home='.length);
      continue;
    }
    throw new Error(`unknown option ${arg}`);
  }
  return result;
}

function run(command, args, cwd, env) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      env,
      stdio: 'inherit',
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} ${args.join(' ')} failed with ${signal || code}`));
    });
  });
}
