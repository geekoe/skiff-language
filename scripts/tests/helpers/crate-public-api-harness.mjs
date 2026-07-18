import { spawn } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
export const PUBLIC_API_CHECKER = join(REPO_ROOT, 'scripts', 'check-crate-public-api.mjs');

export const GATE_POLICY = Object.freeze([
  Object.freeze({
    name: 'skiff-compiler-contract',
    allowedCrates: Object.freeze([
      'skiff-compiler-contract',
      'skiff-artifact-model',
      'skiff-artifact-identity',
      'std',
      'core',
      'alloc',
      'serde',
      'serde_json',
    ]),
    note: 'contract public API exposes only self/artifact-model/artifact-identity/std and approved value crates',
  }),
  Object.freeze({
    name: 'skiff-compiler',
    allowedCrates: Object.freeze([
      'skiff-compiler',
      'skiff-compiler-contract',
      'skiff-compiler-input-model',
      'skiff-compiler-input',
      'skiff-compiler-source',
      'skiff-compiler-emission',
      'skiff-artifact-model',
      'skiff-syntax',
      'std',
      'core',
      'alloc',
      'serde',
      'serde_json',
    ]),
    note: 'package compiler public API exposes only terminal package/contract input-output types and approved value crates',
  }),
]);

export const HELP_ORDER = Object.freeze([
  'skiff-compiler-contract',
  'skiff-compiler',
]);

const fakeCargoSource = String.raw`#!/usr/bin/env node
const { appendFileSync, mkdirSync, readFileSync, writeFileSync } = require('node:fs');
const { join } = require('node:path');

const scenario = JSON.parse(readFileSync(process.env.SKIFF_FAKE_CARGO_SCENARIO, 'utf8'));
const args = process.argv.slice(2);
const attempt = args[0] === '+nightly' ? 'nightly' : 'bootstrap';
const kind = args[0] === 'metadata'
  ? 'metadata'
  : args[0] === '+nightly' && args[1] === '--version'
    ? 'probe'
    : 'rustdoc';
appendFileSync(process.env.SKIFF_FAKE_CARGO_LOG, JSON.stringify({
  args,
  cwd: process.cwd(),
  kind,
  rustcBootstrap: process.env.RUSTC_BOOTSTRAP ?? null,
}) + '\n');

if (kind === 'metadata') {
  if (scenario.metadataFailure) {
    process.stderr.write(scenario.metadataFailure.stderr ?? 'fake metadata failure\n');
    process.exit(scenario.metadataFailure.code ?? 17);
  }
  process.stdout.write(scenario.invalidMetadata ? '{not-json' : JSON.stringify(scenario.metadata));
  process.exit(0);
}

if (kind === 'probe') {
  if (!scenario.nightlyAvailable) {
    process.stderr.write(scenario.probeStderr ?? 'fake nightly unavailable\n');
    process.exit(scenario.probeExitCode ?? 19);
  }
  process.stdout.write('cargo 1.99.0-nightly\n');
  process.exit(0);
}

const crateIndex = args.indexOf('-p');
const crateName = crateIndex >= 0 ? args[crateIndex + 1] : undefined;
const failure = scenario.rustdocFailures?.[crateName]?.[attempt];
if (failure) {
  process.stdout.write(failure.stdout ?? '');
  process.stderr.write(failure.stderr ?? ('fake ' + attempt + ' rustdoc failure for ' + crateName + '\n'));
  process.exit(failure.code ?? 23);
}

if (!scenario.omitRustdoc?.includes(crateName)) {
  const pkg = scenario.metadata.packages.find((entry) => entry.name === crateName);
  const target = pkg?.targets.find((entry) => entry.kind.includes('lib'));
  const stem = target.name.replaceAll('-', '_');
  const docDir = join(scenario.metadata.target_directory, 'doc');
  mkdirSync(docDir, { recursive: true });
  const payload = scenario.invalidRustdoc?.includes(crateName)
    ? '{not-json'
    : JSON.stringify(scenario.rustdocs?.[crateName] ?? scenario.defaultRustdoc);
  writeFileSync(join(docDir, stem + '.json'), payload);
}
process.stdout.write(scenario.rustdocStdout?.[crateName]?.[attempt] ?? '');
process.stderr.write(scenario.rustdocStderr?.[crateName]?.[attempt] ?? '');
`;

export function packageInfo(name) {
  return {
    name,
    targets: [{ kind: ['lib'], name: name.replaceAll('-', '_') }],
  };
}

export function passingRustdoc(crateName = 'fixture-crate') {
  const normalized = crateName.replaceAll('-', '_');
  return {
    root: '0:0',
    index: {
      '0:0': {
        name: normalized,
        visibility: 'public',
        inner: { module: { is_crate: true, items: [] } },
      },
    },
    paths: {
      '0:0': { crate_id: 0, kind: 'module', path: [normalized] },
    },
    external_crates: {},
  };
}

export async function runPublicApiCli(args, overrides = {}) {
  const tempRoot = await mkdtemp(join(tmpdir(), 'skiff-public-api-'));
  const binDir = join(tempRoot, 'bin');
  const targetDirectory = join(tempRoot, 'target');
  const cargoPath = join(binDir, 'cargo');
  const scenarioPath = join(tempRoot, 'scenario.json');
  const logPath = join(tempRoot, 'cargo.jsonl');
  await mkdir(binDir, { recursive: true });
  await writeFile(cargoPath, fakeCargoSource);
  await chmod(cargoPath, 0o755);

  const packageNames = overrides.packageNames ?? GATE_POLICY.map(({ name }) => name);
  const metadata = overrides.metadata ?? {
    packages: packageNames.map(packageInfo),
    target_directory: targetDirectory,
  };
  const defaultRustdoc = overrides.defaultRustdoc ?? passingRustdoc();
  const scenario = {
    metadata,
    nightlyAvailable: overrides.nightlyAvailable ?? true,
    defaultRustdoc,
    ...overrides,
    metadata,
    defaultRustdoc,
  };
  delete scenario.packageNames;
  await writeFile(scenarioPath, JSON.stringify(scenario));
  await writeFile(logPath, '');

  const result = await new Promise((resolveResult, reject) => {
    const child = spawn(process.execPath, [PUBLIC_API_CHECKER, ...args], {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        ...overrides.env,
        PATH: `${binDir}${delimiter}${process.env.PATH ?? ''}`,
        SKIFF_FAKE_CARGO_LOG: logPath,
        SKIFF_FAKE_CARGO_SCENARIO: scenarioPath,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('close', (code, signal) => resolveResult({ code, signal, stderr, stdout }));
  });

  const logText = await readFile(logPath, 'utf8');
  const cargoLog = logText.trim() === ''
    ? []
    : logText.trimEnd().split('\n').map((line) => JSON.parse(line));
  await rm(tempRoot, { force: true, recursive: true });
  return { ...result, cargoLog };
}

export function expectedPassingOutput(records = GATE_POLICY) {
  return records.map(({ name, allowedCrates, note }) => [
    `Public API allow-list for ${name}: ${allowedCrates.join(', ')}`,
    `Policy: ${note}`,
    `Public API check passed for ${name}.`,
  ].join('\n')).join('\n') + '\n';
}

export function cargoKinds(result) {
  return result.cargoLog.map(({ kind }) => kind);
}
