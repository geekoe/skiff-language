#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { chmod, copyFile, mkdir, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  readJsonIfExists,
  sha256File,
  sourceKeyFromInputs,
  writeJsonAtomic,
} from './lib/source-key.mjs';
import { cargoBuildEnv, cargoTargetDir } from './lib/cargo-target-dir.mjs';

const DEFAULT_TARGET = 'x86_64-unknown-linux-gnu';
const DEFAULT_ZIG_DIR = path.join(os.homedir(), '.cache/skiff-tools/zig-aarch64-macos-0.15.2');
const BUILD_ENTRY_SOURCE_INPUTS = [
  'Cargo.toml',
  'Cargo.lock',
  '.cargo/config.toml',
  'scripts/build-runtime-stack.mjs',
  'scripts/lib/cargo-target-dir.mjs',
  'scripts/lib/source-key.mjs',
];

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const skiffRoot = path.resolve(scriptDir, '..');
const rustTargetDir = cargoTargetDir(skiffRoot);

const args = parseArgs(process.argv.slice(2));
const target = args.target || DEFAULT_TARGET;
const buildRoot = path.resolve(args.buildRoot || path.join(skiffRoot, 'build', 'runtime-stack'));
const manifestPath = path.join(buildRoot, 'manifest.json');
const previousManifest = await readJsonIfExists(manifestPath);
const selectedUnits = await selectedBuildUnits(args.only || 'all');
const units = { ...(previousManifest?.units || {}) };

for (const unitName of selectedUnits) {
  units[unitName] = await buildUnit(unitName, previousManifest?.units?.[unitName]);
}

await writeJsonAtomic(manifestPath, {
  schemaVersion: 'skiff-runtime-stack-build-v1',
  target,
  commit: await currentCommit(),
  generatedAt: new Date().toISOString(),
  units,
});

console.log(JSON.stringify({
  manifest: manifestPath,
  target,
  units: selectedUnits,
}, null, 2));

async function buildUnit(unitName, previousUnit) {
  const spec = await unitSpec(unitName);
  const sourceSnapshot = await sourceKeyFromInputs({
    repoRoot: skiffRoot,
    component: unitName,
    inputs: spec.inputs,
    extra: spec.extra,
  });

  if (!args.force && await isReusable(previousUnit, sourceSnapshot, spec)) {
    console.log(`[build-runtime-stack] ${unitName} unchanged; using cached result`);
    return previousUnit;
  }

  console.log(`\n==> ${unitName}`);
  for (const phase of spec.phases) {
    await runPhase(phase);
  }

  const artifacts = await materializeOutputs(spec, sourceSnapshot);
  return {
    name: unitName,
    kind: spec.kind,
    commit: sourceSnapshot.commit,
    inputs: sourceSnapshot.inputs,
    sourceHash: sourceSnapshot.sourceHash,
    sourceKey: sourceSnapshot.sourceKey,
    status: spec.kind === 'ts' ? 'verified' : 'built',
    target: spec.kind === 'rs' ? target : undefined,
    artifacts,
    verifiedAt: new Date().toISOString(),
  };
}

async function unitSpec(unitName) {
  switch (unitName) {
    case 'artifact-model':
      return rsVerificationUnit({
        unitName,
        manifest: 'artifact-model/Cargo.toml',
        inputs: ['artifact-model'],
      });
    case 'compiler':
      return rsUnit({
        unitName,
        manifest: 'compiler/Cargo.toml',
        cargoBin: 'skiff-compiler',
        outputName: 'skiff-compiler',
        inputs: ['compiler', 'artifact-identity', 'artifact-model', 'prelude', 'std'],
      });
    case 'runtime':
      return rsUnit({
        unitName,
        manifest: 'runtime/Cargo.toml',
        cargoBin: 'runtime',
        outputName: 'skiff-runtime',
        inputs: ['runtime', 'artifact-identity', 'artifact-model'],
      });
    case 'artifact-identity':
      return rsUnit({
        unitName,
        manifest: 'artifact-identity/Cargo.toml',
        cargoBin: 'skiff-artifact-identity',
        outputName: 'skiff-artifact-identity',
        inputs: ['artifact-identity', 'artifact-model'],
      });
    case 'router':
      return tsUnit(unitName, 'router');
    case 'telemetry':
      return tsUnit(unitName, 'telemetry');
    default:
      throw new Error(`unknown build unit ${unitName}`);
  }
}

function rsUnit({ unitName, manifest, cargoBin, outputName, inputs, testArgs }) {
  const manifestDir = path.dirname(manifest);
  return {
    kind: 'rs',
    unitName,
    inputs: withBuildEntryInputs(inputs),
    phases: [
      {
        name: `${unitName}:test`,
        command: 'cargo',
        args: testArgs || ['test', '--manifest-path', manifest, '--no-fail-fast'],
      },
      {
        name: `${unitName}:linux-build`,
        command: 'cargo',
        args: ['zigbuild', '--manifest-path', manifest, '--release', '--target', target, '--bin', cargoBin],
        env: buildEnv(),
      },
    ],
    outputs: [
      {
        kind: 'binary',
        source: path.join(rustTargetDir, target, 'release', cargoBin),
        path: path.join(buildRoot, 'bin', outputName),
      },
    ],
  };
}

function rsVerificationUnit({ unitName, manifest, inputs, testArgs }) {
  return {
    kind: 'rs',
    unitName,
    inputs: withBuildEntryInputs(inputs),
    phases: [
      {
        name: `${unitName}:test`,
        command: 'cargo',
        args: testArgs || ['test', '--manifest-path', manifest, '--no-fail-fast'],
      },
    ],
    outputs: [
      {
        kind: 'verification',
        path: path.join(buildRoot, 'verified', `${unitName}.json`),
      },
    ],
  };
}

function tsUnit(unitName, directory) {
  return {
    kind: 'ts',
    unitName,
    inputs: withBuildEntryInputs([directory]),
    phases: [
      {
        name: `${unitName}:type-check`,
        command: 'pnpm',
        args: ['run', 'type-check'],
        cwd: path.join(skiffRoot, directory),
      },
      {
        name: `${unitName}:test`,
        command: 'pnpm',
        args: ['test'],
        cwd: path.join(skiffRoot, directory),
      },
    ],
    outputs: [
      {
        kind: 'verification',
        path: path.join(buildRoot, 'verified', `${unitName}.json`),
      },
    ],
  };
}

function withBuildEntryInputs(inputs) {
  return [
    ...inputs,
    ...BUILD_ENTRY_SOURCE_INPUTS,
  ];
}

async function materializeOutputs(spec, sourceSnapshot) {
  const artifacts = [];
  for (const output of spec.outputs) {
    if (output.kind === 'binary') {
      await mkdir(path.dirname(output.path), { recursive: true });
      await copyFile(output.source, output.path);
      await chmod(output.path, 0o755);
      artifacts.push(await artifactRecord(output, output.path));
      continue;
    }
    if (output.kind === 'verification') {
      await writeJsonAtomic(output.path, {
        schemaVersion: 'skiff-runtime-stack-verification-v1',
        unit: spec.unitName,
        commit: sourceSnapshot.commit,
        sourceKey: sourceSnapshot.sourceKey,
        verifiedAt: new Date().toISOString(),
      });
      artifacts.push(await artifactRecord(output, output.path));
      continue;
    }
  }
  return artifacts;
}

async function artifactRecord(output, file) {
  return {
    kind: output.kind,
    path: toPosix(path.relative(skiffRoot, file)),
    sha256: await sha256File(file),
  };
}

async function isReusable(previousUnit, sourceSnapshot, spec) {
  if (!previousUnit) {
    return false;
  }
  if (previousUnit.commit !== sourceSnapshot.commit || previousUnit.sourceKey !== sourceSnapshot.sourceKey) {
    return false;
  }
  if (spec.kind === 'rs' && previousUnit.target !== target) {
    return false;
  }
  for (const output of spec.outputs) {
    if (!await isFile(output.path)) {
      return false;
    }
  }
  return true;
}

async function runPhase(phase) {
  if (phase.message) {
    console.log(phase.message);
    return;
  }
  for (const directory of phase.mkdirs || []) {
    await mkdir(directory, { recursive: true });
  }
  console.log(`$ ${[phase.command, ...phase.args].join(' ')}`);
  await run(phase.command, phase.args, phase.cwd || skiffRoot, phase.env);
}

function run(command, commandArgs, cwd, env = process.env) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, commandArgs, {
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
      reject(new Error(`${command} ${commandArgs.join(' ')} failed with ${signal || code}`));
    });
  });
}

async function selectedBuildUnits(rawOnly) {
  const values = rawOnly.split(',').map((value) => value.trim()).filter(Boolean);
  const selected = [];
  for (const value of values) {
    selected.push(...await expandBuildSelector(value));
  }
  return [...new Set(selected)];
}

async function expandBuildSelector(rawOnly) {
  switch (rawOnly) {
    case 'all':
      return ['artifact-model', 'artifact-identity', 'compiler', 'runtime', 'router', 'telemetry'];
    case 'rs':
      return ['artifact-model', 'artifact-identity', 'compiler', 'runtime'];
    case 'ts':
      return ['router', 'telemetry'];
    case 'artifact-model':
    case 'artifact-identity':
    case 'compiler':
    case 'runtime':
    case 'router':
    case 'telemetry':
      return [rawOnly];
    default:
      throw new Error(
        `invalid --only ${rawOnly}; expected all, rs, ts, artifact-model, artifact-identity, compiler, runtime, router, or telemetry`,
      );
  }
}

async function currentCommit() {
  return (await capture('git', ['rev-parse', 'HEAD'], skiffRoot)).trim();
}

function capture(command, commandArgs, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, commandArgs, {
      cwd,
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on('data', (chunk) => stdout.push(chunk));
    child.stderr.on('data', (chunk) => stderr.push(chunk));
    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolve(Buffer.concat(stdout).toString('utf8'));
        return;
      }
      reject(new Error(`${command} ${commandArgs.join(' ')} failed with ${signal || code}: ${Buffer.concat(stderr).toString('utf8')}`));
    });
  });
}

async function isFile(file) {
  try {
    return (await stat(file)).isFile();
  } catch (error) {
    if (error.code === 'ENOENT') {
      return false;
    }
    throw error;
  }
}

function buildEnv() {
  const zigDir = args.zigDir || DEFAULT_ZIG_DIR;
  return {
    ...cargoBuildEnv(skiffRoot),
    PATH: `${zigDir}:${process.env.PATH || ''}`,
  };
}

function toPosix(value) {
  return value.split(path.sep).join('/');
}

function parseArgs(rawArgs) {
  const parsed = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '--force') {
      parsed.force = true;
      continue;
    }
    const key = optionKey(arg);
    if (!key) {
      throw new Error(`unknown argument ${arg}`);
    }
    const value = rawArgs[index + 1];
    if (!value || value.startsWith('--')) {
      throw new Error(`${arg} requires a value`);
    }
    parsed[key] = value;
    index += 1;
  }
  return parsed;
}

function optionKey(arg) {
  switch (arg) {
    case '--only':
      return 'only';
    case '--build-root':
      return 'buildRoot';
    case '--target':
      return 'target';
    case '--zig-dir':
      return 'zigDir';
    default:
      return null;
  }
}
