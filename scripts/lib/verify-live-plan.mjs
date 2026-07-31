import { accessSync, constants as fsConstants, existsSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import {
  delimiter,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from 'node:path';

import { discoverRuntimeLiveTests, repoRelative } from './verify-discovery.mjs';
import {
  LOOP_RISK_CONFIG_PROFILES,
  assertReadableRuntimeLogs,
  loadLoopRiskConfig,
} from './loop-risk-config.mjs';
import {
  LIVE_DISCOVERIES,
  LIVE_INPUTS,
  LIVE_PLAN_TYPES,
  LIVE_REGISTRY,
  assertLiveRegistryIntegrity,
  assertOwnershipTier,
  liveInvocationRecords,
} from './verify-live-registry.mjs';
import { maxExpectedAssemblyGeneration } from './package-service-authoring.mjs';

const DISCOVERY_HANDLERS = Object.freeze({
  [LIVE_DISCOVERIES.RUNTIME_LIVE_TESTS]: discoverRuntimeLiveTests,
});

export async function liveSelectorTasks(root, selector, {
  runtimeLiveActivationUrl,
  runtimeLiveIngressUrl,
  runtimeLiveArtifactRoot,
  runtimeLiveEnvironment,
  runtimeLiveExpectedGeneration,
  loopRiskConfig,
  env = process.env,
  registry = LIVE_REGISTRY,
} = {}) {
  assertLiveRegistryIntegrity(registry);
  const matches = liveInvocationRecords(registry)
    .filter(({ invocation }) => invocation.selector === selector);
  if (matches.length !== 1) {
    throw new Error(
      `live selector ${selector} must resolve to exactly one invocation, found ${matches.length}`,
    );
  }
  const { entry, invocation } = matches[0];
  assertSelectedSourceExists(root, entry);

  const inputState = resolveRequiredInputs(invocation, {
    runtimeLiveActivationUrl,
    runtimeLiveIngressUrl,
    runtimeLiveArtifactRoot,
    runtimeLiveEnvironment,
    runtimeLiveExpectedGeneration,
    loopRiskConfig,
  }, env);
  const loopRiskState = invocation.configProfile !== undefined
    && inputState.values.loopRiskConfig !== undefined
    ? await inspectLoopRiskConfigState(root, invocation, inputState.values.loopRiskConfig)
    : undefined;
  const runtimeState = invocation.plan === LIVE_PLAN_TYPES.RUNTIME_FIXTURES
    ? await inspectRuntimeFixtureState(root, entry, inputState.values)
    : undefined;
  const missingExecutables = missingRequiredExecutables(invocation, env, root);
  const missingModules = missingRequiredModules(invocation, root);
  const blockers = [];
  if (inputState.missing.length > 0) {
    blockers.push(
      `${selector} is missing required explicit input(s): ${inputState.missing.join('; ')}`,
    );
  }
  if (missingExecutables.length > 0) {
    blockers.push(
      `${selector} is missing required executable(s) on PATH: ${missingExecutables.join(', ')}`,
    );
  }
  if (missingModules.length > 0) {
    blockers.push(
      `${selector} is missing required module(s): ${missingModules.join(', ')}`,
    );
  }
  if (blockers.length > 0) {
    return [blockedInvocationTask(root, invocation, blockers.join('; '))];
  }

  if (invocation.plan === LIVE_PLAN_TYPES.RUNTIME_FIXTURES) {
    return runtimeFixtureTasks(root, invocation, runtimeState, env);
  }
  if (invocation.plan === LIVE_PLAN_TYPES.FIXED_COMMAND) {
    return [fixedCommandTask(root, entry, invocation, inputState.values, loopRiskState, env)];
  }
  throw new Error(`unsupported live plan type ${invocation.plan}`);
}

async function inspectLoopRiskConfigState(root, invocation, configuredPath) {
  const configPath = resolveInputPath(root, configuredPath);
  const profile = invocation.configProfile === LOOP_RISK_CONFIG_PROFILES.STRESS
    ? LOOP_RISK_CONFIG_PROFILES.STRESS
    : LOOP_RISK_CONFIG_PROFILES.HEALTH;
  const config = await loadLoopRiskConfig(configPath, {
    profile,
    checkLogFiles: profile === LOOP_RISK_CONFIG_PROFILES.STRESS,
  });
  return { config, configPath, profile };
}

export function assertRegistryTaskMetadata(task) {
  const hasOwnership = task.ownership !== undefined;
  const hasTier = task.tier !== undefined;
  if (!hasOwnership && !hasTier) {
    return;
  }
  if (!hasOwnership || !hasTier) {
    throw new Error(`registry task requires ownership and tier metadata: ${task.id}`);
  }
  assertOwnershipTier(task.ownership, task.tier, `task ${task.id}`);
  if (task.kind !== task.tier) {
    throw new Error(`registry task kind must match tier: ${task.id}`);
  }
}

async function inspectRuntimeFixtureState(root, entry, values) {
  const failures = [];
  const sourceRoot = resolve(root, 'runtime', 'live-tests');
  const discover = DISCOVERY_HANDLERS[entry.source.discovery];
  if (discover === undefined) {
    throw new Error(`unsupported live discovery handler: ${entry.source.discovery}`);
  }
  const files = await discover(root);
  if (files.length === 0) {
    failures.push(
      'runtime-live found no *.live.test.skiff fixtures under runtime/live-tests',
    );
  }
  const fixtures = files.map((file) => ({
    file,
    packageRoot: canonicalPackageRoot(root, file),
  }));
  if (!isFile(join(sourceRoot, 'package.yml'))) {
    failures.push(
      `runtime-live canonical source root must own package.yml: ${sourceRoot}`,
    );
  }
  if (!isFile(join(sourceRoot, 'config.skiff-test.yml'))) {
    failures.push(
      `runtime-live canonical source root must own fixed config.skiff-test.yml: ${sourceRoot}`,
    );
  }
  const legacyFiles = fixtures
    .filter((fixture) => fixture.packageRoot === undefined)
    .map((fixture) => repoRelative(root, fixture.file));
  if (legacyFiles.length > 0) {
    failures.push(
      `runtime-live fixture(s) have no canonical package.yml owner and require terminal canonical-harness migration: ${legacyFiles.join(', ')}`,
    );
  }
  const nonCanonicalFiles = fixtures
    .filter((fixture) =>
      fixture.packageRoot !== undefined && fixture.packageRoot !== sourceRoot)
    .map((fixture) => repoRelative(root, fixture.file));
  if (nonCanonicalFiles.length > 0) {
    failures.push(
      `runtime-live fixture(s) must be owned by the canonical runtime/live-tests package root: ${nonCanonicalFiles.join(', ')}`,
    );
  }

  let artifactRoot;
  if (values.runtimeArtifactRoot !== undefined) {
    artifactRoot = resolveInputPath(root, values.runtimeArtifactRoot);
    if (!isDirectory(artifactRoot)) {
      failures.push(
        `runtime-live artifact root must be an existing directory: ${artifactRoot}`,
      );
    }
  }

  let activationUrl;
  if (values.runtimeActivationUrl !== undefined) {
    try {
      activationUrl = canonicalRuntimeUrl(
        values.runtimeActivationUrl,
        '/__skiff/activate-assembly',
      );
    } catch (error) {
      failures.push(error instanceof Error ? error.message : String(error));
    }
  }
  let ingressUrl;
  if (values.runtimeIngressUrl !== undefined) {
    try {
      ingressUrl = canonicalRuntimeUrl(values.runtimeIngressUrl, '/');
    } catch (error) {
      failures.push(error instanceof Error ? error.message : String(error));
    }
  }
  if (values.runtimeEnvironment !== undefined && !/^[A-Za-z0-9._-]{1,200}$/.test(values.runtimeEnvironment)) {
    failures.push('runtime-live environment must be a canonical ASCII token');
  }
  let expectedGenerations;
  if (values.runtimeExpectedGeneration !== undefined) {
    try {
      expectedGenerations = runtimeExpectedGenerations(
        values.runtimeExpectedGeneration,
        fixtures.length,
      );
    } catch (error) {
      failures.push(error instanceof Error ? error.message : String(error));
    }
  }

  if (failures.length > 0) {
    throw new Error(failures.join('; '));
  }
  return {
    artifactRoot,
    activationUrl,
    ingressUrl,
    environment: values.runtimeEnvironment,
    expectedGenerations,
    fixtures,
    sourceRoot,
  };
}

function runtimeFixtureTasks(root, invocation, runtimeState, env) {
  const {
    artifactRoot,
    activationUrl,
    ingressUrl,
    environment,
    expectedGenerations,
    fixtures,
    sourceRoot,
  } = runtimeState;
  const executionPreflight = () => {
    const failures = [];
    for (const { file } of fixtures) {
      if (!isFile(file)) {
        failures.push(`runtime-live fixture is no longer an existing file: ${file}`);
      }
    }
    if (!isFile(join(sourceRoot, 'package.yml'))) {
      failures.push(`runtime-live package root is no longer canonical: ${sourceRoot}`);
    }
    if (!isFile(join(sourceRoot, 'config.skiff-test.yml'))) {
      failures.push(
        `runtime-live package root no longer owns fixed config.skiff-test.yml: ${sourceRoot}`,
      );
    }
    if (!isDirectory(artifactRoot)) {
      failures.push(
        `runtime-live artifact root is no longer an existing directory: ${artifactRoot}`,
      );
    }
    try {
      canonicalRuntimeUrl(activationUrl, '/__skiff/activate-assembly');
      canonicalRuntimeUrl(ingressUrl, '/');
    } catch (error) {
      failures.push(
        error instanceof Error
          ? error.message
          : 'runtime-live URL failed execution preflight validation',
      );
    }
    failures.push(...executionExecutableFailures(invocation, env, root));
    return failures.length === 0 ? undefined : failures;
  };

  return fixtures.map(({ file }, index) => {
    const args = [
      'run',
      '--manifest-path',
      'test-runner/Cargo.toml',
      '--',
      file,
      '--live',
      '--artifact-root',
      artifactRoot,
      ...runtimeLivePlatformSourceArgs(root),
      '--activation-url',
      activationUrl,
      '--ingress-url',
      ingressUrl,
      '--environment',
      environment,
      '--expected-generation',
      expectedGenerations[index],
      ...(invocation.canonicalPolicy.forbidSkips ? ['--deny-skips'] : []),
      ...(invocation.canonicalPolicy.forbidUnchecked ? ['--require-tests'] : []),
    ];
    return executableInvocationTask(root, invocation, {
      id: `${invocation.idPrefix}${repoRelative(root, file)}`,
      command: 'cargo',
      args,
      executionPreflight,
    });
  });
}

function runtimeExpectedGenerations(initialValue, fixtureCount) {
  if (!/^(?:0|[1-9][0-9]*)$/.test(initialValue)) {
    throw new Error('runtime-live expected generation must be a non-negative integer');
  }
  const initial = BigInt(initialValue);
  const maximum = BigInt(maxExpectedAssemblyGeneration);
  const last = initial + BigInt(Math.max(0, fixtureCount - 1));
  if (initial > maximum || last > maximum) {
    throw new Error(
      `runtime-live expected generation sequence ending at ${last} must not exceed ${maximum}`,
    );
  }
  return Array.from(
    { length: fixtureCount },
    (_, index) => (initial + BigInt(index)).toString(),
  );
}

export function runtimeLivePlatformSourceArgs(skiffRoot) {
  return ['--platform-source-root', resolve(skiffRoot)];
}

function canonicalPackageRoot(repositoryRoot, file) {
  const boundary = resolve(repositoryRoot);
  let current = dirname(resolve(file));
  while (true) {
    const fromBoundary = relative(boundary, current);
    if (
      fromBoundary === '..'
      || fromBoundary.startsWith(`..${sep}`)
      || isAbsolute(fromBoundary)
    ) {
      break;
    }
    if (isFile(join(current, 'package.yml'))) {
      return current;
    }
    if (current === boundary) {
      break;
    }
    current = dirname(current);
  }
  return undefined;
}

function canonicalRuntimeUrl(value, expectedPath) {
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new Error('runtime-live URL must be an absolute http:// URL');
  }
  if (
    url.protocol !== 'http:'
    || url.username !== ''
    || url.password !== ''
    || url.search !== ''
    || url.hash !== ''
    || url.pathname !== expectedPath
  ) {
    throw new Error(`runtime-live URL must point exactly to ${expectedPath}`);
  }
  return url.toString().replace(/\/$/, expectedPath === '/' ? '' : '');
}

function fixedCommandTask(root, entry, invocation, inputValues, loopRiskState, env) {
  const scriptPath = resolve(root, entry.source.path);
  const resolvedInputValues = {
    ...inputValues,
    ...(loopRiskState === undefined
      ? {}
      : { loopRiskConfig: loopRiskState.configPath }),
  };
  const inputArgs = Object.entries(invocation.inputArgs ?? {})
    .flatMap(([input, option]) => [option, resolvedInputValues[input]]);
  return executableInvocationTask(root, invocation, {
    id: invocation.id,
    command: 'node',
    args: [entry.source.path, ...invocation.args, ...inputArgs],
    executionPreflight: async () => {
      const failures = [];
      if (!isFile(scriptPath)) {
        failures.push(`live registry script is no longer an existing file: ${entry.source.path}`);
      }
      failures.push(...executionExecutableFailures(invocation, env, root));
      failures.push(...moduleRequirementFailures(invocation, root));
      if (loopRiskState !== undefined) {
        let currentConfig;
        try {
          currentConfig = await loadLoopRiskConfig(loopRiskState.configPath, {
            profile: loopRiskState.profile,
            checkLogFiles: false,
          });
        } catch (error) {
          failures.push(error instanceof Error ? error.message : String(error));
        }
        if (currentConfig?.stress !== undefined) {
          try {
            await assertReadableRuntimeLogs(currentConfig.stress.runtimeLogs);
          } catch (error) {
            failures.push(error instanceof Error ? error.message : String(error));
          }
          failures.push(...runtimePidFailures(currentConfig.stress.runtimePids));
        }
      }
      return failures.length === 0 ? undefined : failures;
    },
  });
}

function executableInvocationTask(root, invocation, execution) {
  return {
    ...execution,
    kind: invocation.tier,
    tier: invocation.tier,
    ownership: invocation.ownership,
    cwd: root,
  };
}

function blockedInvocationTask(root, invocation, preconditionError) {
  return {
    id: invocation.id ?? `${invocation.idPrefix}inputs`,
    kind: invocation.tier,
    tier: invocation.tier,
    ownership: invocation.ownership,
    cwd: root,
    preconditionError,
  };
}

function resolveRequiredInputs(invocation, configured, env) {
  const values = {};
  const missing = [];
  for (const inputName of invocation.requiredInputs) {
    const definition = LIVE_INPUTS[inputName];
    const value = nonEmptyValue(
      configured[definition.option],
      env[definition.environment],
    );
    if (value === undefined) {
      missing.push(definition.description);
    } else {
      values[inputName] = value;
    }
  }
  return { values, missing };
}

function missingRequiredExecutables(invocation, env, cwd) {
  return invocation.requiredExecutables.filter((executable) =>
    resolveExecutable(executable, env, cwd) === undefined);
}

function missingRequiredModules(invocation, root) {
  return invocation.requiredModules
    .filter((requirement) => !canResolveModule(requirement, root))
    .map((requirement) => `${requirement.specifier} from ${requirement.from}`);
}

function executionExecutableFailures(invocation, env, cwd) {
  const missing = missingRequiredExecutables(invocation, env, cwd);
  return missing.length === 0
    ? []
    : [
      `${invocation.selector} required executable(s) are no longer available on PATH: ${missing.join(', ')}`,
    ];
}

function moduleRequirementFailures(invocation, root) {
  const missing = missingRequiredModules(invocation, root);
  return missing.length === 0
    ? []
    : [
      `${invocation.selector} required module(s) are no longer resolvable: ${missing.join(', ')}`,
    ];
}

function canResolveModule(requirement, root) {
  try {
    createRequire(resolve(root, requirement.from)).resolve(requirement.specifier);
    return true;
  } catch {
    return false;
  }
}

function runtimePidFailures(runtimePids) {
  const failures = [];
  for (const pid of runtimePids) {
    try {
      process.kill(pid, 0);
    } catch {
      failures.push(`loop-risk runtime PID is not alive: ${pid}`);
    }
  }
  return failures;
}

function resolveExecutable(executable, env, cwd) {
  const pathValue = typeof env.PATH === 'string' ? env.PATH : '';
  for (const directory of pathValue.split(delimiter)) {
    const pathRoot = isAbsolute(directory)
      ? directory
      : resolve(cwd, directory || '.');
    const candidate = join(pathRoot, executable);
    try {
      if (!statSync(candidate).isFile()) {
        continue;
      }
      accessSync(candidate, fsConstants.X_OK);
      return candidate;
    } catch {
      // Continue to the next PATH entry.
    }
  }
  return undefined;
}

function assertSelectedSourceExists(root, entry) {
  if (entry.source.type !== 'script') {
    return;
  }
  const scriptPath = resolve(root, entry.source.path);
  if (!isFile(scriptPath)) {
    throw new Error(`live registry script must be an existing file: ${entry.source.path}`);
  }
}

function nonEmptyValue(configured, environment) {
  if (configured !== undefined) {
    return typeof configured === 'string' && configured.length > 0 ? configured : undefined;
  }
  return typeof environment === 'string' && environment.length > 0 ? environment : undefined;
}

function resolveInputPath(root, path) {
  return isAbsolute(path) ? path : resolve(root, path);
}

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function isDirectory(path) {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}
