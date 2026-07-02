import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

export function defaultDevHome(env = process.env) {
  const home = env.HOME || env.USERPROFILE || homedir();
  return join(home, '.skiff', 'dev');
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

export function identityCliBinaryName(platform = process.platform) {
  return platform === 'win32' ? 'skiff-artifact-identity.exe' : 'skiff-artifact-identity';
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
    identityCli: join(runtimeBinDir, identityCliBinaryName(platform)),
    runtimeBinary: join(runtimeBinDir, runtimeBinaryName(platform)),
    routerConfig: join(resolvedDevHome, 'router.yml'),
    telemetryConfig: join(resolvedDevHome, 'telemetry.yml'),
    watchConfig: join(resolvedDevHome, 'watch.json'),
  };
}
