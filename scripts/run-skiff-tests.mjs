#!/usr/bin/env node

import { runCanonicalSkiffSourceTests } from './lib/skiff-source-test-suite.mjs';

try {
  const plan = await runCanonicalSkiffSourceTests();
  console.log(`[skiff-tests] passed ${plan.length} canonical source test entr${plan.length === 1 ? 'y' : 'ies'}`);
} catch (error) {
  console.error(`error: ${error?.message || String(error)}`);
  process.exitCode = 1;
}
