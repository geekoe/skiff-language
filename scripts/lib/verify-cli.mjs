import { PUBLIC_SELECTORS } from './verify-plan.mjs';
import { renderLiveSelectorHelp } from './verify-live-registry.mjs';

const runtimeLiveValueOptions = new Map([
  ['--runtime-live-activation-url', 'runtimeLiveActivationUrl'],
  ['--runtime-live-ingress-url', 'runtimeLiveIngressUrl'],
  ['--runtime-live-artifact-root', 'runtimeLiveArtifactRoot'],
  ['--runtime-live-environment', 'runtimeLiveEnvironment'],
  ['--runtime-live-expected-generation', 'runtimeLiveExpectedGeneration'],
]);

export function parseVerifyArgs(argv) {
  const options = {
    help: false,
    list: false,
    jobs: undefined,
    selectors: [],
    runtimeLiveActivationUrl: undefined,
    runtimeLiveIngressUrl: undefined,
    runtimeLiveArtifactRoot: undefined,
    runtimeLiveEnvironment: undefined,
    runtimeLiveExpectedGeneration: undefined,
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
    if (arg === '--jobs') {
      rejectRepeatedFlag(options.jobs !== undefined, '--jobs');
      options.jobs = parseJobsValue(requiredValue(argv, index, '--jobs'));
      index += 1;
      continue;
    }
    if (arg.startsWith('--jobs=')) {
      rejectRepeatedFlag(options.jobs !== undefined, '--jobs');
      options.jobs = parseJobsValue(arg.slice('--jobs='.length));
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
    const equalsIndex = arg.indexOf('=');
    const optionName = equalsIndex === -1 ? arg : arg.slice(0, equalsIndex);
    const optionKey = runtimeLiveValueOptions.get(optionName);
    if (optionKey !== undefined) {
      const value = equalsIndex === -1
        ? requiredValue(argv, index, optionName)
        : requiredInlineValue(arg, optionName);
      setSingletonOption(options, optionKey, value, optionName);
      if (equalsIndex === -1) {
        index += 1;
      }
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
  if (options.jobs === undefined) {
    options.jobs = 1;
  }
  return options;
}

export function printVerifyUsage() {
  console.log(`usage: node scripts/verify.mjs [--only <selectors>] [--jobs <n>] [--list]

Default: complete non-live repository verification (tests + quality/check gates).
Runs every selected task and reports all failures; use --list to audit every selected task before running.

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
  rust-quality                 workspace rustfmt + Rust file/function line gates
  type-check                   Router, telemetry, scripts, and VS Code static checks
  checks                       repository architecture and policy checks
  scripts  vscode              focused tooling tests
  scripts-syntax  scripts-dev-sync  focused tooling tasks
  compiler-boundaries          focused compiler source-boundary check
${renderLiveSelectorHelp()}

options:
  --only <a,b>                 select one or more groups; may be specified once
  --jobs <n>                   concurrent slot budget; default 1, minimum 1
  --list, --dry-run            print the expanded plan without executing it
  --runtime-live-activation-url <url>
                                explicit canonical assembly activation target
  --runtime-live-ingress-url <url>
                                explicit runtime ingress origin
  --runtime-live-artifact-root <dir>
                                explicit existing runtime artifact directory
  --runtime-live-environment <id>
                                explicit activation environment
  --runtime-live-expected-generation <n>
                                explicit non-negative expected generation
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

function parseJobsValue(value) {
  if (!/^(?:[1-9][0-9]*)$/.test(value) || !Number.isSafeInteger(Number(value))) {
    throw new Error('--jobs requires a positive integer');
  }
  return Number(value);
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
