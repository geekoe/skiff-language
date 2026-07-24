#!/usr/bin/env node

import { realpath } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { runActorFullChainAcceptance } from './lib/actor-full-chain-acceptance-real.mjs';

const checkout = await realpath(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
const result = await runActorFullChainAcceptance({ checkout });
process.stdout.write(`${JSON.stringify(result)}\n`);
