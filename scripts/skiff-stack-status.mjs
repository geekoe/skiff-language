#!/usr/bin/env node
// `skiff stack status` CLI: ssh + /__router/health cross-check.

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { parseStackConfigDirArg } from './lib/stack-config.mjs';
import { stackStatus } from './lib/stack-status.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const skiffRoot = resolve(scriptDir, '..');

const { configDir } = parseStackConfigDirArg(process.argv.slice(2));
const result = await stackStatus({ configDir, skiffRoot });
console.log(JSON.stringify(result, null, 2));
