#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { chmod, copyFile, mkdir, rename, rm, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  captureCheckedCommand,
  runAttachedCommand,
} from './lib/command-execution.mjs';

const COMPONENT_SPECS = {
  router: {
    crate: 'skiff-router',
    bin: 'skiff-router',
    manifest: 'router/Cargo.toml',
  },
  runtime: {
    crate: 'runtime',
    bin: 'runtime',
    manifest: 'runtime/Cargo.toml',
  },
  compiler: {
    crate: 'skiff-compiler',
    bin: 'skiff-compiler',
    manifest: 'compiler/Cargo.toml',
  },
};

const USAGE = [
  'usage: skiff build <component...> [--profile debug|release]',
  '  components: router, runtime, compiler, all',
].join('\n');

const skiffRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function componentSpec(name) {
  const spec = COMPONENT_SPECS[name];
  if (spec === undefined) {
    throw new Error(`unknown component ${name}; expected one of: router, runtime, compiler, all`);
  }
  return spec;
}

export function expandComponents(names) {
  const expanded = [];
  for (const name of names) {
    if (name === 'all') {
      expanded.push('router', 'runtime', 'compiler');
      continue;
    }
    componentSpec(name);
    expanded.push(name);
  }
  return [...new Set(expanded)].map(componentSpec);
}

export function parseBuildArgs(rawArgs) {
  const components = [];
  let profile = 'debug';
  let help = false;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--profile') {
      const value = rawArgs[index + 1];
      if (typeof value !== 'string' || value.length === 0 || value.startsWith('-')) {
        throw new Error('--profile requires a value (debug or release)');
      }
      profile = value;
      index += 1;
      continue;
    }
    if (arg === '--help' || arg === '-h') {
      help = true;
      continue;
    }
    if (arg.startsWith('-')) {
      throw new Error(`unknown argument ${arg}`);
    }
    components.push(arg);
  }
  return { components, profile, help };
}

export function parseTargetDirectory(metadataJson) {
  let parsed;
  try {
    parsed = JSON.parse(metadataJson);
  } catch (error) {
    throw new Error(`invalid cargo metadata output: ${error.message}`);
  }
  if (typeof parsed?.target_directory !== 'string' || parsed.target_directory.length === 0) {
    throw new Error('cargo metadata output missing target_directory');
  }
  return parsed.target_directory;
}

export async function resolveTargetDirectory({
  skiffRoot: root,
  manifest,
  env = process.env,
  runCommand = runCargoMetadata,
}) {
  if (env.CARGO_TARGET_DIR) {
    return resolve(env.CARGO_TARGET_DIR);
  }
  const outcome = await runCommand(
    'cargo',
    ['metadata', '--format-version', '1', '--manifest-path', join(root, manifest)],
    { cwd: root, env },
  );
  return resolve(parseTargetDirectory(outcome.stdout));
}

export async function sha256Hex(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) {
    hash.update(chunk);
  }
  return hash.digest('hex');
}

export async function copyBinary(source, destination) {
  // Write to a fresh temp file then rename: in-place overwrite of a signed
  // binary that a running process still maps poisons subsequent execs on
  // macOS (Killed: 9 until the stale mapping is gone). Atomic rename gives
  // every exec a fresh inode with a valid signature.
  const temporary = `${destination}.tmp-${process.pid}-${Date.now()}`;
  try {
    await copyFile(source, temporary);
    await chmod(temporary, 0o755);
    await rename(temporary, destination);
  } catch (error) {
    await rm(temporary, { force: true }).catch(() => {});
    throw error;
  }
}

export async function installBinary({ source, destination, hashFile }) {
  await mkdir(dirname(destination), { recursive: true });
  await copyBinary(source, destination);
  const sha256 = await sha256Hex(destination);
  await writeFile(hashFile, `${sha256} ${basename(destination)}`);
  return { destination, hashFile, sha256 };
}

export async function buildComponent({
  spec,
  skiffRoot: root,
  profile,
  targetDir,
  runCommand = runAttachedCommand,
}) {
  const args = ['build', '--manifest-path', spec.manifest, '--bin', spec.bin];
  if (profile === 'release') {
    args.push('--release');
  }
  await runCommand('cargo', args, { cwd: root, env: process.env });
  const profileDir = profile === 'release' ? 'release' : 'debug';
  const source = join(targetDir, profileDir, spec.bin);
  const destination = join(root, 'build', 'bin', spec.bin);
  const installed = await installBinary({
    source,
    destination,
    hashFile: `${destination}.sha256`,
  });
  return { spec, ...installed };
}

export async function runBuild({
  skiffRoot: root,
  components,
  profile,
  env = process.env,
  runCommand = runAttachedCommand,
}) {
  if (profile !== 'debug' && profile !== 'release') {
    throw new Error(`build profile must be "debug" or "release"; got ${profile}`);
  }
  const specs = expandComponents(components);
  if (specs.length === 0) {
    throw new Error('no components given; expected router, runtime, compiler, or all');
  }
  const targetDir = await resolveTargetDirectory({
    skiffRoot: root,
    manifest: specs[0].manifest,
    env,
  });
  const results = [];
  for (const spec of specs) {
    results.push(await buildComponent({ spec, skiffRoot: root, profile, targetDir, runCommand }));
  }
  return results;
}

async function runCargoMetadata(command, args, options) {
  const outcome = await captureCheckedCommand(command, args, options);
  return { stdout: outcome.stdout };
}

async function main() {
  const parsed = parseBuildArgs(process.argv.slice(2));
  if (parsed.help) {
    console.log(USAGE);
    return;
  }
  const results = await runBuild({
    skiffRoot,
    components: parsed.components,
    profile: parsed.profile,
  });
  for (const result of results) {
    console.log(`built ${result.spec.bin} -> ${result.destination}`);
  }
}

if (process.argv[1] !== undefined && pathToFileURL(process.argv[1]).href === import.meta.url) {
  await main().catch((error) => {
    console.error(`skiff build: ${error.message}`);
    process.exitCode = 1;
  });
}
