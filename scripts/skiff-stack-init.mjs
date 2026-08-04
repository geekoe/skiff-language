#!/usr/bin/env node
// `skiff stack init` CLI: formal production bootstrap for a fresh host.
//
// Authors an empty RuntimeAssembly + profile RuntimeConfigSnapshot + canonical
// std records + actor routing projection through the compiler/config-snapshot
// authoring libraries, materializes them to the remote artifact root, seeds
// Mongo `skiff-router.activation_state` (generation 0), and starts the router.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { initStack } from './lib/stack-init.mjs';
import { parseStackConfigDirArg } from './lib/stack-config.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');

const { configDir } = parseStackConfigDirArg(process.argv.slice(2));
const result = await initStack({ configDir, skiffRoot });
console.log(JSON.stringify(result, null, 2));
