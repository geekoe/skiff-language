#!/usr/bin/env node
// `skiff stack deploy` CLI (stage D copy mode).
//
// The three YAML files are copied verbatim from --configDir; no YAML is
// rendered or augmented here. Binaries come from the stack build manifest
// (build.yml buildRoot). Remote facts come from config.yml.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { deployStack } from './lib/stack-deploy.mjs';
import { parseStackConfigDirArg } from './lib/stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');

const { configDir } = parseStackConfigDirArg(process.argv.slice(2));
const result = await deployStack({ configDir, skiffRoot });
console.log(JSON.stringify(result, null, 2));
