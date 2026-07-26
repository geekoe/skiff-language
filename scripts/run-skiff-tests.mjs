#!/usr/bin/env node

import { runCanonicalSkiffSourceTests } from './lib/skiff-source-test-suite.mjs';
import { renderIsolatedRuntimeLogEvidence } from './lib/isolated-test-runtime-log-evidence.mjs';

try {
  const plan = await runCanonicalSkiffSourceTests();
  console.log(`[skiff-tests] passed ${plan.length} canonical source test entr${plan.length === 1 ? 'y' : 'ies'}`);
} catch (error) {
  const message = error?.message || String(error);
  const evidence = renderIsolatedRuntimeLogEvidence(error);
  console.error(`error: ${message}${evidence.length === 0 ? '' : `\n${evidence}`}`);
  process.exitCode = 1;
}
