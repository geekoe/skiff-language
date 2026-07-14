#!/usr/bin/env node

import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { runCratePublicApiCli } from './lib/crate-public-api-cli.mjs';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const directEntry = process.argv[1]
  ? pathToFileURL(resolve(process.argv[1])).href === import.meta.url
  : false;

if (directEntry) {
  try {
    process.exitCode = await runCratePublicApiCli({
      argv: process.argv.slice(2),
      env: process.env,
      root,
      stderr: process.stderr,
      stdout: process.stdout,
    });
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
