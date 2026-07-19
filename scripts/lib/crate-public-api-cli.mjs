import { runCratePublicApiGate } from './crate-public-api-gate.mjs';
import { MANAGED_CRATE_HELP_NAMES } from './crate-public-api-policy.mjs';
import { runCratePublicApiSelfTest } from './crate-public-api-self-test.mjs';

export async function runCratePublicApiCli({
  argv,
  env = process.env,
  gateDependencies,
  root,
  stderr = process.stderr,
  stdout = process.stdout,
}) {
  const options = parseCratePublicApiArgs(argv);

  if (options.help) {
    stdout.write(`${renderCratePublicApiUsage()}\n`);
    return 0;
  }

  if (options.selfTest) {
    runCratePublicApiSelfTest({ stdout });
    return 0;
  }

  if (!options.crateName && !options.allConfigured) {
    throw new Error('missing crate name; run with --help for usage');
  }

  const outcome = await runCratePublicApiGate({
    dependencies: gateDependencies,
    env,
    options,
    report: (event) => reportGateEvent(event, { stderr, stdout }),
    root,
  });
  return outcome.exitCode;
}

export function parseCratePublicApiArgs(argv) {
  const options = {
    crateName: undefined,
    extraAllowedCrates: [],
    allConfigured: false,
    help: false,
    selfTest: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--self-test' || arg === '--test') {
      options.selfTest = true;
      continue;
    }
    if (arg === '--all-configured') {
      if (options.allConfigured) {
        throw new Error('--all-configured may be specified only once');
      }
      options.allConfigured = true;
      continue;
    }
    if (arg === '--crate') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--crate requires a crate name');
      }
      if (options.crateName) {
        throw new Error(`crate name was specified more than once: ${value}`);
      }
      options.crateName = value;
      index += 1;
      continue;
    }
    if (arg.startsWith('--crate=')) {
      const value = arg.slice('--crate='.length);
      if (!value) {
        throw new Error('--crate requires a crate name');
      }
      if (options.crateName) {
        throw new Error(`crate name was specified more than once: ${value}`);
      }
      options.crateName = value;
      continue;
    }
    if (arg === '--allow-crate' || arg === '--allow') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${arg} requires a crate name`);
      }
      options.extraAllowedCrates.push(value);
      index += 1;
      continue;
    }
    if (arg.startsWith('--allow-crate=')) {
      options.extraAllowedCrates.push(arg.slice('--allow-crate='.length));
      continue;
    }
    if (arg.startsWith('--allow=')) {
      options.extraAllowedCrates.push(arg.slice('--allow='.length));
      continue;
    }
    if (arg === '--allow-list') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--allow-list requires a comma-separated crate list');
      }
      options.extraAllowedCrates.push(...splitCrateList(value));
      index += 1;
      continue;
    }
    if (arg.startsWith('--allow-list=')) {
      options.extraAllowedCrates.push(...splitCrateList(arg.slice('--allow-list='.length)));
      continue;
    }
    if (arg.startsWith('--')) {
      throw new Error(`unknown option: ${arg}`);
    }
    if (options.crateName) {
      throw new Error(`unexpected extra crate name: ${arg}`);
    }
    options.crateName = arg;
  }

  if (options.allConfigured && options.crateName) {
    throw new Error('--all-configured cannot be combined with an explicit crate');
  }
  if (options.allConfigured && options.extraAllowedCrates.length > 0) {
    throw new Error('--all-configured cannot be combined with --allow-crate/--allow-list');
  }

  return options;
}

export function renderCratePublicApiUsage() {
  const managedCrates = MANAGED_CRATE_HELP_NAMES.map((crateName) => `  ${crateName}`).join('\n');
  return `Usage:
  node scripts/check-crate-public-api.mjs --crate <crate> [--allow-crate <crate> ...]
  node scripts/check-crate-public-api.mjs --all-configured
  node scripts/check-crate-public-api.mjs --self-test

Checks exported public API types with rustdoc JSON:
  cargo +nightly rustdoc -p <crate> --lib -- -Z unstable-options --output-format json
  RUSTC_BOOTSTRAP=1 cargo rustdoc -p <crate> --lib -- -Z unstable-options --output-format json

Default gated crates:
${managedCrates}`;
}

function reportGateEvent(event, { stderr, stdout }) {
  if (event.kind === 'skip') {
    stdout.write(
      `SKIP public API check for ${event.crateName}: package is not present in this workspace yet.\n`,
    );
    return;
  }
  if (event.kind === 'warning' && event.code === 'nightly-unavailable') {
    stderr.write(
      'Nightly Rust toolchain is unavailable; falling back to current toolchain with RUSTC_BOOTSTRAP=1.\n',
    );
    return;
  }
  if (event.kind === 'warning' && event.code === 'rustdoc-fallback-succeeded') {
    stderr.write(`Built rustdoc JSON for ${event.crateName} with ${event.label}.\n`);
    return;
  }
  if (event.kind === 'crate-result') {
    printConfig(event.crateName, event.config, stdout);
    printResult(event.result, { stderr, stdout });
    return;
  }
  throw new Error(`unknown crate public API gate event: ${JSON.stringify(event)}`);
}

function printConfig(crateName, config, stdout) {
  stdout.write(`Public API allow-list for ${crateName}: ${config.allowedCrates.join(', ')}\n`);
  stdout.write(`Policy: ${config.note}\n`);
}

function printResult(result, { stderr, stdout }) {
  if (result.violations.length === 0) {
    stdout.write(`Public API check passed for ${result.crateName}.\n`);
    return;
  }

  stderr.write(
    `Public API check failed for ${result.crateName}: ${result.violations.length} forbidden reference(s).\n`,
  );
  for (const violation of result.violations) {
    stderr.write(
      `DENY ${violation.site} references ${violation.referencedPath} from forbidden crate ${violation.crateName}\n`,
    );
  }
}

function splitCrateList(value) {
  return value
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean);
}
