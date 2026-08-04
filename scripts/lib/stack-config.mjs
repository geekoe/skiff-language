import { access, readFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';

import { parseDocument } from 'yaml';

export const STACK_CONFIG_FILES = [
  'build.yml',
  'config.yml',
  'router.yml',
  'runtime.yml',
  'telemetry.yml',
];

export const PROFILE_TOKEN_PATTERN = /^[A-Za-z0-9._-]{1,200}$/;

export const KNOWN_BUILD_UNITS = new Set([
  'artifact-model',
  'artifact-identity',
  'compiler',
  'runtime',
  'router',
  'telemetry',
]);

export function assertProfileToken(profile, label) {
  if (
    typeof profile !== 'string'
    || profile === '.'
    || profile === '..'
    || !PROFILE_TOKEN_PATTERN.test(profile)
  ) {
    throw new Error(
      `${label} must be a canonical ASCII profile token ([A-Za-z0-9._-]{1,200}, rejecting "." and "..")`,
    );
  }
  return profile;
}

export function parseStackYaml(source, label) {
  const document = parseDocument(source, {
    uniqueKeys: true,
    merge: false,
    schema: 'core',
    prettyErrors: false,
  });
  const problems = [...document.errors, ...document.warnings];
  if (problems.length > 0) {
    throw new Error(`${label} YAML parse error: ${problems[0].message}`);
  }
  const value = document.toJS({ maxAliasCount: -1 });
  if (!isPlainObject(value)) {
    throw new Error(`${label} root must be an object`);
  }
  return value;
}

export async function readStackYamlFile(file, label) {
  let source;
  try {
    source = await readFile(file, 'utf8');
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(`${label} is required at ${file}`);
    }
    throw new Error(`failed to read ${label} at ${file}: ${error.message}`, { cause: error });
  }
  return parseStackYaml(source, label);
}

export function parseStackConfigDirArg(rawArgs, { options = [] } = {}) {
  const allowed = new Set(['--configDir', ...options]);
  let configDir;
  const extra = {};
  for (let index = 0; index < rawArgs.length; index += 1) {
    const argument = rawArgs[index];
    let value;
    if (argument === '--configDir') {
      value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error('--configDir requires a directory path');
      }
      index += 1;
    } else if (argument.startsWith('--configDir=')) {
      value = argument.slice('--configDir='.length);
    } else if (allowed.has(argument)) {
      value = rawArgs[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${argument} requires a value`);
      }
      index += 1;
      const key = argument.slice(2);
      if (extra[key] !== undefined) {
        throw new Error(`${argument} was provided more than once`);
      }
      extra[key] = value;
    } else {
      throw new Error(`unknown stack option ${argument}`);
    }
    if (argument === '--configDir' || argument.startsWith('--configDir=')) {
      if (configDir !== undefined) {
        throw new Error('--configDir was provided more than once');
      }
      configDir = value;
    }
  }
  if (configDir === undefined) {
    throw new Error('stack command requires --configDir <dir>');
  }
  return { configDir: resolve(configDir), ...extra };
}

export async function loadStackConfig(configDir, {
  skiffRoot,
  files = STACK_CONFIG_FILES,
} = {}) {
  if (typeof skiffRoot !== 'string' || !isAbsolute(skiffRoot)) {
    throw new Error('loadStackConfig requires an absolute skiffRoot');
  }
  const resolvedConfigDir = resolve(configDir);
  try {
    await access(resolvedConfigDir);
  } catch (error) {
    throw new Error(`stack configDir must exist: ${resolvedConfigDir}`);
  }

  const parsed = {};
  for (const file of files) {
    if (!STACK_CONFIG_FILES.includes(file)) {
      throw new Error(`unknown stack config file ${file}`);
    }
    const label = file.replace(/\.yml$/, '');
    parsed[file] = await readStackYamlFile(join(resolvedConfigDir, file), `${label}.yml`);
  }

  const build = parsed['build.yml'] === undefined ? null : validateBuildConfig(parsed['build.yml']);
  const config = parsed['config.yml'] === undefined ? null : validateStackConfig(parsed['config.yml']);
  const router = parsed['router.yml'] === undefined ? null : validateRouterConfig(parsed['router.yml']);
  const runtime = parsed['runtime.yml'] === undefined ? null : parsed['runtime.yml'];
  const telemetry = parsed['telemetry.yml'] === undefined ? null : parsed['telemetry.yml'];

  if (config !== null && router !== null) {
    if (config.profile !== router.profile) {
      throw new Error(
        `stack profile mismatch: config.yml.profile=${JSON.stringify(config.profile)} but router.yml.profile=${JSON.stringify(router.profile)}; deploy/init/validate fail closed`,
      );
    }
  }

  return {
    configDir: resolvedConfigDir,
    build,
    config,
    router,
    runtime,
    telemetry,
    paths: build === null ? null : {
      buildRoot: resolveStackPath(build.buildRoot, skiffRoot, 'build.yml buildRoot'),
      cargoTargetDir: resolveStackPath(
        build.cargoTargetDir,
        skiffRoot,
        'build.yml cargoTargetDir',
      ),
    },
  };
}

function validateBuildConfig(value) {
  const label = 'build.yml';
  return {
    target: readRequiredString(value, 'target', label),
    zigDir: readRequiredString(value, 'zigDir', label),
    buildRoot: readRequiredString(value, 'buildRoot', label),
    cargoTargetDir: readRequiredString(value, 'cargoTargetDir', label),
    units: readOptionalUnits(value.units, label),
    profile: readOptionalProfile(value.profile, label),
    process: normalizeBuildProcess(value.process, label),
  };
}

function readOptionalProfile(value, label) {
  if (value === undefined) {
    return 'release';
  }
  if (value !== 'debug' && value !== 'release') {
    throw new Error(`${label} profile must be "debug" or "release"`);
  }
  return value;
}

function normalizeBuildProcess(value, label) {
  if (value === undefined) {
    return {
      mongo: 'disabled',
      telemetry: 'managed',
      mongoBinary: 'mongod',
      mongoDbPath: undefined,
      devHome: undefined,
    };
  }
  if (!isPlainObject(value)) {
    throw new Error(`${label} process must be an object`);
  }
  const mongo = value.mongo === undefined
    ? 'disabled'
    : readOptionalManagedFlag(value.mongo, `${label} process.mongo`);
  const telemetry = value.telemetry === undefined
    ? 'managed'
    : readOptionalManagedFlag(value.telemetry, `${label} process.telemetry`);
  const mongoBinary = value.mongoBinary === undefined
    ? 'mongod'
    : readRequiredString(value, 'mongoBinary', `${label} process`);
  const mongoDbPath = value.mongoDbPath === undefined
    ? undefined
    : readRequiredString(value, 'mongoDbPath', `${label} process`);
  const devHome = value.devHome === undefined
    ? undefined
    : readRequiredString(value, 'devHome', `${label} process`);
  return { mongo, telemetry, mongoBinary, mongoDbPath, devHome };
}

function readOptionalManagedFlag(value, label) {
  if (value !== 'managed' && value !== 'disabled') {
    throw new Error(`${label} must be "managed" or "disabled"`);
  }
  return value;
}

function validateStackConfig(value) {
  const label = 'config.yml';
  const profile = assertProfileToken(
    readRequiredString(value, 'profile', label),
    `${label} profile`,
  );
  const rawRemote = readOptionalObject(value, 'remote', label);
  const remote = rawRemote === undefined
    ? undefined
    : {
        host: readRequiredString(rawRemote, 'host', `${label} remote`),
        remoteSkiff: readRequiredPosixPath(
          readRequiredString(rawRemote, 'remoteSkiff', `${label} remote`),
          `${label} remote.remoteSkiff`,
        ),
        nodeBin: readRequiredPosixPath(
          readRequiredString(rawRemote, 'nodeBin', `${label} remote`),
          `${label} remote.nodeBin`,
        ),
        serviceDbKeyringFile: rawRemote.serviceDbKeyringFile === undefined
          ? undefined
          : readRequiredPosixPath(
              readRequiredString(rawRemote, 'serviceDbKeyringFile', `${label} remote`),
              `${label} remote.serviceDbKeyringFile`,
            ),
      };
  const rawVerify = readOptionalObject(value, 'verify', label);
  const verify = rawVerify === undefined
    ? undefined
    : {
        httpPort: readRequiredPort(rawVerify, 'httpPort', `${label} verify`),
        controlPort: readRequiredPort(rawVerify, 'controlPort', `${label} verify`),
        telemetryPort: readRequiredPort(rawVerify, 'telemetryPort', `${label} verify`),
        healthPath: readRequiredHealthPath(rawVerify, 'healthPath', `${label} verify`),
      };
  return {
    profile,
    remote,
    verify,
  };
}

export function requireRemoteStackConfig(stack, command) {
  if (stack.config?.remote === undefined || stack.config?.verify === undefined) {
    throw new Error(`${command} requires config.yml remote and verify`);
  }
  return stack;
}

function validateRouterConfig(value) {
  const label = 'router.yml';
  return {
    ...value,
    profile: assertProfileToken(
      readRequiredString(value, 'profile', label),
      `${label} profile`,
    ),
  };
}

function readRequiredString(value, field, label) {
  const fieldValue = value?.[field];
  if (typeof fieldValue !== 'string' || fieldValue.trim().length === 0) {
    throw new Error(`${label} ${field} is required`);
  }
  return fieldValue;
}

function readRequiredObject(value, field, label) {
  const fieldValue = value?.[field];
  if (!isPlainObject(fieldValue)) {
    throw new Error(`${label} ${field} must be an object`);
  }
  return fieldValue;
}

function readOptionalObject(value, field, label) {
  const fieldValue = value?.[field];
  if (fieldValue === undefined) {
    return undefined;
  }
  if (!isPlainObject(fieldValue)) {
    throw new Error(`${label} ${field} must be an object`);
  }
  return fieldValue;
}

function readRequiredPosixPath(value, label) {
  if (!value.startsWith('/')) {
    throw new Error(`${label} must be an absolute path on the remote runtime host`);
  }
  return value;
}

function readRequiredPort(value, field, label) {
  const fieldValue = value?.[field];
  if (
    !Number.isSafeInteger(fieldValue)
    || fieldValue <= 0
    || fieldValue > 65535
  ) {
    throw new Error(`${label} ${field} must be a TCP port`);
  }
  return fieldValue;
}

function readRequiredHealthPath(value, field, label) {
  const fieldValue = readRequiredString(value, field, label);
  if (!fieldValue.startsWith('/')) {
    throw new Error(`${label} ${field} must start with /`);
  }
  return fieldValue;
}

function readOptionalUnits(value, label) {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error(`${label} units must be a non-empty array`);
  }
  const units = [];
  for (const unit of value) {
    if (typeof unit !== 'string' || !KNOWN_BUILD_UNITS.has(unit)) {
      throw new Error(
        `${label} units must be one of ${[...KNOWN_BUILD_UNITS].join(', ')}; got ${JSON.stringify(unit)}`,
      );
    }
    if (!units.includes(unit)) {
      units.push(unit);
    }
  }
  return units;
}

function resolveStackPath(value, skiffRoot, label) {
  return isAbsolute(value) ? value : resolve(skiffRoot, value);
}

export function posixShellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`;
}

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
