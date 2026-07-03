export const devSyncCheckFlags = Object.freeze(['--check', '--check-sync']);

export const serviceDevSyncOptions = Object.freeze([
  '--root',
  '--profile',
  '--artifact-root',
  '--reload-url',
  '--config',
  '--service-id',
  '--packages-dir',
  '--service-artifact-root',
]);

export const serviceDevWatchOptions = Object.freeze([
  '--root',
  '--profile',
  '--artifact-root',
  '--reload-url',
  '--config',
  '--service-id',
  '--poll-interval-ms',
  '--packages-dir',
  '--service-artifact-root',
]);

export const instanceDevSyncOptions = Object.freeze([
  '--root',
  '--profile',
  '--service-id',
  '--poll-interval-ms',
  '--packages-dir',
  '--default-packages-dir',
  '--service-artifact-root',
]);

const repeatableDevSyncOptions = new Set(['--packages-dir', '--default-packages-dir', '--service-artifact-root']);
const forwardOptionOrder = Object.freeze([
  ['profile', '--profile'],
  ['artifactRoot', '--artifact-root'],
  ['buildRoot', '--build-root'],
  ['reloadUrl', '--reload-url'],
  ['config', '--config'],
  ['serviceId', '--service-id'],
  ['pollIntervalMs', '--poll-interval-ms'],
]);

export function parseDevSyncArgs(rawArgs, spec) {
  const allowedFlags = new Set(spec.flags ?? []);
  const allowedOptions = new Set(spec.options ?? []);
  const flags = new Set();
  const options = {};
  let root;

  for (let index = 0; index < rawArgs.length; index += 1) {
    const arg = rawArgs[index];
    if (allowedFlags.has(arg)) {
      flags.add(arg);
      continue;
    }

    const equalsIndex = arg.indexOf('=');
    const optionName = equalsIndex === -1 ? arg : arg.slice(0, equalsIndex);
    if (allowedOptions.has(optionName)) {
      const value = equalsIndex === -1
        ? requireNext(rawArgs, index, optionName)
        : arg.slice(equalsIndex + 1);
      if (value.length === 0 && spec.allowEmptyEquals !== true) {
        throw new Error(`${optionName} requires a value`);
      }
      if (value.startsWith('--') && spec.allowDashEquals !== true) {
        throw new Error(`${optionName} requires a value`);
      }
      if (optionName === '--root') {
        root = resolveRoot(value, spec);
      } else {
        const key = toCamelOption(optionName);
        if (repeatableDevSyncOptions.has(optionName)) {
          options[key] ??= [];
          options[key].push(value);
        } else {
          options[key] = value;
        }
      }
      if (equalsIndex === -1) {
        index += 1;
      }
      continue;
    }

    if (arg.startsWith('-')) {
      throw new Error(`unknown option ${arg}`);
    }
    if (root !== undefined) {
      throw new Error(`unexpected argument ${arg}`);
    }
    root = resolveRoot(arg, spec);
  }

  if (spec.requireRoot && root === undefined) {
    throw new Error('missing root path');
  }
  return { flags, options, root };
}

export function renderDevSyncArgs(parsed, options = {}) {
  return [
    ...(options.prefix ?? []),
    ...renderDevSyncFlags(parsed.flags),
    ...renderDevSyncRoot(parsed.root),
    ...renderDevSyncOptions(options.injectOptions ?? {}),
    ...renderDevSyncOptions(parsed.options ?? {}),
  ];
}

export function renderDevSyncFlags(flags) {
  const selected = flags instanceof Set ? flags : new Set(flags ?? []);
  return devSyncCheckFlags.filter((flag) => selected.has(flag));
}

export function renderDevSyncRoot(root) {
  return root === undefined ? [] : ['--root', root];
}

export function renderDevSyncOptions(options) {
  const result = [];
  for (const [key, option] of forwardOptionOrder) {
    if (options[key] !== undefined) {
      result.push(option, options[key]);
    }
  }
  for (const packageDir of options.packagesDir ?? []) {
    result.push('--packages-dir', packageDir);
  }
  for (const packageDir of options.defaultPackagesDir ?? []) {
    result.push('--default-packages-dir', packageDir);
  }
  for (const serviceArtifactRoot of options.serviceArtifactRoot ?? []) {
    result.push('--service-artifact-root', serviceArtifactRoot);
  }
  return result;
}

function resolveRoot(value, spec) {
  return spec.resolveRoot === false ? value : spec.resolve(value);
}

function requireNext(args, index, optionName) {
  const value = args[index + 1];
  if (value === undefined || value.startsWith('--')) {
    throw new Error(`${optionName} requires a value`);
  }
  return value;
}

function toCamelOption(optionName) {
  return optionName.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
}
