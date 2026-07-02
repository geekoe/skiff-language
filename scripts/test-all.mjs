import { spawn } from 'node:child_process';
import { existsSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const BUILD_UNIT_ORDER = [
  'artifact-model',
  'compiler',
  'runtime',
  'router',
  'telemetry',
];

const DEV_SUPPORT_UNIT_ORDER = [
  'test-runner',
  'operation-abi-identity',
  'scripts',
  'vscode',
];

const SCRIPT_SYNTAX_CHECK_FILES = [
  'scripts/skiff.mjs',
  'scripts/test-all.mjs',
  'scripts/check-compiler-boundaries.mjs',
  'scripts/check-operation-abi-identity-single-source.mjs',
  'scripts/check-compiler-crate-dag.mjs',
  'scripts/check-runtime-crate-dag.mjs',
  'scripts/check-runtime-eval-error-boundary.mjs',
  'scripts/check-crate-public-api.mjs',
  'scripts/check-skiff-source-layout.mjs',
  'scripts/check-package-store-discovery.mjs',
  'scripts/build-runtime-stack.mjs',
  'scripts/deploy-runtime-stack.mjs',
  'scripts/lib/cargo-target-dir.mjs',
  'scripts/lib/dev-runtime-paths.mjs',
  'scripts/lib/source-key.mjs',
];

const args = parseArgs(process.argv.slice(2));
if (args.help) {
  printUsage();
  process.exit(0);
}
const selectedUnits = selectedTestUnits(args.only || 'all');

for (const unit of selectedUnits) {
  await runUnit(unit);
}

console.log('\nAll selected Skiff test units passed.');

async function runUnit(unit) {
  const phases = unitPhases(unit);
  console.log(`\n## ${unitLabel(unit)}`);
  for (const phase of phases) {
    await runPhase(phase);
  }
}

function unitPhases(unit) {
  switch (unit.name) {
    case 'artifact-model':
      return [
        {
          name: 'artifact-model:cargo-test',
          command: 'cargo',
          args: ['test', '--manifest-path', 'artifact-model/Cargo.toml', '--no-fail-fast'],
        },
      ];
    case 'compiler':
      return [
        {
          name: 'compiler:cargo-test',
          command: 'cargo',
          args: ['test', '--manifest-path', 'compiler/Cargo.toml', '--no-fail-fast'],
        },
      ];
    case 'runtime':
      return [
        {
          name: 'runtime:cargo-test',
          command: 'cargo',
          args: ['test', '--manifest-path', 'runtime/Cargo.toml', '--no-fail-fast'],
        },
      ];
    case 'router':
      return [
        {
          name: 'router:type-check',
          command: 'pnpm',
          args: ['run', 'type-check'],
          cwd: join(root, 'router'),
        },
        {
          name: 'router:test',
          command: 'pnpm',
          args: ['test'],
          cwd: join(root, 'router'),
        },
      ];
    case 'telemetry':
      return [
        {
          name: 'telemetry:type-check',
          command: 'pnpm',
          args: ['run', 'type-check'],
          cwd: join(root, 'telemetry'),
        },
        {
          name: 'telemetry:test',
          command: 'pnpm',
          args: ['test'],
          cwd: join(root, 'telemetry'),
        },
      ];
    case 'test-runner':
      return [
        {
          name: 'test-runner:cargo-test',
          command: 'cargo',
          args: ['test', '--manifest-path', 'test-runner/Cargo.toml', '--no-fail-fast'],
        },
      ];
    case 'operation-abi-identity':
      return [
        {
          name: 'operation-abi-identity:self-test',
          command: 'node',
          args: ['scripts/check-operation-abi-identity-single-source.mjs', '--self-test'],
        },
        {
          name: 'operation-abi-identity:check',
          command: 'node',
          args: ['scripts/check-operation-abi-identity-single-source.mjs'],
        },
      ];
    case 'scripts':
      return [
        ...SCRIPT_SYNTAX_CHECK_FILES.map((file) => ({
          name: `scripts:syntax:${file}`,
          command: 'node',
          args: ['--check', file],
        })),
        {
          name: 'scripts:runtime-eval-error-boundary',
          command: 'node',
          args: ['scripts/check-runtime-eval-error-boundary.mjs'],
        },
        {
          name: 'scripts:operation-abi-identity-single-source-self-test',
          command: 'node',
          args: ['scripts/check-operation-abi-identity-single-source.mjs', '--self-test'],
        },
        {
          name: 'scripts:operation-abi-identity-single-source',
          command: 'node',
          args: ['scripts/check-operation-abi-identity-single-source.mjs'],
        },
        {
          name: 'scripts:compiler-crate-dag-self-test',
          command: 'node',
          args: ['scripts/check-compiler-crate-dag.mjs', '--self-test'],
        },
        {
          name: 'scripts:compiler-crate-dag',
          command: 'node',
          args: ['scripts/check-compiler-crate-dag.mjs'],
        },
        {
          name: 'scripts:runtime-crate-dag-self-test',
          command: 'node',
          args: ['scripts/check-runtime-crate-dag.mjs', '--self-test'],
        },
        {
          name: 'scripts:runtime-crate-dag',
          command: 'node',
          args: ['scripts/check-runtime-crate-dag.mjs'],
        },
        {
          name: 'scripts:crate-public-api-self-test',
          command: 'node',
          args: ['scripts/check-crate-public-api.mjs', '--self-test'],
        },
        {
          name: 'scripts:package-store-discovery',
          command: 'node',
          args: ['scripts/check-package-store-discovery.mjs'],
        },
      ];
    case 'vscode':
      return [
        {
          name: 'vscode:grammar',
          command: 'pnpm',
          args: ['run', 'test:grammar'],
          cwd: join(root, 'vscode'),
        },
      ];
    case 'runtime-live':
      return runtimeLivePhases();
    default:
      throw new Error(`unknown test unit ${unit.name}`);
  }
}

function runPhase(phase) {
  if (phase.skip) {
    console.log(`SKIP ${phase.name}: ${phase.skip}`);
    return Promise.resolve();
  }

  const cwd = phase.cwd ?? root;
  const label = relative(root, cwd) || '.';
  console.log(`\n==> ${phase.name} (${label})`);
  console.log(`$ ${[phase.command, ...phase.args].join(' ')}`);

  return new Promise((resolve, reject) => {
    const child = spawn(phase.command, phase.args, {
      cwd,
      stdio: 'inherit',
      env: process.env,
    });

    child.on('error', reject);
    child.on('exit', (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      const status = signal ?? code;
      reject(new Error(`${phase.name} failed with ${status}`));
    });
  });
}

function selectedTestUnits(rawOnly) {
  const values = rawOnly.split(',').map((value) => value.trim()).filter(Boolean);
  const selected = [];
  for (const value of values) {
    selected.push(...expandTestSelector(value));
  }
  return uniqueUnits(selected);
}

function expandTestSelector(rawOnly) {
  switch (rawOnly) {
    case 'all':
      return [...BUILD_UNIT_ORDER, ...DEV_SUPPORT_UNIT_ORDER].map((name) => unit(name));
    case 'build':
    case 'runtime-stack':
      return BUILD_UNIT_ORDER.map((name) => unit(name));
    case 'dev-support':
    case 'support':
      return DEV_SUPPORT_UNIT_ORDER.map((name) => unit(name));
    case 'rs':
      return ['artifact-model', 'compiler', 'runtime'].map((name) => unit(name));
    case 'ts':
      return ['router', 'telemetry'].map((name) => unit(name));
    case 'skiff':
    case 'std':
      throw new Error(`${rawOnly} tests moved out of language; run them from ../skiff-packages/`);
    case 'artifact-model':
    case 'compiler':
    case 'runtime':
    case 'router':
    case 'telemetry':
    case 'test-runner':
    case 'operation-abi-identity':
    case 'scripts':
    case 'vscode':
    case 'runtime-live':
      return [unit(rawOnly)];
    default:
      throw new Error(
        `invalid --only ${rawOnly}; expected all, build, support, rs, ts, artifact-model, compiler, runtime, router, telemetry, test-runner, operation-abi-identity, scripts, vscode, or runtime-live`,
      );
  }
}

function uniqueUnits(units) {
  const seen = new Set();
  const result = [];

  for (const candidate of units) {
    const key = `${candidate.name}:${candidate.selector || ''}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    result.push(candidate);
  }

  return result;
}

function unit(name, selector = null) {
  return { name, selector };
}

function unitLabel(unit) {
  if (unit.selector) {
    return `test unit: ${unit.name} (${unit.selector})`;
  }
  return `test unit: ${unit.name}`;
}

function parseArgs(rawArgs) {
  const parsed = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (arg === '-h' || arg === '--help') {
      parsed.help = true;
      continue;
    }
    if (arg === '--only') {
      const value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--only requires a value');
      }
      parsed.only = value;
      index += 1;
      continue;
    }
    if (arg.startsWith('--only=')) {
      parsed.only = arg.slice('--only='.length);
      if (!parsed.only) {
        throw new Error('--only requires a value');
      }
      continue;
    }
    if (arg === '--runtime-live-config') {
      const value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--runtime-live-config requires a path');
      }
      parsed.runtimeLiveConfig = value;
      index += 1;
      continue;
    }
    if (arg.startsWith('--runtime-live-config=')) {
      parsed.runtimeLiveConfig = arg.slice('--runtime-live-config='.length);
      if (!parsed.runtimeLiveConfig) {
        throw new Error('--runtime-live-config requires a path');
      }
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }
  return parsed;
}

function runtimeLivePhases() {
  const rawConfigPath = args.runtimeLiveConfig || process.env.SKIFF_RUNTIME_LIVE_CONFIG;
  if (!rawConfigPath) {
    return [
      {
        name: 'runtime-live:config',
        skip: 'set SKIFF_RUNTIME_LIVE_CONFIG or pass --runtime-live-config <path> to run live runtime fixtures',
      },
    ];
  }
  const configPath = resolve(process.cwd(), rawConfigPath);
  if (!existsSync(configPath)) {
    throw new Error(`runtime-live config path does not exist: ${configPath}`);
  }

  const files = runtimeLiveTestFiles();
  if (files.length === 0) {
    throw new Error('runtime-live selector found no *.live.test.skiff fixtures under runtime/live-tests');
  }

  return files.map((file) => ({
    name: `runtime-live:${relative(root, file)}`,
    command: 'cargo',
    args: [
      'run',
      '--manifest-path',
      'test-runner/Cargo.toml',
      '--',
      file,
      '--live',
      '--allow-network',
      '--config',
      configPath,
      ...runtimeLivePackageArgs(),
    ],
  }));
}

function runtimeLivePackageArgs() {
  const packageStore = join(root, 'runtime', 'live-tests', 'package-store');
  if (!existsSync(packageStore)) {
    return [];
  }
  return ['--packages-dir', packageStore];
}

function runtimeLiveTestFiles() {
  const liveRoot = join(root, 'runtime', 'live-tests');
  const files = [];
  collectRuntimeLiveTestFiles(liveRoot, files);
  return files.sort();
}

function collectRuntimeLiveTestFiles(dir, files) {
  if (!existsSync(dir)) {
    return;
  }
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      collectRuntimeLiveTestFiles(path, files);
    } else if (entry.endsWith('.live.test.skiff')) {
      files.push(path);
    }
  }
}

function printUsage() {
  console.log(`usage: node scripts/test-all.mjs [--only <selector>] [--runtime-live-config <path>]

selectors:
  all             build + dev-support; excludes runtime-live
  build           artifact-model, compiler, runtime, router, telemetry
  support         test-runner, operation-abi-identity, scripts, vscode
  rs              artifact-model, compiler, runtime
  ts              router, telemetry
  artifact-model  compiler  runtime  router  telemetry  test-runner  operation-abi-identity  scripts  vscode
  runtime-live    explicit live runtime fixtures; requires --runtime-live-config or SKIFF_RUNTIME_LIVE_CONFIG
`);
}
