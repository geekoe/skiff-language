#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const defaultCliPhase = 10;
const facadePackage = 'skiff-compiler';
const artifactPackages = ['skiff-artifact-model', 'skiff-artifact-identity'];
const compilerSubCrates = [
  'skiff-syntax',
  'skiff-compiler-core',
  'skiff-compiler-input-model',
  'skiff-compiler-input',
  'skiff-compiler-source',
  'skiff-compiler-lowering',
  'skiff-compiler-projection-input',
  'skiff-compiler-compiled',
  'skiff-compiler-projection',
  'skiff-compiler-emission',
];
const targetPackageNames = [
  ...artifactPackages,
  ...compilerSubCrates,
  facadePackage,
];

// skiff-artifact-identity owns shared identity prefixes/projections used by compiler stages.
const finalProductionEdges = new Map([
  ['skiff-artifact-model', []],
  ['skiff-artifact-identity', ['skiff-artifact-model']],
  ['skiff-syntax', []],
  ['skiff-compiler-core', ['skiff-syntax', 'skiff-artifact-model', 'skiff-artifact-identity']],
  ['skiff-compiler-input-model', ['skiff-syntax', 'skiff-compiler-core', 'skiff-artifact-model']],
  [
    'skiff-compiler-input',
    [
      'skiff-syntax',
      'skiff-compiler-core',
      'skiff-compiler-input-model',
      'skiff-artifact-model',
      'skiff-artifact-identity',
    ],
  ],
  [
    'skiff-compiler-source',
    [
      'skiff-syntax',
      'skiff-compiler-core',
      'skiff-compiler-input-model',
      'skiff-artifact-model',
      'skiff-artifact-identity',
    ],
  ],
  [
    'skiff-compiler-lowering',
    [
      'skiff-syntax',
      'skiff-compiler-core',
      'skiff-compiler-source',
      'skiff-artifact-model',
      'skiff-artifact-identity',
    ],
  ],
  ['skiff-compiler-projection-input', ['skiff-compiler-core', 'skiff-artifact-model']],
  [
    'skiff-compiler-compiled',
    [
      'skiff-compiler-core',
      'skiff-compiler-source',
      'skiff-compiler-lowering',
      'skiff-compiler-projection-input',
      'skiff-artifact-model',
    ],
  ],
  [
    'skiff-compiler-projection',
    [
      'skiff-compiler-core',
      'skiff-compiler-projection-input',
      'skiff-artifact-model',
      'skiff-artifact-identity',
    ],
  ],
  [
    'skiff-compiler-emission',
    [
      'skiff-compiler-core',
      'skiff-compiler-projection',
      'skiff-artifact-model',
      'skiff-artifact-identity',
    ],
  ],
  [facadePackage, compilerSubCrates],
]);

const temporaryDependencyExceptions = [];

const cliOptions = parseCliOptions(process.argv.slice(2));

if (cliOptions.help) {
  printUsage();
} else if (cliOptions.selfTest) {
  runSelfTests();
} else {
  const metadata = await readCargoMetadata();
  const result = checkCompilerCrateDag(metadata, {
    phase: cliOptions.phase,
    exceptions: temporaryDependencyExceptions,
  });
  printCheckResult(result);
  if (result.failures.length > 0) {
    process.exitCode = 1;
  }
}

function checkCompilerCrateDag(metadata, options = {}) {
  const phase = options.phase ?? 0;
  const exceptions = options.exceptions ?? [];
  const result = {
    phase,
    failures: [],
    notes: [],
    checkedEdges: [],
    usedExceptions: [],
  };

  validateExceptionRegistry(exceptions, result.failures);

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

  for (const packageName of compilerSubCrates) {
    if (!workspacePackagesByName.has(packageName)) {
      result.notes.push(`skipped future compiler package not present in workspace: ${packageName}`);
    }
  }

  if (metadata.resolve === null || metadata.resolve === undefined || !Array.isArray(metadata.resolve.nodes)) {
    result.failures.push('cargo metadata resolve graph is missing; run cargo metadata without --no-deps');
    return result;
  }

  const resolveNodeById = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
  const activeExceptions = exceptions.filter((exception) => exceptionAppliesToPhase(exception, phase));

  for (const packageName of targetPackageNames) {
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
        checkEdge(edge, phase, activeExceptions, result);
      }
    }
  }

  return result;
}

function checkEdge(edge, phase, activeExceptions, result) {
  if (isAllowedProductionEdge(edge.package, edge.dependency, phase)) {
    return;
  }

  if (edge.dependency_kind !== 'normal') {
    const matchingException = activeExceptions.find((exception) => exceptionMatchesEdge(exception, edge));
    if (matchingException !== undefined) {
      result.usedExceptions.push({ ...matchingException, dependency_key: edge.dependency_key });
      return;
    }
  }

  result.failures.push(formatDisallowedEdge(edge, phase));
}

function isAllowedProductionEdge(packageName, dependencyName, phase) {
  const allowedDependencies = finalProductionEdges.get(packageName);
  if (allowedDependencies === undefined) {
    return false;
  }
  if (
    packageName === 'skiff-compiler-compiled'
    && dependencyName === 'skiff-compiler-projection-input'
    && phase < 7.5
  ) {
    return false;
  }
  if (allowedDependencies.includes(dependencyName)) {
    return true;
  }
  if (phase >= 0 && phase <= 9 && packageName === facadePackage) {
    return facadeMigrationShellDependencies().has(dependencyName);
  }
  return false;
}

function facadeMigrationShellDependencies() {
  return new Set([...compilerSubCrates, ...artifactPackages]);
}

function resolvedDependencyKinds(resolvedDependency) {
  const depKinds = resolvedDependency.dep_kinds;
  if (!Array.isArray(depKinds) || depKinds.length === 0) {
    return ['normal'];
  }
  return unique(depKinds.map((depKind) => normalizeDependencyKind(depKind.kind)));
}

function validateExceptionRegistry(exceptions, failures) {
  for (const [index, exception] of exceptions.entries()) {
    const label = `temporary dependency exception #${index + 1}`;
    for (const key of ['package', 'dependency', 'dependency_kind', 'reason', 'remove_when']) {
      if (exception[key] === undefined || exception[key] === '') {
        failures.push(`${label} is missing ${key}`);
      }
    }
    if (exception.phase === undefined && exception.issue === undefined) {
      failures.push(`${label} must include phase or issue`);
    }
    const dependencyKind = normalizeDependencyKind(exception.dependency_kind);
    if (!['normal', 'build', 'dev'].includes(dependencyKind)) {
      failures.push(`${label} has unsupported dependency_kind ${exception.dependency_kind}`);
    }
    if (dependencyKind === 'normal') {
      failures.push(`${label} uses dependency_kind normal; normal dependency exceptions are not allowed`);
    }
  }
}

function exceptionMatchesEdge(exception, edge) {
  return (
    exception.package === edge.package
    && exception.dependency === edge.dependency
    && normalizeDependencyKind(exception.dependency_kind) === edge.dependency_kind
  );
}

function exceptionAppliesToPhase(exception, phase) {
  if (exception.phase === undefined) {
    return true;
  }
  if (Array.isArray(exception.phase)) {
    return exception.phase.includes(phase);
  }
  return Number(exception.phase) === phase;
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
  for (const exception of result.usedExceptions) {
    console.log(
      [
        `NOTE temporary ${exception.dependency_kind} exception used:`,
        `${exception.package} -> ${exception.dependency}`,
        `remove_when=${JSON.stringify(exception.remove_when)}`,
        `reason=${JSON.stringify(exception.reason)}`,
      ].join(' '),
    );
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

function readCargoMetadata() {
  return new Promise((resolve, reject) => {
    const child = spawn('cargo', ['metadata', '--format-version', '1'], {
      cwd: root,
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
    child.on('exit', (code, signal) => {
      if (code !== 0) {
        reject(new Error([
          `cargo metadata --format-version 1 failed with ${signal ?? code}`,
          stderr.trim(),
        ].filter(Boolean).join('\n')));
        return;
      }
      try {
        resolve(JSON.parse(stdout));
      } catch (error) {
        reject(new Error(`failed to parse cargo metadata JSON: ${error.message}`));
      }
    });
  });
}

function runSelfTests() {
  const tests = [
    {
      name: 'renamed dependency key uses resolved package name',
      run: () => {
        const metadata = fixtureMetadata({
          packages: ['skiff-compiler-core', 'skiff-syntax'],
          edges: [
            {
              package: 'skiff-compiler-core',
              dependency: 'skiff-syntax',
              dependency_key: 'syntax_renamed',
              dependency_kind: 'normal',
            },
          ],
        });
        assertPass(checkCompilerCrateDag(metadata), 'renamed dependency should pass by resolved package name');
      },
    },
    {
      name: 'temporary dev and build exceptions are accepted',
      run: () => {
        const metadata = fixtureMetadata({
          packages: ['skiff-compiler-projection', 'skiff-syntax', 'skiff-compiler-input'],
          edges: [
            {
              package: 'skiff-compiler-projection',
              dependency: 'skiff-syntax',
              dependency_kind: 'dev',
            },
            {
              package: 'skiff-compiler-projection',
              dependency: 'skiff-compiler-input',
              dependency_kind: 'build',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata, {
          exceptions: [
            {
              package: 'skiff-compiler-projection',
              dependency: 'skiff-syntax',
              dependency_kind: 'dev',
              phase: 0,
              reason: 'test fixture needs parser syntax during split',
              remove_when: 'projection fixtures use projection-input DTOs only',
            },
            {
              package: 'skiff-compiler-projection',
              dependency: 'skiff-compiler-input',
              dependency_kind: 'build',
              phase: 0,
              reason: 'build fixture validates temporary generated inputs',
              remove_when: 'generated inputs move behind projection-input fixture data',
            },
          ],
        });
        assertPass(result, 'temporary dev/build exceptions should pass');
        assertEqual(result.usedExceptions.length, 2, 'temporary dev/build exceptions should be used');
      },
    },
    {
      name: 'normal dependency exceptions are rejected',
      run: () => {
        const metadata = fixtureMetadata({
          packages: ['skiff-compiler-projection', 'skiff-syntax'],
          edges: [
            {
              package: 'skiff-compiler-projection',
              dependency: 'skiff-syntax',
              dependency_kind: 'normal',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata, {
          exceptions: [
            {
              package: 'skiff-compiler-projection',
              dependency: 'skiff-syntax',
              dependency_kind: 'normal',
              phase: 0,
              reason: 'fixture attempts to exempt a production edge',
              remove_when: 'never',
            },
          ],
        });
        assertFail(result, 'normal dependency exceptions must fail');
        assertIncludes(result.failures.join('\n'), 'normal dependency exceptions are not allowed');
      },
    },
    {
      name: 'future packages missing emit skipped notes only',
      run: () => {
        const metadata = fixtureMetadata({
          packages: ['skiff-compiler', 'skiff-artifact-model', 'skiff-artifact-identity'],
          edges: [
            {
              package: 'skiff-compiler',
              dependency: 'skiff-artifact-model',
              dependency_kind: 'normal',
            },
            {
              package: 'skiff-artifact-identity',
              dependency: 'skiff-artifact-model',
              dependency_kind: 'normal',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata);
        assertPass(result, 'missing future packages should not fail');
        assertIncludes(result.notes.join('\n'), 'skipped future compiler package not present in workspace: skiff-syntax');
      },
    },
    {
      name: 'sub-crate cannot depend back on facade',
      run: () => {
        const metadata = fixtureMetadata({
          packages: ['skiff-compiler-core', 'skiff-compiler'],
          edges: [
            {
              package: 'skiff-compiler-core',
              dependency: 'skiff-compiler',
              dependency_kind: 'normal',
            },
          ],
        });
        const result = checkCompilerCrateDag(metadata);
        assertFail(result, 'sub-crate dependency back on facade must fail');
        assertIncludes(result.failures.join('\n'), 'skiff-compiler-core has disallowed normal dependency on skiff-compiler');
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
