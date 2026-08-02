import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptLibDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptLibDir, '..', '..');

export function defaultDevHome(env = process.env) {
  return join(skiffRoot, '.skiff-instance', 'dev-home');
}

export function resolveDevHome(value, env = process.env) {
  if (value) {
    const trimmed = value.trim();
    if (trimmed.length > 0) {
      return resolve(trimmed);
    }
  }
  return resolve(defaultDevHome(env));
}

export function runtimeBinaryName(platform = process.platform) {
  return platform === 'win32' ? 'skiff-runtime.exe' : 'skiff-runtime';
}

export function routerBinaryName(platform = process.platform) {
  return platform === 'win32' ? 'skiff-router.exe' : 'skiff-router';
}

export function ecosystemStoreCliBinaryName(platform = process.platform) {
  return platform === 'win32' ? 'skiff-compiler.exe' : 'skiff-compiler';
}

export function devRuntimePaths({ devHome, env = process.env, platform = process.platform } = {}) {
  const resolvedDevHome = resolveDevHome(devHome ?? env.SKIFF_DEV_HOME, env);
  const runtimeBinDir = join(resolvedDevHome, 'bin');
  return {
    devHome: resolvedDevHome,
    artifactRoot: join(resolvedDevHome, 'artifacts'),
    serviceBuildRoot: join(resolvedDevHome, 'build'),
    runtimeConfig: join(resolvedDevHome, 'runtime.yml'),
    runtimeHome: join(resolvedDevHome, 'runtime-home'),
    runtimeBinDir,
    ecosystemStoreCli: join(runtimeBinDir, ecosystemStoreCliBinaryName(platform)),
    runtimeBinary: join(runtimeBinDir, runtimeBinaryName(platform)),
    routerBinary: join(runtimeBinDir, routerBinaryName(platform)),
    routerConfig: join(resolvedDevHome, 'router.yml'),
    telemetryConfig: join(resolvedDevHome, 'telemetry.yml'),
    watchConfig: join(resolvedDevHome, 'watch.json'),
  };
}

export function resolveRouterProcessSpec({
  devHome,
  implementation,
  repoRoot = skiffRoot,
  platform = process.platform,
} = {}) {
  if (devHome === undefined || devHome === null || String(devHome).trim().length === 0) {
    throw new Error('RouterProcessSpec requires an explicit devHome');
  }
  assertRouterImplementation(implementation);
  const resolvedDevHome = resolve(devHome);
  const resolvedRepoRoot = resolve(repoRoot);
  const spec = {
    implementation,
    config_path: join(resolvedDevHome, 'router.yml'),
  };
  if (implementation === 'ts') {
    spec.ts_source_root = join(resolvedRepoRoot, 'router');
  } else {
    spec.rust_binary_path = join(resolvedDevHome, 'bin', routerBinaryName(platform));
  }
  return deepFreeze(spec);
}

export function assertRouterImplementation(value) {
  if (value !== 'ts' && value !== 'rust') {
    throw new Error('router implementation must be exactly "ts" or "rust"');
  }
  return value;
}

export function assertRouterProcessSpec(spec) {
  if (!spec || typeof spec !== 'object' || Array.isArray(spec)) {
    throw new Error('RouterProcessSpec must be an object');
  }
  assertRouterImplementation(spec.implementation);
  if (!isAbsolutePath(spec.config_path)) {
    throw new Error('RouterProcessSpec.config_path must be an absolute path');
  }
  const expectedKeys = spec.implementation === 'ts'
    ? ['implementation', 'config_path', 'ts_source_root']
    : ['implementation', 'config_path', 'rust_binary_path'];
  const actualKeys = Object.keys(spec).sort();
  if (actualKeys.join(',') !== [...expectedKeys].sort().join(',')) {
    throw new Error(
      `RouterProcessSpec must contain exactly ${expectedKeys.join(', ')}`,
    );
  }
  const sourceKey = spec.implementation === 'ts'
    ? 'ts_source_root'
    : 'rust_binary_path';
  if (!isAbsolutePath(spec[sourceKey])) {
    throw new Error(`RouterProcessSpec.${sourceKey} must be an absolute path`);
  }
  return spec;
}

export function routerProcessInvocation(spec) {
  assertRouterProcessSpec(spec);
  return Object.freeze(
    spec.implementation === 'ts'
      ? {
          command: 'pnpm',
          args: Object.freeze([
            '--dir',
            spec.ts_source_root,
            'dev',
            '--config',
            spec.config_path,
          ]),
        }
      : {
          command: spec.rust_binary_path,
          args: Object.freeze([spec.config_path]),
        },
  );
}

function isAbsolutePath(value) {
  return typeof value === 'string' && value.trim().length > 0 && resolve(value) === value;
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
