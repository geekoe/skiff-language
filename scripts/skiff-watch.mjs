#!/usr/bin/env node
// `skiff watch` — local-only dev watch.
//
// Reads the generated instance.yml (profile, artifactRoot, compilerBinary)
// plus its own watch directory (watch.yml + watch.json roots).
// `--once` performs a single sync; the default mode watches the roots and
// re-syncs on changes. It replaces the old `skiff dev sync` / `skiff dev watch`.

import { access, readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readDevRegistry, runDevSyncOnce, runDevWatch } from './skiff-dev-sync.mjs';
import { runCompilerAuthoring } from './lib/package-service-authoring.mjs';
import { parseStackYaml } from './lib/stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');
const DEFAULT_RUNTIME_DIR = join(skiffRoot, 'build', 'runtime-stack');

const usage = `usage:
  skiff watch [--once] [--runtime <dir>] --config <watchDir> [--poll-interval-ms <ms>] [--build-only] [--json]`;

try {
  await main(process.argv.slice(2));
} catch (error) {
  console.error(`error: ${error?.message || String(error)}`);
  process.exitCode = 1;
}

async function main(rawArgs) {
  if (rawArgs.length === 0 || rawArgs.includes('-h') || rawArgs.includes('--help')) {
    console.log(usage);
    return;
  }
  const args = parseArgs(rawArgs);
  const runtimeSpec = await loadRuntimeSpec(args.runtimeDir);
  const watchConfig = await loadWatchConfig(args.configDir);
  const compilerRunner = (options) => runCompilerAuthoring({
    ...options,
    compilerBinary: runtimeSpec.compilerBinary,
  });
  const roots = (await readDevRegistry(join(args.configDir, 'watch.json'), {
    allowMissing: true,
  })).roots;
  if (args.once) {
    const result = await runDevSyncOnce({
      roots,
      profile: runtimeSpec.profile,
      artifactRoot: runtimeSpec.artifactRoot,
      buildOnly: watchConfig.buildOnly,
      skiffRoot,
      compilerRunner,
    });
    console.log(args.json ? JSON.stringify(result, null, 2) : 'watch: sync ok');
    return;
  }
  await runDevWatch({
    config: join(args.configDir, 'watch.json'),
    roots: [],
    profile: runtimeSpec.profile,
    artifactRoot: runtimeSpec.artifactRoot,
    buildOnly: watchConfig.buildOnly,
    pollIntervalMs: watchConfig.pollIntervalMs,
    json: args.json,
  }, { compilerRunner });
}

function parseArgs(rawArgs) {
  let runtimeDir = DEFAULT_RUNTIME_DIR;
  let configDir;
  let once = false;
  let pollIntervalMs = 500;
  let buildOnly = false;
  let json = false;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    const valueOf = (name) => {
      const value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${name} requires a value`);
      }
      index += 1;
      return value;
    };
    if (argument === '--runtime') {
      runtimeDir = resolve(valueOf(argument));
    } else if (argument.startsWith('--runtime=')) {
      runtimeDir = resolve(argument.slice('--runtime='.length));
    } else if (argument === '--config') {
      configDir = resolve(valueOf(argument));
    } else if (argument.startsWith('--config=')) {
      configDir = resolve(argument.slice('--config='.length));
    } else if (argument === '--poll-interval-ms') {
      pollIntervalMs = parsePositiveInteger(valueOf(argument), argument);
    } else if (argument === '--once') {
      once = true;
    } else if (argument === '--build-only') {
      buildOnly = true;
    } else if (argument === '--json') {
      json = true;
    } else {
      throw new Error(`unknown watch option ${argument}\n${usage}`);
    }
  }
  if (configDir === undefined) {
    throw new Error('watch requires --config <watchDir>\n' + usage);
  }
  return { runtimeDir, configDir, once, pollIntervalMs, buildOnly, json };
}

async function loadRuntimeSpec(runtimeDir) {
  const file = join(runtimeDir, 'router.yml');
  const source = await readFile(file, 'utf8').catch(() => null);
  if (source === null) {
    throw new Error(
      `router.yml not found at ${file}; run "skiff build router runtime" first`,
    );
  }
  const router = parseStackYaml(source, 'router.yml');
  if (typeof router.profile !== 'string' || router.profile.length === 0) {
    throw new Error('router.yml must declare profile');
  }
  if (typeof router.artifactsPath !== 'string' || router.artifactsPath.length === 0) {
    throw new Error('router.yml must declare artifactsPath');
  }
  const compilerBinary = await resolveCompilerBinary(runtimeDir);
  return {
    profile: router.profile,
    artifactRoot: router.artifactsPath,
    compilerBinary,
  };
}

async function resolveCompilerBinary(runtimeDir) {
  for (const candidate of [
    join(runtimeDir, 'bin', 'skiff-compiler'),
    join(skiffRoot, 'build', 'bin', 'skiff-compiler'),
  ]) {
    try {
      await access(candidate);
      return candidate;
    } catch {
      // try next candidate
    }
  }
  throw new Error('skiff-compiler binary not found; run "skiff build compiler" first');
}

async function loadWatchConfig(configDir) {
  const file = join(configDir, 'watch.yml');
  const source = await readFile(file, 'utf8').catch(() => null);
  const defaults = { packageDirs: [], pollIntervalMs: 500, buildOnly: false };
  if (source === null) {
    return defaults;
  }
  const value = parseStackYaml(source, 'watch.yml');
  if (!Array.isArray(value.packageDirs ?? [])) {
    throw new Error('watch.yml packageDirs must be an array');
  }
  const pollIntervalMs = value.pollIntervalMs ?? defaults.pollIntervalMs;
  if (!Number.isSafeInteger(pollIntervalMs) || pollIntervalMs <= 0) {
    throw new Error('watch.yml pollIntervalMs must be a positive integer');
  }
  return {
    packageDirs: value.packageDirs ?? [],
    pollIntervalMs,
    buildOnly: value.buildOnly === true,
  };
}

function parsePositiveInteger(value, label) {
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number <= 0) {
    throw new Error(`${label} must be a positive integer`);
  }
  return number;
}
