#!/usr/bin/env node
// `skiff stack validate` CLI: local parse/consistency validation of configDir.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { loadStackConfig, parseStackConfigDirArg } from './lib/stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');

const { configDir } = parseStackConfigDirArg(process.argv.slice(2));
const stack = await loadStackConfig(configDir, { skiffRoot });
console.log(JSON.stringify({
  ok: true,
  configDir,
  profile: stack.config.profile,
  remote: stack.config.remote,
  verify: stack.config.verify,
  build: {
    target: stack.build.target,
    buildRoot: stack.paths.buildRoot,
    cargoTargetDir: stack.paths.cargoTargetDir,
    units: stack.build.units,
  },
  files: {
    build: true,
    config: true,
    router: true,
    runtime: true,
  },
}, null, 2));
