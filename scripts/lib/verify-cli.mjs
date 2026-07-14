import { PUBLIC_SELECTORS } from './verify-plan.mjs';

export function parseVerifyArgs(argv) {
  const options = {
    help: false,
    list: false,
    selectors: [],
    runtimeLiveConfig: undefined,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--') {
      continue;
    }
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--list' || arg === '--dry-run') {
      options.list = true;
      continue;
    }
    if (arg === '--only') {
      rejectRepeatedOnly(options);
      const value = requiredValue(argv, index, '--only');
      options.selectors.push(...splitSelectors(value));
      index += 1;
      continue;
    }
    if (arg.startsWith('--only=')) {
      rejectRepeatedOnly(options);
      options.selectors.push(...splitSelectors(arg.slice('--only='.length)));
      continue;
    }
    if (arg === '--runtime-live-config') {
      options.runtimeLiveConfig = requiredValue(argv, index, '--runtime-live-config');
      index += 1;
      continue;
    }
    if (arg.startsWith('--runtime-live-config=')) {
      options.runtimeLiveConfig = requiredInlineValue(arg, '--runtime-live-config');
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }

  if (options.selectors.length === 0) {
    options.selectors.push('verify');
  }
  return options;
}

export function printVerifyUsage() {
  console.log(`usage: node scripts/verify.mjs [--only <selectors>] [--list]

Default: complete non-live repository verification (rust + node + checks).
Execution is fail-fast; use --list to audit every selected phase before running.

selectors:
  verify                       complete non-live repository verification
  node                         Node/TypeScript plan; no Rust workspace test phase
  rust                         cargo test --workspace --no-fail-fast
  router  telemetry  scripts  scripts-syntax  scripts-dev-sync  vscode  checks  type-check
  compiler-boundaries          focused compiler source-boundary check
  runtime-live                 explicit live fixtures; requires a runtime config
  db-encrypted-storage-live    explicit managed Mongo/runtime/keyring live check

options:
  --only <a,b>                 select one or more groups; may be specified once
  --list, --dry-run            print the expanded plan without executing it
  --runtime-live-config <path> runtime-live config, relative to the repository root
  -h, --help                   show this help

check-loop-risk-health remains a manual command because it requires endpoint/runtime arguments.
The checks selector includes compiler boundaries plus hermetic and actual configured public API
checks; rustdoc falls back to the current toolchain when nightly is unavailable.`);
}

function splitSelectors(value) {
  const selectors = value.split(',').map((entry) => entry.trim()).filter(Boolean);
  if (selectors.length === 0) {
    throw new Error('--only requires a selector');
  }
  for (const selector of selectors) {
    if (!PUBLIC_SELECTORS.includes(selector)) {
      throw new Error(
        `invalid selector ${selector}; expected one of ${PUBLIC_SELECTORS.join(', ')}`,
      );
    }
  }
  return selectors;
}

function rejectRepeatedOnly(options) {
  if (options.selectors.length > 0) {
    throw new Error('--only may be specified only once');
  }
}

function requiredValue(argv, index, option) {
  const value = argv[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${option} requires a value`);
  }
  return value;
}

function requiredInlineValue(arg, option) {
  const value = arg.slice(`${option}=`.length);
  if (!value) {
    throw new Error(`${option} requires a value`);
  }
  return value;
}
