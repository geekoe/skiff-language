import { PUBLIC_SELECTORS } from './verify-plan.mjs';
import { renderLiveSelectorHelp } from './verify-live-registry.mjs';

export function parseVerifyArgs(argv) {
  const options = {
    help: false,
    list: false,
    selectors: [],
    runtimeLiveConfig: undefined,
    runtimeLiveReloadUrl: undefined,
    runtimeLiveArtifactRoot: undefined,
    loopRiskConfig: undefined,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--') {
      continue;
    }
    if (arg === '--help' || arg === '-h') {
      rejectRepeatedFlag(options.help, '--help');
      options.help = true;
      continue;
    }
    if (arg === '--list' || arg === '--dry-run') {
      rejectRepeatedFlag(options.list, '--list/--dry-run');
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
      setSingletonOption(
        options,
        'runtimeLiveConfig',
        requiredValue(argv, index, '--runtime-live-config'),
        '--runtime-live-config',
      );
      index += 1;
      continue;
    }
    if (arg.startsWith('--runtime-live-config=')) {
      setSingletonOption(
        options,
        'runtimeLiveConfig',
        requiredInlineValue(arg, '--runtime-live-config'),
        '--runtime-live-config',
      );
      continue;
    }
    if (arg === '--runtime-live-reload-url') {
      setSingletonOption(
        options,
        'runtimeLiveReloadUrl',
        requiredValue(argv, index, '--runtime-live-reload-url'),
        '--runtime-live-reload-url',
      );
      index += 1;
      continue;
    }
    if (arg.startsWith('--runtime-live-reload-url=')) {
      setSingletonOption(
        options,
        'runtimeLiveReloadUrl',
        requiredInlineValue(arg, '--runtime-live-reload-url'),
        '--runtime-live-reload-url',
      );
      continue;
    }
    if (arg === '--runtime-live-artifact-root') {
      setSingletonOption(
        options,
        'runtimeLiveArtifactRoot',
        requiredValue(argv, index, '--runtime-live-artifact-root'),
        '--runtime-live-artifact-root',
      );
      index += 1;
      continue;
    }
    if (arg.startsWith('--runtime-live-artifact-root=')) {
      setSingletonOption(
        options,
        'runtimeLiveArtifactRoot',
        requiredInlineValue(arg, '--runtime-live-artifact-root'),
        '--runtime-live-artifact-root',
      );
      continue;
    }
    if (arg === '--loop-risk-config') {
      setSingletonOption(
        options,
        'loopRiskConfig',
        requiredValue(argv, index, '--loop-risk-config'),
        '--loop-risk-config',
      );
      index += 1;
      continue;
    }
    if (arg.startsWith('--loop-risk-config=')) {
      setSingletonOption(
        options,
        'loopRiskConfig',
        requiredInlineValue(arg, '--loop-risk-config'),
        '--loop-risk-config',
      );
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

Default: complete non-live repository verification (tests + quality/check gates).
Execution is fail-fast; use --list to audit every selected phase before running.

test domains:
  tests                        all non-live Skiff source and implementation tests
  skiff-tests                  canonical Skiff source suite on a reusable real runtime
  implementation-tests         all implementation subjects below

implementation subjects:
  foundation                   shared artifact-model, artifact-identity, and syntax crates
  compiler                     compiler crate tests
  runtime                      runtime crate tests
  test-runner                  test-runner crate tests
  router                       router tests
  telemetry                    telemetry tests
  tooling                      scripts and VS Code tooling tests

quality and focused selectors:
  verify                       tests plus every non-live quality/check gate
  rust-quality                 workspace rustfmt + baseline-aware Clippy gate
  type-check                   Router, telemetry, scripts, and VS Code static checks
  checks                       repository architecture and policy checks
  scripts  vscode              focused tooling tests
  scripts-syntax  scripts-dev-sync  focused tooling phases
  compiler-boundaries          focused compiler source-boundary check
${renderLiveSelectorHelp()}

options:
  --only <a,b>                 select one or more groups; may be specified once
  --list, --dry-run            print the expanded plan without executing it
  --runtime-live-config <path> runtime-live config, relative to the repository root
  --runtime-live-reload-url <url>
                                explicit http://host:port router reload target
  --runtime-live-artifact-root <dir>
                                explicit existing runtime artifact directory
  --loop-risk-config <path>     canonical loop-risk target/runtime config
  -h, --help                   show this help

Loop-risk live selectors require one canonical --loop-risk-config path (or SKIFF_LOOP_RISK_CONFIG);
the default plan runs only the hermetic health evaluator self-test, never a live loop-risk target.
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

function setSingletonOption(options, key, value, option) {
  if (options[key] !== undefined) {
    throw new Error(`${option} may be specified only once`);
  }
  options[key] = value;
}

function rejectRepeatedFlag(alreadySet, option) {
  if (alreadySet) {
    throw new Error(`${option} may be specified only once`);
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
