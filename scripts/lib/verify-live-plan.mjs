import { accessSync, constants as fsConstants, existsSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { delimiter, isAbsolute, join, resolve } from 'node:path';

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
import { parseRuntimeReloadUrl } from './runtime-reload-url.mjs';

const DISCOVERY_HANDLERS = Object.freeze({
  [LIVE_DISCOVERIES.RUNTIME_LIVE_TESTS]: discoverRuntimeLiveTests,
});

export async function liveSelectorPhases(root, selector, {
  runtimeLiveConfig,
  runtimeLiveReloadUrl,
  runtimeLiveArtifactRoot,
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
    runtimeLiveConfig,
    runtimeLiveReloadUrl,
    runtimeLiveArtifactRoot,
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
    return [blockedInvocationPhase(root, invocation, blockers.join('; '))];
  }

  if (invocation.plan === LIVE_PLAN_TYPES.RUNTIME_FIXTURES) {
    return runtimeFixturePhases(root, invocation, runtimeState, env);
  }
  if (invocation.plan === LIVE_PLAN_TYPES.FIXED_COMMAND) {
    return [fixedCommandPhase(root, entry, invocation, inputState.values, loopRiskState, env)];
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

export function assertRegistryPhaseMetadata(phase) {
  const hasOwnership = phase.ownership !== undefined;
  const hasTier = phase.tier !== undefined;
  if (!hasOwnership && !hasTier) {
    return;
  }
  if (!hasOwnership || !hasTier) {
    throw new Error(`registry phase requires ownership and tier metadata: ${phase.id}`);
  }
  assertOwnershipTier(phase.ownership, phase.tier, `phase ${phase.id}`);
  if (phase.kind !== phase.tier) {
    throw new Error(`registry phase kind must match tier: ${phase.id}`);
  }
}

async function inspectRuntimeFixtureState(root, entry, values) {
  const failures = [];
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

  let configPath;
  if (values.runtimeConfig !== undefined) {
    configPath = resolveInputPath(root, values.runtimeConfig);
    if (!isFile(configPath)) {
      failures.push(`runtime-live config path must be an existing file: ${configPath}`);
    }
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

  let reloadTarget;
  if (values.runtimeReloadUrl !== undefined) {
    try {
      reloadTarget = parseRuntimeReloadUrl(values.runtimeReloadUrl);
    } catch (error) {
      failures.push(error instanceof Error ? error.message : String(error));
    }
  }

  if (failures.length > 0) {
    throw new Error(failures.join('; '));
  }
  return {
    artifactRoot,
    configPath,
    files,
    reloadTarget,
  };
}

function runtimeFixturePhases(root, invocation, runtimeState, env) {
  const {
    artifactRoot,
    configPath,
    files,
    reloadTarget,
  } = runtimeState;
  const packageStore = join(root, 'runtime', 'live-tests', 'package-store');
  const packageArgs = existsSync(packageStore) ? ['--packages-dir', packageStore] : [];
  const executionPreflight = () => {
    const failures = [];
    for (const file of files) {
      if (!isFile(file)) {
        failures.push(`runtime-live fixture is no longer an existing file: ${file}`);
      }
    }
    if (!isFile(configPath)) {
      failures.push(`runtime-live config path is no longer an existing file: ${configPath}`);
    }
    if (!isDirectory(artifactRoot)) {
      failures.push(
        `runtime-live artifact root is no longer an existing directory: ${artifactRoot}`,
      );
    }
    try {
      parseRuntimeReloadUrl(reloadTarget.normalized);
    } catch (error) {
      failures.push(
        error instanceof Error
          ? error.message
          : 'runtime-live reload URL failed execution preflight validation',
      );
    }
    failures.push(...executionExecutableFailures(invocation, env, root));
    return failures.length === 0 ? undefined : failures;
  };

  return files.map((file) => {
    const args = [
      'run',
      '--manifest-path',
      'test-runner/Cargo.toml',
      '--',
      file,
      '--live',
      '--allow-network',
      '--config',
      configPath,
      '--router-reload-url',
      reloadTarget.normalized,
      '--artifact-root',
      artifactRoot,
      ...(invocation.canonicalPolicy.forbidSkips ? ['--deny-skips'] : []),
      ...(invocation.canonicalPolicy.forbidUnchecked ? ['--require-tests'] : []),
      ...packageArgs,
    ];
    const displayArgs = [...args];
    displayArgs[displayArgs.indexOf('--router-reload-url') + 1] = reloadTarget.display;
    return executableInvocationPhase(root, invocation, {
      id: `${invocation.idPrefix}${repoRelative(root, file)}`,
      command: 'cargo',
      args,
      displayArgs,
      executionPreflight,
    });
  });
}

function fixedCommandPhase(root, entry, invocation, inputValues, loopRiskState, env) {
  const scriptPath = resolve(root, entry.source.path);
  const resolvedInputValues = {
    ...inputValues,
    ...(loopRiskState === undefined
      ? {}
      : { loopRiskConfig: loopRiskState.configPath }),
  };
  const inputArgs = Object.entries(invocation.inputArgs ?? {})
    .flatMap(([input, option]) => [option, resolvedInputValues[input]]);
  return executableInvocationPhase(root, invocation, {
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

function executableInvocationPhase(root, invocation, execution) {
  return {
    ...execution,
    kind: invocation.tier,
    tier: invocation.tier,
    ownership: invocation.ownership,
    cwd: root,
  };
}

function blockedInvocationPhase(root, invocation, preconditionError) {
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
