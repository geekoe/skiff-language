#!/usr/bin/env node

import { mkdir, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoBuildEnv, cargoTargetDir } from './lib/cargo-target-dir.mjs';
import { runAttachedCommand } from './lib/command-execution.mjs';
import { devRuntimePaths } from './lib/dev-runtime-paths.mjs';
import { readInstanceConfig } from './lib/local-instance-config.mjs';
import { installManagedBinary } from './lib/managed-binary.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const skiffRoot = path.resolve(scriptDir, '..');
const usage = 'usage: node scripts/build-dev-runtime.mjs [--config <path>] [--dev-home <dir>] [--no-refresh]';
const cli = parseCli(process.argv.slice(2));
if (cli.help) {
  console.log(usage);
  process.exit(0);
}
const paths = devRuntimePaths({ devHome: cli.devHome });
const configPath = path.resolve(cli.config ?? path.join(path.dirname(paths.devHome), 'config.yml'));
let refreshConfig = null;
if (!cli.noRefresh) {
  refreshConfig = await readInstanceConfig({ configPath, repoRoot: skiffRoot });
  if (refreshConfig.paths.devHome !== paths.devHome) {
    throw new Error(
      `selected instance devHome ${refreshConfig.paths.devHome} does not match install devHome ${paths.devHome}`,
    );
  }
}
const targetDir = cargoTargetDir(skiffRoot);
const runtimeManifest = path.join(skiffRoot, 'runtime', 'Cargo.toml');
const compilerManifest = path.join(skiffRoot, 'compiler', 'Cargo.toml');
const cargoRuntimeBinary = path.join(
  targetDir,
  'debug',
  process.platform === 'win32' ? 'runtime.exe' : 'runtime',
);
const cargoEcosystemStoreCli = path.join(
  targetDir,
  'debug',
  process.platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler',
);

await mkdir(targetDir, { recursive: true });
await run('cargo', ['build', '--manifest-path', runtimeManifest, '--bin', 'runtime'], skiffRoot, {
  ...cargoBuildEnv(skiffRoot),
  CARGO_TARGET_DIR: targetDir,
});
await run(
  'cargo',
  ['build', '--manifest-path', compilerManifest, '--bin', 'skiff-compiler'],
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
const ecosystemStoreCliBinary = await stat(cargoEcosystemStoreCli);
if (!ecosystemStoreCliBinary.isFile()) {
  throw new Error(`ecosystem store CLI was not produced at ${cargoEcosystemStoreCli}`);
}

await mkdir(paths.runtimeBinDir, { recursive: true });
await installManagedBinary(cargoRuntimeBinary, paths.runtimeBinary);
await installManagedBinary(cargoEcosystemStoreCli, paths.ecosystemStoreCli);

const installed = await stat(paths.runtimeBinary);
if (!installed.isFile()) {
  throw new Error(`runtime binary was not installed at ${paths.runtimeBinary}`);
}
const installedEcosystemStoreCli = await stat(paths.ecosystemStoreCli);
if (!installedEcosystemStoreCli.isFile()) {
  throw new Error(`ecosystem store CLI was not installed at ${paths.ecosystemStoreCli}`);
}

const refresh = cli.noRefresh
  ? {
      action: 'skipped-explicitly',
      activeRuntimeMayBeStale: true,
      recovery: `node scripts/skiff.mjs instance refresh-binaries ${configPath}`,
    }
  : {
      action: 'reconciled',
      configPath: refreshConfig.paths.configPath,
    };
if (!cli.noRefresh) {
  await run(
    process.execPath,
    [path.join(scriptDir, 'skiff.mjs'), 'instance', 'refresh-binaries', configPath],
    skiffRoot,
    process.env,
  );
}

console.log(JSON.stringify({
  devHome: paths.devHome,
  runtimeBinary: paths.runtimeBinary,
  ecosystemStoreCli: paths.ecosystemStoreCli,
  runtimeConfig: paths.runtimeConfig,
  runtimeHome: paths.runtimeHome,
  cargoRuntimeBinary,
  cargoEcosystemStoreCli,
  cargoTargetDir: targetDir,
  refresh,
}, null, 2));

function parseCli(rawArgs) {
  const result = { config: undefined, devHome: undefined, help: false, noRefresh: false };
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
    if (arg === '--config') {
      const value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--config requires a value');
      }
      result.config = value;
      index += 1;
      continue;
    }
    if (arg.startsWith('--config=')) {
      result.config = arg.slice('--config='.length);
      continue;
    }
    if (arg === '--no-refresh') {
      result.noRefresh = true;
      continue;
    }
    if (arg === '--help' || arg === '-h') {
      result.help = true;
      continue;
    }
    throw new Error(`unknown option ${arg}\n${usage}`);
  }
  return result;
}

function run(command, args, cwd, env) {
  return runAttachedCommand(command, args, { cwd, env });
}
