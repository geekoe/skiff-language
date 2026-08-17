import { PUBLIC_SELECTORS } from './verify-plan.mjs';
import { renderLiveSelectorHelp } from './verify-live-registry.mjs';

const runtimeLiveValueOptions = new Map([
  ['--runtime-live-ingress-url', 'runtimeLiveIngressUrl'],
  ['--runtime-live-artifact-root', 'runtimeLiveArtifactRoot'],
  ['--runtime-live-profile', 'runtimeLiveProfile'],
]);

export function parseVerifyArgs(argv) {
  const options = {
    help: false,
    list: false,
    jobs: undefined,
    selectors: [],
    runtimeLiveIngressUrl: undefined,
    runtimeLiveArtifactRoot: undefined,
    runtimeLiveProfile: undefined,
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
  tooling                      scripts and VS Code tooling tests

quality and focused selectors:
  verify                       tests plus every non-live quality/check gate
  rust-quality                 workspace rustfmt + Rust file/function line gates
  bytecode-vm-phase-0-gate     Phase 0 exact-candidate closure gate
  bytecode-vm-phase-1-gate     Phase 1 trusted synchronous core gate
  bytecode-vm-phase-2-gate     Phase 2 value-lifecycle gate (Phase 1 full regression)
  bytecode-vm-phase-3-gate     Phase 3 exception/unwind lifecycle gate
  bytecode-vm-phase-4-gate     Phase 4 request/Pending/cancel/deadline gate
  bytecode-vm-phase-5-gate     Phase 5 r1 typed host/resource/stream full-chain gate
  bytecode-vm-phase-6-gate     Phase 6 r1 cross-owner expected-red Gate baseline
  bytecode-vm-phase-7-gate     Phase 7 whole-system closure Gate
  type-check                   scripts and VS Code static checks
  checks                       repository architecture and policy checks
  scripts  vscode              focused tooling tests
  scripts-syntax  scripts-dev-sync  focused tooling tasks
  compiler-boundaries          focused compiler source-boundary check
${renderLiveSelectorHelp()}

options:
  --only <a,b>                 select one or more groups; may be specified once
  --jobs <n>                   concurrent slot budget; default 1, minimum 1
  --list, --dry-run            print the expanded plan without executing it
  --runtime-live-ingress-url <url>
                                explicit runtime ingress origin
  --runtime-live-artifact-root <dir>
                                explicit existing runtime artifact directory
  --runtime-live-profile <id>
                                explicit profile
  --loop-risk-config <path>     canonical loop-risk target/runtime config
  -h, --help                   show this help

Loop-risk live selectors require one canonical --loop-risk-config path (or SKIFF_LOOP_RISK_CONFIG);
the default plan runs only the hermetic health evaluator self-test, never a live loop-risk target.
The checks selector includes compiler boundaries plus hermetic and actual configured public API
checks; rustdoc falls back to the current toolchain when nightly is unavailable.

The bytecode-vm-phase-0-gate selector requires all three caller-supplied environment variables:
  SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE0_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE0_EVIDENCE_DIR       caller-chosen canonical absolute absent path
The Gate checks those identities; it does not choose them from HEAD or choose a temporary evidence
directory.

The bytecode-vm-phase-1-gate selector uses an independent evidence epoch and requires:
  SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE1_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE1_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It checks a fixed 21-receipt K0/T-C/T-R/V1/Phase-0-regression matrix and never selects
its own candidate or evidence directory.`);

  console.log(`
The bytecode-vm-phase-2-gate selector uses an independent evidence epoch and requires:
  SKIFF_BYTECODE_VM_PHASE2_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE2_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE2_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It checks the Phase 2 VCP/missing-plan/K2/C2 scenario matrix plus the Phase 1 full
regression selector, and never selects its own candidate or evidence directory.`);

  console.log(`
The bytecode-vm-phase-3-gate selector requires:
  SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE3_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE3_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It checks the Phase 3 exception/unwind VCP/negative/K3/C3 matrix plus the Phase 1/2
regression, and never selects its own candidate or evidence directory.`);

  console.log(`
The bytecode-vm-phase-4-gate selector requires:
  SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE4_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE4_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It checks the Phase 4 VCP/sentinel/negative/K4/V4/C4 matrix plus the Phase 1/2/3
regression, and never selects its own candidate or evidence directory.`);

  console.log(`
The bytecode-vm-phase-5-gate selector is the independent recovery epoch r1 and requires:
  SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE5_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE5_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It executes G1-G10 plus the complete accepted Phase 1-4 regression under the exclusive
/tmp/skiff-bcvm-p5-r1-cargo.lockdir lease and records every command without fail-fast.`);

  console.log(`
The bytecode-vm-phase-6-gate selector is the independent expected-red epoch r1 and requires:
  SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE6_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It executes G6 plus the complete accepted Phase 1-5 regression under the exclusive
/tmp/skiff-bcvm-p6-r1-cargo.lockdir lease, records every command, and continues after reds.`);

  console.log(`
The bytecode-vm-phase-7-gate selector is the whole-system closure epoch and requires:
  SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_COMMIT   literal 40-hex commit from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE7_CANDIDATE_TREE     literal 40-hex tree from the freeze receipt
  SKIFF_BYTECODE_VM_PHASE7_EVIDENCE_DIR       caller-chosen canonical absolute absent path
It executes G7 plus the exactly-one Phase 6 cumulative import under the exclusive
/tmp/skiff-bcvm-p7-r1-cargo.lockdir lease, binds a deterministic receipt hash chain, and
never reuses an evidence directory or re-derives provenance from ID prefixes.`);
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
