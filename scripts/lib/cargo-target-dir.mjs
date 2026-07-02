import { resolve } from 'node:path';

export function defaultCargoTargetDir(skiffRoot) {
  return resolve(skiffRoot, 'build', 'cargo-target');
}

export function cargoTargetDir(skiffRoot, env = process.env) {
  return resolve(env.CARGO_TARGET_DIR || defaultCargoTargetDir(skiffRoot));
}

export function cargoBuildEnv(skiffRoot, env = process.env) {
  return {
    ...env,
    CARGO_TARGET_DIR: cargoTargetDir(skiffRoot, env),
  };
}
