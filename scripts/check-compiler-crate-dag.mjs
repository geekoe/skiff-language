#!/usr/bin/env node

import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { captureCheckedCommand } from './lib/command-execution.mjs';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const defaultCliPhase = 10;
const facadePackage = 'skiff-compiler';
const contractPackage = 'skiff-compiler-contract';
const terminalProducerPackages = [facadePackage, contractPackage];
const terminalPackageDependencies = [
  contractPackage,
  'skiff-compiler-core',
  'skiff-compiler-input-model',
  'skiff-compiler-input',
  'skiff-compiler-source',
  'skiff-compiler-lowering',
  'skiff-compiler-compiled',
  'skiff-compiler-projection-input',
  'skiff-compiler-projection',
  'skiff-compiler-emission',
  'skiff-artifact-model',
  'skiff-artifact-identity',
  'skiff-canonical-json',
  'skiff-syntax',
];

// The public compiler graph has only two owners: package compilation and code-free contract compilation.
const finalProductionEdges = new Map([
  [facadePackage, terminalPackageDependencies],
  [contractPackage, ['skiff-artifact-model', 'skiff-artifact-identity']],
]);

const cliOptions = parseCliOptions(process.argv.slice(2));

if (cliOptions.help) {
  printUsage();
} else if (cliOptions.selfTest) {
  runSelfTests();
} else {
  const metadata = await readCargoMetadata();
  const result = checkCompilerCrateDag(metadata, {
    phase: cliOptions.phase,
  });
  printCheckResult(result);
  if (result.failures.length > 0) {
    process.exitCode = 1;
  }
}

function checkCompilerCrateDag(metadata, options = {}) {
  const phase = options.phase ?? defaultCliPhase;
  const result = {
    phase,
    failures: [],
    notes: [],
    checkedEdges: [],
  };

  const workspaceMemberIds = new Set(metadata.workspace_members ?? []);
  const packageById = new Map((metadata.packages ?? []).map((pkg) => [pkg.id, pkg]));
  const workspacePackagesByName = new Map();

  for (const id of workspaceMemberIds) {
    const pkg = packageById.get(id);
    if (pkg === undefined) {
      result.failures.push(`workspace member id ${id} is missing from cargo metadata packages`);
      continue;
    }
    if (workspacePackagesByName.has(pkg.name)) {
      result.failures.push(
        `workspace package name ${pkg.name} appears more than once; DAG rules require unique package names`,
      );
      continue;
    }
    workspacePackagesByName.set(pkg.name, pkg);
  }

  for (const packageName of terminalProducerPackages) {
    if (!workspacePackagesByName.has(packageName)) {
      result.failures.push(`required terminal compiler package is missing from workspace: ${packageName}`);
    }
  }

  if (metadata.resolve === null || metadata.resolve === undefined || !Array.isArray(metadata.resolve.nodes)) {
    result.failures.push('cargo metadata resolve graph is missing; run cargo metadata without --no-deps');
    return result;
  }

  const resolveNodeById = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));

  for (const packageName of terminalProducerPackages) {
    const sourcePackage = workspacePackagesByName.get(packageName);
    if (sourcePackage === undefined) {
      continue;
    }
    const node = resolveNodeById.get(sourcePackage.id);
    if (node === undefined) {
      result.failures.push(`workspace package ${packageName} is missing from cargo metadata resolve nodes`);
      continue;
    }
    if (!Array.isArray(node.deps)) {
      result.failures.push(`workspace package ${packageName} has no resolved dependency kind data`);
      continue;
    }

    for (const resolvedDependency of node.deps) {
      if (!workspaceMemberIds.has(resolvedDependency.pkg)) {
        continue;
      }
      const dependencyPackage = packageById.get(resolvedDependency.pkg);
      if (dependencyPackage === undefined) {
        result.failures.push(
          `resolved dependency ${resolvedDependency.pkg} of ${packageName} is missing from cargo metadata packages`,
        );
        continue;
      }

      for (const dependencyKind of resolvedDependencyKinds(resolvedDependency)) {
        const edge = {
          package: packageName,
          dependency: dependencyPackage.name,
          dependency_kind: dependencyKind,
          dependency_key: resolvedDependency.name,
        };
        result.checkedEdges.push(edge);
        checkEdge(edge, phase, result);
      }
    }
  }

  return result;
}

function checkEdge(edge, phase, result) {
  if (isAllowedProductionEdge(edge.package, edge.dependency)) {
    return;
  }
  result.failures.push(formatDisallowedEdge(edge, phase));
}

function isAllowedProductionEdge(packageName, dependencyName) {
  const allowedDependencies = finalProductionEdges.get(packageName);
  if (allowedDependencies === undefined) {
    return false;
  }
  return allowedDependencies.includes(dependencyName);
}

function resolvedDependencyKinds(resolvedDependency) {
  const depKinds = resolvedDependency.dep_kinds;
  if (!Array.isArray(depKinds) || depKinds.length === 0) {
    return ['normal'];
  }
  return unique(depKinds.map((depKind) => normalizeDependencyKind(depKind.kind)));
}

function normalizeDependencyKind(kind) {
  if (kind === null || kind === undefined || kind === '') {
    return 'normal';
  }
  return String(kind);
}

function formatDisallowedEdge(edge, phase) {
  return [
    `${edge.package} has disallowed ${edge.dependency_kind} dependency on ${edge.dependency} in phase ${phase}`,
    `(Cargo.toml dependency key: ${JSON.stringify(edge.dependency_key)})`,
  ].join(' ');
}

function printCheckResult(result) {
  for (const note of result.notes) {
    console.log(`NOTE ${note}`);
  }
  for (const failure of result.failures) {
    console.error(`FAIL ${failure}`);
  }
  if (result.failures.length > 0) {
    console.error(`Compiler crate DAG check failed for phase ${result.phase}: ${result.failures.length} failure(s).`);
  } else {
    console.log(
      `Compiler crate DAG check passed for phase ${result.phase}: ${result.checkedEdges.length} workspace edge(s) checked.`,
    );
  }
}

function parseCliOptions(args) {
  const options = {
    phase: defaultCliPhase,
    selfTest: false,
    help: false,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--self-test' || arg === '--test') {
      options.selfTest = true;
      continue;
    }
    if (arg === '--help' || arg === '-h') {
      options.help = true;
      continue;
    }
    if (arg === '--phase') {
      index += 1;
      options.phase = parsePhase(args[index]);
      continue;
    }
    if (arg.startsWith('--phase=')) {
      options.phase = parsePhase(arg.slice('--phase='.length));
      continue;
    }
    throw new Error(`unknown argument ${arg}`);
  }

  return options;
}

function parsePhase(value) {
  const phase = Number(value);
  if (!Number.isFinite(phase) || phase < 0 || phase > 10) {
    throw new Error(`phase must be a number from 0 through 10, got ${value}`);
  }
  return phase;
}

function printUsage() {
  console.log(
    `usage: node scripts/check-compiler-crate-dag.mjs [--phase <0-10>] [--self-test]\nDefault phase: ${defaultCliPhase}`,
  );
}

async function readCargoMetadata() {
  let result;
  try {
    result = await captureCheckedCommand(
      'cargo',
      ['metadata', '--format-version', '1'],
      {
        cwd: root,
      },
    );
  } catch (error) {
    throw new Error([
      `cargo metadata failed with ${error?.signal ?? error?.code ?? 'UNKNOWN'}`,
      streamDiagnostic('stderr', error?.stderr),
      streamDiagnostic('stdout', error?.stdout),
    ].filter(Boolean).join('\n'));
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`failed to parse cargo metadata JSON: ${error.message}`);
  }
}

function streamDiagnostic(label, value) {
  return typeof value === 'string' && value.trim().length > 0
    ? `${label}:\n${value.trim()}`
    : '';
}

function runSelfTests() {
  const tests = [
    {
      name: 'terminal producers accept the final package and contract edges by resolved package name',
      run: () => {
        const metadata = fixtureMetadata({
          packages: [
            facadePackage,
            contractPackage,
            'skiff-compiler-core',
            'skiff-compiler-emission',
            'skiff-artifact-model',
            'skiff-artifact-identity',
            'skiff-syntax',
          ],
          edges: [
            {
              package: facadePackage,
              dependency: contractPackage,
              dependency_key: 'contract_renamed',
              dependency_kind: 'normal',
            },
            {
              package: facadePackage,
              dependency: 'skiff-compiler-core',
              dependency_kind: 'normal',
            },
            {
              package: facadePackage,
              dependency: 'skiff-compiler-emission',
              dependency_kind: 'dev',
            },
            {
              package: facadePackage,
              dependency: 'skiff-syntax',
              dependency_key: 'syntax_renamed',
              dependency_kind: 'normal',
            },
            {
              package: contractPackage,
              dependency: 'skiff-artifact-model',
              dependency_kind: 'normal',
            },
            {
              package: contractPackage,
              dependency: 'skiff-artifact-identity',
              dependency_kind: 'normal',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata);
        assertPass(result, 'terminal dependency graph should pass');
        assertEqual(result.checkedEdges.length, 6, 'every terminal owner edge should be checked');
      },
    },
    {
      name: 'both terminal producer packages are required',
      run: () => {
        const metadata = fixtureMetadata({
          packages: [facadePackage],
          edges: [],
        });
        const result = checkCompilerCrateDag(metadata);
        assertFail(result, 'missing contract producer must fail');
        assertIncludes(
          result.failures.join('\n'),
          `required terminal compiler package is missing from workspace: ${contractPackage}`,
        );
      },
    },
    {
      name: 'deleted publication ABI edge fails in every phase',
      run: () => {
        const metadata = fixtureMetadata({
          packages: [facadePackage, contractPackage, 'skiff-compiler-publication-abi'],
          edges: [
            {
              package: facadePackage,
              dependency: 'skiff-compiler-publication-abi',
              dependency_kind: 'normal',
            },
          ],
        });
        for (const phase of [0, defaultCliPhase]) {
          const result = checkCompilerCrateDag(metadata, { phase });
          assertFail(result, `publication ABI edge must fail in phase ${phase}`);
          assertIncludes(
            result.failures.join('\n'),
            `${facadePackage} has disallowed normal dependency on skiff-compiler-publication-abi in phase ${phase}`,
          );
        }
      },
    },
    {
      name: 'temporary exception-shaped input cannot exempt dev or build edges',
      run: () => {
        const metadata = fixtureMetadata({
          packages: [facadePackage, contractPackage, 'skiff-compiler-publication-abi'],
          edges: [
            {
              package: facadePackage,
              dependency: 'skiff-compiler-publication-abi',
              dependency_kind: 'dev',
            },
            {
              package: facadePackage,
              dependency: 'skiff-compiler-publication-abi',
              dependency_kind: 'build',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata, {
          exceptions: [{
            package: facadePackage,
            dependency: 'skiff-compiler-publication-abi',
            dependency_kind: 'dev',
            phase: defaultCliPhase,
            reason: 'legacy option must have no effect',
            remove_when: 'already terminal',
          }],
        });
        assertFail(result, 'deleted exception mechanism must not exempt edges');
        assertEqual(result.failures.length, 2, 'both disallowed dependency kinds should fail');
        assertEqual(result.usedExceptions, undefined, 'result must not expose an exception ledger');
      },
    },
    {
      name: 'contract compiler cannot depend on facade or package implementation crates',
      run: () => {
        const metadata = fixtureMetadata({
          packages: [facadePackage, contractPackage, 'skiff-syntax'],
          edges: [
            {
              package: contractPackage,
              dependency: facadePackage,
              dependency_kind: 'normal',
            },
            {
              package: contractPackage,
              dependency: 'skiff-syntax',
              dependency_kind: 'normal',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata);
        assertFail(result, 'code-free contract producer must remain independent');
        assertIncludes(
          result.failures.join('\n'),
          `${contractPackage} has disallowed normal dependency on ${facadePackage}`,
        );
        assertIncludes(
          result.failures.join('\n'),
          `${contractPackage} has disallowed normal dependency on skiff-syntax`,
        );
      },
    },
    {
      name: 'non-terminal implementation crates are not public DAG owners',
      run: () => {
        const metadata = fixtureMetadata({
          packages: [facadePackage, contractPackage, 'skiff-compiler-core'],
          edges: [
            {
              package: 'skiff-compiler-core',
              dependency: facadePackage,
              dependency_kind: 'normal',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata);
        assertPass(result, 'only the two terminal producer owners should be declared and checked');
        assertEqual(result.checkedEdges.length, 0, 'implementation-crate edges must be outside this public DAG');
      },
    },
    {
      name: 'missing resolve graph fails closed after terminal owner validation',
      run: () => {
        const metadata = fixtureMetadata({ packages: [facadePackage, contractPackage], edges: [] });
        metadata.resolve = null;
        const result = checkCompilerCrateDag(metadata);
        assertFail(result, 'missing resolve graph must fail');
        assertIncludes(result.failures.join('\n'), 'cargo metadata resolve graph is missing');
      },
    },
  ];

  const failures = [];
  for (const test of tests) {
    try {
      test.run();
      console.log(`ok ${test.name}`);
    } catch (error) {
      failures.push(`${test.name}: ${error.message}`);
      console.error(`not ok ${test.name}`);
      console.error(error.stack ?? error.message);
    }
  }

  if (failures.length > 0) {
    console.error(`Compiler crate DAG self-test failed: ${failures.length} failure(s).`);
    process.exitCode = 1;
    return;
  }
  console.log(`Compiler crate DAG self-test passed: ${tests.length} test(s).`);
}

function fixtureMetadata({ packages, edges }) {
  const packageEntries = packages.map((name) => ({
    name,
    version: '0.0.0',
    id: fixturePackageId(name),
    source: null,
  }));
  const nodes = packageEntries.map((pkg) => ({
    id: pkg.id,
    deps: [],
  }));
  const nodeByName = new Map(nodes.map((node, index) => [packageEntries[index].name, node]));

  for (const edge of edges) {
    const node = nodeByName.get(edge.package);
    if (node === undefined) {
      throw new Error(`fixture edge package ${edge.package} is not declared`);
    }
    if (!packages.includes(edge.dependency)) {
      throw new Error(`fixture edge dependency ${edge.dependency} is not declared`);
    }
    node.deps.push({
      name: edge.dependency_key ?? edge.dependency,
      pkg: fixturePackageId(edge.dependency),
      dep_kinds: [
        {
          kind: edge.dependency_kind === 'normal' ? null : edge.dependency_kind,
          target: null,
        },
      ],
    });
  }

  return {
    packages: packageEntries,
    workspace_members: packageEntries.map((pkg) => pkg.id),
    resolve: {
      nodes,
    },
  };
}

function fixturePackageId(name) {
  return `path+file:///fixture/${name}#${name}@0.0.0`;
}

function assertPass(result, message) {
  if (result.failures.length > 0) {
    throw new Error(`${message}: ${result.failures.join('; ')}`);
  }
}

function assertFail(result, message) {
  if (result.failures.length === 0) {
    throw new Error(`${message}: expected failure`);
  }
}

function assertIncludes(text, expected) {
  if (!text.includes(expected)) {
    throw new Error(`expected ${JSON.stringify(text)} to include ${JSON.stringify(expected)}`);
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, got ${actual}`);
  }
}

function unique(values) {
  return [...new Set(values)];
}
