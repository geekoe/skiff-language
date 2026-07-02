#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));

const runtimeDag = new Map([
  [
    'skiff-runtime-host',
    [
      'skiff-runtime-transport',
      'skiff-runtime-request',
      'skiff-runtime-package-test',
      'skiff-runtime-loader',
      'skiff-runtime-linker',
      'skiff-runtime-linked-program',
      'skiff-runtime-linked-type-plan',
      'skiff-runtime-activation',
      'skiff-runtime-capability-context',
      'skiff-runtime-eval',
      'skiff-runtime-native',
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-service-db',
    [
      'skiff-runtime-capability-context',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-transport',
    ['skiff-runtime-request-contract', 'skiff-runtime-model'],
  ],
  [
    'skiff-runtime-package-test',
    [
      'skiff-runtime-loader',
      'skiff-runtime-linked-program',
      'skiff-runtime-linker',
      'skiff-runtime-activation',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-request',
    [
      'skiff-runtime-eval',
      'skiff-runtime-request-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-capability-context',
      'skiff-runtime-linked-program',
      'skiff-runtime-linker',
      'skiff-runtime-activation',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-eval',
    [
      'skiff-runtime-native',
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-linked-type-plan',
      'skiff-runtime-linked-program',
      'skiff-runtime-capability-context',
      'skiff-runtime-activation',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-native',
    [
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-capability-context',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-capability-context',
    ['skiff-runtime-native-contract', 'skiff-runtime-boundary', 'skiff-runtime-model'],
  ],
  [
    'skiff-runtime-activation',
    [
      'skiff-runtime-linked-program',
      'skiff-runtime-linker',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-linked-type-plan',
    [
      'skiff-runtime-linked-program',
      'skiff-runtime-boundary',
      'skiff-runtime-native-contract',
      'skiff-runtime-model',
    ],
  ],
  [
    'skiff-runtime-linker',
    [
      'skiff-runtime-loader',
      'skiff-runtime-linked-program',
      'skiff-runtime-native-contract',
      'skiff-runtime-boundary',
      'skiff-runtime-model',
    ],
  ],
  ['skiff-runtime-linked-program', ['skiff-runtime-model']],
  [
    'skiff-runtime-request-contract',
    ['skiff-runtime-capability-context'],
  ],
  ['skiff-runtime-native-contract', ['skiff-runtime-model']],
  ['skiff-runtime-loader', ['skiff-runtime-model']],
  ['skiff-runtime-boundary', ['skiff-runtime-model']],
  ['skiff-runtime-model', []],
]);

const expectedPromotedRuntimePackages = new Set([
  'skiff-runtime-activation',
  'skiff-runtime-boundary',
  'skiff-runtime-capability-context',
  'skiff-runtime-eval',
  'skiff-runtime-host',
  'skiff-runtime-linked-program',
  'skiff-runtime-linked-type-plan',
  'skiff-runtime-linker',
  'skiff-runtime-loader',
  'skiff-runtime-model',
  'skiff-runtime-native',
  'skiff-runtime-native-contract',
  'skiff-runtime-package-test',
  'skiff-runtime-request',
  'skiff-runtime-request-contract',
  'skiff-runtime-service-db',
  'skiff-runtime-transport',
]);

const hostBoundaryTarget = {
  hostPackageName: 'skiff-runtime-host',
  docs: [
    'doc/architecture/runtime-layered-crate-architecture.md',
  ],
  allowedRuntimeDeps: [
    'skiff-runtime-transport',
    'skiff-runtime-request',
    'skiff-runtime-package-test',
    'skiff-runtime-loader',
    'skiff-runtime-linker',
    'skiff-runtime-linked-program',
    'skiff-runtime-activation',
    'skiff-runtime-capability-context',
    'skiff-runtime-model',
  ],
  temporaryDebtRationales: new Map([
    [
      'skiff-runtime-boundary',
      'host still calls other boundary utilities/conversions after request_mapper, control_mapper, and control_response_mapper moved router-session request/control/control-response frame mappings to transport',
    ],
    [
      'skiff-runtime-eval',
      'host still wires eval-facing request execution while request/eval adapters are being narrowed',
    ],
    [
      'skiff-runtime-native',
      'host still reaches native dispatch wiring that should be hidden behind eval/request composition',
    ],
    [
      'skiff-runtime-native-contract',
      'host still consumes native contract metadata during current request and package-test assembly',
    ],
    [
      'skiff-runtime-linked-type-plan',
      'host still reads linked type plans at register/type-plan boundaries before those projections move down',
    ],
  ]),
};

const expectedHostBoundaryTargetDebts = [
  'skiff-runtime-boundary',
  'skiff-runtime-eval',
  'skiff-runtime-native',
  'skiff-runtime-native-contract',
  'skiff-runtime-linked-type-plan',
];

try {
  const cliOptions = parseArgs(process.argv.slice(2));

  validateEncodedDag(runtimeDag);
  validateHostBoundaryTarget();

  if (cliOptions.help) {
    printUsage();
  } else if (cliOptions.selfTest) {
    runSelfTests();
  } else {
    const metadata = await cargoMetadata();
    const dagResult = checkRuntimeDag(metadata);
    printRuntimeDagResult(dagResult);

    let exitCode = dagResult.violations.length > 0 ? 1 : 0;

    if (cliOptions.hostBoundary !== null) {
      const hostBoundaryResult = checkHostBoundaryTarget(metadata);
      printHostBoundaryResult(hostBoundaryResult, cliOptions.hostBoundary);
      exitCode = Math.max(exitCode, hostBoundaryExitCode(hostBoundaryResult, cliOptions.hostBoundary));
    }

    if (exitCode !== 0) {
      process.exitCode = exitCode;
    }
  }
} catch (error) {
  console.error(`ERROR ${error.message}`);
  process.exitCode = 1;
}

function checkRuntimeDag(metadata) {
  const workspacePackages = workspaceMemberPackages(metadata);
  const workspacePackageNames = new Set(workspacePackages.map((pkg) => pkg.name));
  const promotedRuntimePackages = workspacePackages
    .filter((pkg) => isRuntimePackageName(pkg.name))
    .sort((left, right) => left.name.localeCompare(right.name));
  const violations = [];

  for (const packageName of expectedPromotedRuntimePackages) {
    if (!workspacePackageNames.has(packageName)) {
      violations.push({
        packageName,
        manifestPath: '(workspace)',
        message:
          'expected promoted runtime crate is not a workspace member; add its manifest to Cargo.toml members before relying on DAG checks',
      });
    }
  }

  for (const pkg of promotedRuntimePackages) {
    const allowedRuntimeDeps = runtimeDag.get(pkg.name);
    if (!allowedRuntimeDeps) {
      violations.push({
        packageName: pkg.name,
        manifestPath: pkg.manifest_path,
        message:
          'no runtime DAG rule is encoded for this promoted crate; add the architecture rule before adding the crate to the workspace',
      });
      continue;
    }

    const allowed = new Set(allowedRuntimeDeps);
    for (const dependency of pkg.dependencies ?? []) {
      if (!isRuntimePackageName(dependency.name)) {
        continue;
      }

      const kind = dependencyKind(dependency);
      if (!isProductionDependency(dependency)) {
        continue;
      }

      if (!workspacePackageNames.has(dependency.name)) {
        violations.push({
          packageName: pkg.name,
          manifestPath: pkg.manifest_path,
          message: `${kind} dependency ${dependency.name} is a skiff-runtime-* crate but is not a workspace member`,
        });
        continue;
      }

      if (!allowed.has(dependency.name)) {
        violations.push({
          packageName: pkg.name,
          manifestPath: pkg.manifest_path,
          message: `${kind} dependency ${dependency.name} is not allowed by the runtime crate DAG; allowed skiff-runtime-* dependencies: ${formatAllowed(allowedRuntimeDeps)}`,
        });
      }
    }
  }

  return { promotedRuntimePackages, violations };
}

function checkHostBoundaryTarget(metadata) {
  const failures = [];
  const workspacePackages = workspaceMemberPackages(metadata);
  const workspacePackageNames = new Set(workspacePackages.map((pkg) => pkg.name));
  const workspacePackageByName = new Map(workspacePackages.map((pkg) => [pkg.name, pkg]));
  const hostPackage = workspacePackageByName.get(hostBoundaryTarget.hostPackageName);
  const allowedTargetDeps = new Set(hostBoundaryTarget.allowedRuntimeDeps);
  const allowed = [];
  const debts = [];
  const unregisteredDebts = [];
  const ignoredNonProductionDeps = [];

  if (hostPackage === undefined) {
    failures.push(`${hostBoundaryTarget.hostPackageName} is not a workspace member`);
    return { failures, allowed, debts, unregisteredDebts, ignoredNonProductionDeps, retiredExpectedDebts: [] };
  }

  for (const dependency of hostPackage.dependencies ?? []) {
    if (!isRuntimePackageName(dependency.name)) {
      continue;
    }

    const edge = {
      packageName: hostPackage.name,
      dependencyName: dependency.name,
      kind: dependencyKind(dependency),
      manifestPath: hostPackage.manifest_path,
    };

    if (!workspacePackageNames.has(dependency.name)) {
      failures.push(
        `${hostPackage.name} ${edge.kind} dependency ${dependency.name} is a skiff-runtime-* crate but is not a workspace member`,
      );
      continue;
    }

    if (!isProductionDependency(dependency)) {
      ignoredNonProductionDeps.push(edge);
      continue;
    }

    if (allowedTargetDeps.has(dependency.name)) {
      allowed.push(edge);
      continue;
    }

    const rationale = hostBoundaryTarget.temporaryDebtRationales.get(dependency.name);
    if (rationale === undefined) {
      unregisteredDebts.push({
        ...edge,
        message:
          'target host boundary does not allow this direct production runtime dependency and no Stage 1 temporary debt rationale is registered',
      });
      continue;
    }

    debts.push({
      ...edge,
      rationale,
    });
  }

  const presentProductionDeps = new Set([
    ...allowed.map((edge) => edge.dependencyName),
    ...debts.map((edge) => edge.dependencyName),
    ...unregisteredDebts.map((edge) => edge.dependencyName),
  ]);
  const retiredExpectedDebts = expectedHostBoundaryTargetDebts.filter(
    (dependencyName) => !presentProductionDeps.has(dependencyName),
  );

  debts.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));
  unregisteredDebts.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));
  allowed.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));
  ignoredNonProductionDeps.sort((left, right) => left.dependencyName.localeCompare(right.dependencyName));

  return { failures, allowed, debts, unregisteredDebts, ignoredNonProductionDeps, retiredExpectedDebts };
}

function printRuntimeDagResult(result) {
  if (result.violations.length > 0) {
    console.error('\nRuntime crate DAG check failed.\n');
    console.error(
      'Only promoted skiff-runtime-* workspace crates are checked; non-runtime workspace crates are not constrained by this script.\n',
    );
    for (const violation of result.violations) {
      console.error(
        `- ${violation.packageName} (${toRepoRelative(violation.manifestPath)}): ${violation.message}`,
      );
    }
    return;
  }

  console.log(
    `Runtime crate DAG check passed for ${result.promotedRuntimePackages.length} promoted crate${result.promotedRuntimePackages.length === 1 ? '' : 's'}: ${formatCheckedPackages(result.promotedRuntimePackages)}.`,
  );
}

function printHostBoundaryResult(result, mode) {
  console.log(`\nRuntime host boundary target debt report (${mode} mode).`);
  console.log(`Docs: ${hostBoundaryTarget.docs.join(', ')}.`);
  console.log(`Target direct runtime deps: ${formatAllowed(hostBoundaryTarget.allowedRuntimeDeps)}.`);
  console.log('Only normal production dependencies are evaluated for target host debt.');

  if (result.failures.length > 0) {
    console.error('\nRuntime host boundary target structural failure(s):');
    for (const failure of result.failures) {
      console.error(`- ${failure}`);
    }
  }

  if (result.debts.length === 0 && result.unregisteredDebts.length === 0) {
    console.log('\nNo runtime host target dependency debt remains.');
  } else if (result.debts.length > 0) {
    console.log(
      `\nRuntime host target debt remains (${result.debts.length} direct production dependenc${result.debts.length === 1 ? 'y' : 'ies'}):`,
    );
    for (const debt of result.debts) {
      console.log(
        `- ${debt.packageName} -> ${debt.dependencyName} (${debt.kind}): ${debt.rationale}; temporarily allowed by the current DAG while Stage 1 tracks the migration.`,
      );
    }
  }

  if (result.unregisteredDebts.length > 0) {
    console.error(
      `\nUnregistered runtime host target debt (${result.unregisteredDebts.length} direct production dependenc${result.unregisteredDebts.length === 1 ? 'y' : 'ies'}):`,
    );
    for (const debt of result.unregisteredDebts) {
      console.error(`- ${debt.packageName} -> ${debt.dependencyName} (${debt.kind}): ${debt.message}.`);
    }
  }

  if (result.retiredExpectedDebts.length > 0) {
    console.log(`\nRetired Stage 1 expected debt not present in Cargo metadata: ${result.retiredExpectedDebts.join(', ')}.`);
  }

  if (result.ignoredNonProductionDeps.length > 0) {
    const ignored = result.ignoredNonProductionDeps.map(
      (edge) => `${edge.dependencyName} (${edge.kind})`,
    );
    console.log(`\nIgnored non-production host runtime deps: ${ignored.join(', ')}.`);
  }

  if (result.failures.length > 0 || result.unregisteredDebts.length > 0) {
    console.error(
      '\nRuntime host boundary target check failed because unregistered or structurally invalid debt must fail closed in every mode.',
    );
  } else if (mode === 'deny' && result.debts.length > 0) {
    console.error(
      '\nRuntime host boundary deny failed because target dependency debt remains. This is the expected Stage 1 failure until the listed host edges are removed or the target is intentionally corrected.',
    );
  } else if (mode === 'report') {
    console.log('\nReport mode is informational and exits 0 when only registered target debt remains.');
  }
}

function hostBoundaryExitCode(result, mode) {
  if (result.failures.length > 0 || result.unregisteredDebts.length > 0) {
    return 1;
  }
  if (mode === 'deny' && result.debts.length > 0) {
    return 1;
  }
  return 0;
}

async function cargoMetadata() {
  const { status, stdout, stderr } = await run('cargo', ['metadata', '--format-version', '1', '--no-deps']);
  if (status !== 0) {
    throw new Error(`cargo metadata failed with exit code ${status}\n${stderr}`.trim());
  }

  try {
    return JSON.parse(stdout);
  } catch (error) {
    throw new Error(`cargo metadata did not return valid JSON: ${error.message}`);
  }
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: root,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('close', (status) => {
      resolve({ status, stdout, stderr });
    });
  });
}

function workspaceMemberPackages(metadata) {
  if (!Array.isArray(metadata.packages) || !Array.isArray(metadata.workspace_members)) {
    throw new Error('cargo metadata is missing packages or workspace_members');
  }

  const workspaceMemberIds = new Set(metadata.workspace_members);
  return metadata.packages.filter((pkg) => workspaceMemberIds.has(pkg.id));
}

function isRuntimePackageName(packageName) {
  return packageName.startsWith('skiff-runtime-');
}

function dependencyKind(dependency) {
  return dependency.kind ?? 'normal';
}

function isProductionDependency(dependency) {
  return dependency.kind === null || dependency.kind === undefined || dependency.kind === 'normal';
}

function formatAllowed(allowed) {
  return allowed.length === 0 ? '(none)' : allowed.join(', ');
}

function formatCheckedPackages(packages) {
  if (packages.length === 0) {
    return '(none)';
  }
  return packages.map((pkg) => pkg.name).join(', ');
}

function toRepoRelative(path) {
  return relative(root, path).split('\\').join('/');
}

function validateEncodedDag(dag) {
  for (const [packageName, allowedDependencies] of dag) {
    for (const dependencyName of allowedDependencies) {
      if (!dag.has(dependencyName)) {
        throw new Error(`${packageName} allows unknown runtime dependency ${dependencyName}`);
      }
    }
  }

  const permanent = new Set();
  const temporary = new Set();
  const stack = [];

  for (const packageName of dag.keys()) {
    visit(packageName);
  }

  function visit(packageName) {
    if (permanent.has(packageName)) {
      return;
    }
    if (temporary.has(packageName)) {
      const cycleStart = stack.indexOf(packageName);
      const cycle = [...stack.slice(cycleStart), packageName].join(' -> ');
      throw new Error(`encoded runtime crate DAG contains a cycle: ${cycle}`);
    }

    temporary.add(packageName);
    stack.push(packageName);
    for (const dependencyName of dag.get(packageName) ?? []) {
      visit(dependencyName);
    }
    stack.pop();
    temporary.delete(packageName);
    permanent.add(packageName);
  }
}

function validateHostBoundaryTarget() {
  if (!runtimeDag.has(hostBoundaryTarget.hostPackageName)) {
    throw new Error(`host boundary target references unknown host package ${hostBoundaryTarget.hostPackageName}`);
  }

  const runtimePackages = new Set(runtimeDag.keys());
  const targetAllowedDeps = new Set(hostBoundaryTarget.allowedRuntimeDeps);

  for (const dependencyName of hostBoundaryTarget.allowedRuntimeDeps) {
    if (!runtimePackages.has(dependencyName)) {
      throw new Error(`host boundary target allows unknown runtime dependency ${dependencyName}`);
    }
  }

  for (const dependencyName of expectedHostBoundaryTargetDebts) {
    if (!runtimePackages.has(dependencyName)) {
      throw new Error(`expected host boundary debt references unknown runtime dependency ${dependencyName}`);
    }
    if (targetAllowedDeps.has(dependencyName)) {
      throw new Error(`expected host boundary debt ${dependencyName} is also target-allowed`);
    }
    if (!hostBoundaryTarget.temporaryDebtRationales.has(dependencyName)) {
      throw new Error(`expected host boundary debt ${dependencyName} is missing a temporary rationale`);
    }
  }

  for (const dependencyName of hostBoundaryTarget.temporaryDebtRationales.keys()) {
    if (!runtimePackages.has(dependencyName)) {
      throw new Error(`host boundary debt rationale references unknown runtime dependency ${dependencyName}`);
    }
    if (targetAllowedDeps.has(dependencyName)) {
      throw new Error(`host boundary debt rationale ${dependencyName} is also target-allowed`);
    }
  }
}

function parseArgs(args) {
  const options = {
    help: false,
    selfTest: false,
    hostBoundary: null,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--self-test' || arg === '--test') {
      options.selfTest = true;
      continue;
    }
    if (arg.startsWith('--host-boundary=')) {
      options.hostBoundary = parseHostBoundaryMode(arg.slice('--host-boundary='.length));
      continue;
    }
    if (arg === '--host-boundary') {
      const value = args[index + 1];
      if (value === undefined) {
        throw new Error('--host-boundary requires report or deny');
      }
      options.hostBoundary = parseHostBoundaryMode(value);
      index += 1;
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }

  return options;
}

function parseHostBoundaryMode(value) {
  if (value === 'report' || value === 'deny') {
    return value;
  }
  throw new Error(`--host-boundary must be report or deny, got ${value}`);
}

function printUsage() {
  console.log(`Usage: node scripts/check-runtime-crate-dag.mjs [--self-test] [--host-boundary=report|deny]

Default mode checks the current promoted skiff-runtime-* crate DAG.
--self-test runs synthetic checks without invoking cargo.
--host-boundary=report prints target host-boundary dependency debt and exits 0 for registered known debt only.
--host-boundary=deny exits non-zero while target host-boundary dependency debt remains.`);
}

function runSelfTests() {
  const cases = [
    {
      name: 'encoded runtime DAG validates',
      run: () => {
        validateEncodedDag(runtimeDag);
      },
    },
    {
      name: 'current runtime DAG fixture passes',
      run: () => {
        const result = checkRuntimeDag(metadataFromRuntimeDag());
        assert(result.violations.length === 0, `expected no violations, got ${result.violations.length}`);
      },
    },
    {
      name: 'current DAG rejects an unlisted runtime edge',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const modelPackage = metadata.packages.find((pkg) => pkg.name === 'skiff-runtime-model');
        modelPackage.dependencies.push(runtimeDependency('skiff-runtime-host'));
        const result = checkRuntimeDag(metadata);
        assert(
          result.violations.some(
            (violation) =>
              violation.packageName === 'skiff-runtime-model'
              && violation.message.includes('skiff-runtime-host is not allowed'),
          ),
          'expected skiff-runtime-model -> skiff-runtime-host to be rejected',
        );
      },
    },
    {
      name: 'current DAG ignores dev-only runtime edges',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const modelPackage = metadata.packages.find((pkg) => pkg.name === 'skiff-runtime-model');
        modelPackage.dependencies.push(runtimeDependency('skiff-runtime-host', 'dev'));
        const result = checkRuntimeDag(metadata);
        assert(
          result.violations.length === 0,
          `expected dev-only skiff-runtime-model -> skiff-runtime-host to be ignored, got ${result.violations.length} violations`,
        );
      },
    },
    {
      name: 'host boundary report flags Stage 1 target debt while current DAG passes',
      run: () => {
        const metadata = metadataFromRuntimeDag();
        const dagResult = checkRuntimeDag(metadata);
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(dagResult.violations.length === 0, 'expected current DAG fixture to pass');
        assertSameSet(
          hostResult.debts.map((edge) => edge.dependencyName),
          expectedHostBoundaryTargetDebts,
          'expected Stage 1 host target debts',
        );
        assert(hostResult.unregisteredDebts.length === 0, 'expected no unregistered host target debts');
      },
    },
    {
      name: 'host boundary report and deny modes have staged exit behavior',
      run: () => {
        const hostResult = checkHostBoundaryTarget(metadataFromRuntimeDag());
        assert(hostResult.unregisteredDebts.length === 0, 'expected current fixture debt to be fully registered');
        assert(hostBoundaryExitCode(hostResult, 'report') === 0, 'report mode should exit 0 for debt');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 1, 'deny mode should exit 1 for debt');
      },
    },
    {
      name: 'host boundary report fails on unregistered target debt',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: [
            ...hostBoundaryTarget.allowedRuntimeDeps,
            'skiff-runtime-request-contract',
          ],
        });
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(
          hostResult.unregisteredDebts.some(
            (edge) => edge.dependencyName === 'skiff-runtime-request-contract',
          ),
          'expected skiff-runtime-request-contract to be unregistered host target debt',
        );
        assert(hostBoundaryExitCode(hostResult, 'report') === 1, 'report mode should fail for unregistered debt');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 1, 'deny mode should fail for unregistered debt');
      },
    },
    {
      name: 'host boundary fails on retired service-db production edge',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: [
            ...hostBoundaryTarget.allowedRuntimeDeps,
            'skiff-runtime-service-db',
          ],
        });
        const dagResult = checkRuntimeDag(metadata);
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(
          dagResult.violations.some(
            (violation) =>
              violation.packageName === 'skiff-runtime-host'
              && violation.message.includes('skiff-runtime-service-db is not allowed'),
          ),
          'expected skiff-runtime-host -> skiff-runtime-service-db to be rejected by DAG',
        );
        assert(
          hostResult.unregisteredDebts.some(
            (edge) => edge.dependencyName === 'skiff-runtime-service-db',
          ),
          'expected skiff-runtime-service-db to be unregistered host target debt',
        );
        assert(hostBoundaryExitCode(hostResult, 'report') === 1, 'report mode should fail for service-db regression');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 1, 'deny mode should fail for service-db regression');
      },
    },
    {
      name: 'host boundary target allow-list has no debt when only target deps remain',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: hostBoundaryTarget.allowedRuntimeDeps,
        });
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(hostResult.debts.length === 0, `expected no host target debt, got ${hostResult.debts.length}`);
        assert(hostResult.unregisteredDebts.length === 0, 'expected no unregistered host target debt');
        assert(hostBoundaryExitCode(hostResult, 'deny') === 0, 'deny mode should pass without debt');
      },
    },
    {
      name: 'host boundary target ignores dev-only runtime dependencies',
      run: () => {
        const metadata = metadataFromRuntimeDag({
          hostDependencies: [
            ...hostBoundaryTarget.allowedRuntimeDeps,
            { name: 'skiff-runtime-boundary', kind: 'dev' },
          ],
        });
        const hostResult = checkHostBoundaryTarget(metadata);
        assert(hostResult.debts.length === 0, 'dev-only boundary dependency should not be target debt');
        assert(
          hostResult.ignoredNonProductionDeps.some(
            (edge) => edge.dependencyName === 'skiff-runtime-boundary' && edge.kind === 'dev',
          ),
          'expected dev-only boundary dependency to be reported as ignored',
        );
      },
    },
  ];

  const failures = [];
  for (const testCase of cases) {
    try {
      testCase.run();
      console.log(`PASS ${testCase.name}`);
    } catch (error) {
      failures.push(`${testCase.name}: ${error.message}`);
      console.error(`FAIL ${testCase.name}: ${error.message}`);
    }
  }

  if (failures.length > 0) {
    process.exitCode = 1;
    return;
  }

  console.log(`Runtime crate DAG self-test passed (${cases.length} cases).`);
}

function metadataFromRuntimeDag(options = {}) {
  const packageNames = [...expectedPromotedRuntimePackages].sort();
  const packages = packageNames.map((packageName) => {
    const dependencies =
      packageName === hostBoundaryTarget.hostPackageName && options.hostDependencies !== undefined
        ? options.hostDependencies
        : runtimeDag.get(packageName) ?? [];
    return packageFixture(packageName, dependencies);
  });

  return {
    packages,
    workspace_members: packages.map((pkg) => pkg.id),
  };
}

function packageFixture(packageName, dependencies) {
  return {
    id: `${packageName} 0.1.0 (path+file:///workspace/${packageName})`,
    name: packageName,
    manifest_path: `/workspace/${packageName}/Cargo.toml`,
    dependencies: dependencies.map((dependency) =>
      typeof dependency === 'string' ? runtimeDependency(dependency) : runtimeDependency(dependency.name, dependency.kind),
    ),
  };
}

function runtimeDependency(name, kind = null) {
  return { name, kind };
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function assertSameSet(actual, expected, message) {
  const actualSorted = [...actual].sort();
  const expectedSorted = [...expected].sort();
  if (
    actualSorted.length !== expectedSorted.length
    || actualSorted.some((value, index) => value !== expectedSorted[index])
  ) {
    throw new Error(`${message}: expected ${expectedSorted.join(', ')}, got ${actualSorted.join(', ')}`);
  }
}
