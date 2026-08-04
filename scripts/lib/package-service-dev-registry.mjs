import { resolve } from 'node:path';

import {
  classifyAuthoringRoot,
  readDevRegistry,
  writeDevRegistry,
} from '../skiff-dev-sync.mjs';

export async function runDevRegistryCommand(rawArgs, {
  defaultConfig,
  stdout = console.log,
} = {}) {
  const action = rawArgs[0];
  if (!['list', 'add', 'remove'].includes(action)) {
    throw new Error(`unknown dev registry command ${action || '(missing)'}; expected list, add, or remove`);
  }
  const parsed = parseArgs(action, rawArgs.slice(1), defaultConfig);
  const registry = await readDevRegistry(parsed.config, { allowMissing: true });
  if (action !== 'add' && parsed.profile !== undefined) {
    throw new Error(`dev registry ${action} does not accept --profile`);
  }
  if (parsed.profile !== undefined) {
    registry.profile = parsed.profile;
  }

  if (action === 'list') {
    if (registry.roots.length === 0) {
      stdout(`no authoring roots registered in ${parsed.config}`);
      return registry;
    }
    stdout(`authoring roots for ${registry.profile} in ${parsed.config}:`);
    for (const entry of registry.roots) {
      stdout(
        entry.kind === 'service'
          ? `- service ${entry.serviceId} at ${entry.root}`
          : `- package ${entry.root}`,
      );
    }
    stdout(
      'only listed roots are watched; add every locally developed package dependency explicitly',
    );
    return registry;
  }

  if (action === 'add') {
    const target = resolve(parsed.root);
    const entry = await classifyAuthoringRoot(target);
    registry.roots = registry.roots.filter(({ root }) => root !== target);
    registry.roots.push(entry);
    await writeDevRegistry(parsed.config, registry);
    stdout(`registered ${entry.kind} root ${entry.root}`);
    return registry;
  }

  const matches = matchRegistryRemovalTarget(registry.roots, parsed.root);
  if (matches.length === 0) {
    throw new Error(
      `no registered authoring root or service ID matched ${parsed.root}`,
    );
  }
  const [removed] = matches;
  registry.roots = registry.roots.filter(({ root }) => root !== removed.root);
  await writeDevRegistry(parsed.config, registry);
  stdout(
    `removed ${removed.kind} root ${removed.root}`
    + (removed.serviceId === undefined ? '' : ` (${removed.serviceId})`),
  );
  return registry;
}

export function matchRegistryRemovalTarget(
  roots,
  target,
  {
    resolveTarget = resolve,
  } = {},
) {
  const resolvedTarget = resolveTarget(target);
  const matches = roots.filter(
    ({ root, serviceId }) => root === resolvedTarget || serviceId === target,
  );
  if (matches.length > 1) {
    throw new Error(
      `registry remove target ${target} is ambiguous across ${matches
        .map(({ root, serviceId }) => `${serviceId ?? '(package)'} at ${root}`)
        .join(', ')}`,
    );
  }
  return matches;
}

function parseArgs(action, rawArgs, defaultConfig) {
  let root;
  let config = resolve(defaultConfig);
  let profile;
  for (let index = 0; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    const equals = argument.indexOf('=');
    const option = equals === -1 ? argument : argument.slice(0, equals);
    if (option === '--config' || option === '--profile') {
      const value = equals === -1 ? rawArgs[index + 1] : argument.slice(equals + 1);
      if (!value || value.startsWith('--')) {
        throw new Error(`${option} requires a value`);
      }
      if (option === '--config') {
        config = resolve(value);
      } else {
        profile = value;
      }
      if (equals === -1) {
        index += 1;
      }
      continue;
    }
    if (argument.startsWith('-')) {
      throw new Error(`unknown option ${argument}`);
    }
    if (root !== undefined) {
      throw new Error(`unexpected argument ${argument}`);
    }
    root = argument;
  }
  if (action === 'list' && root !== undefined) {
    throw new Error('dev registry list does not accept a root');
  }
  if (action !== 'list' && root === undefined) {
    throw new Error(`dev registry ${action} requires a root`);
  }
  return { root, config, profile };
}
