import assert from 'node:assert/strict';
import test from 'node:test';

import {
  assertPhase1LaneCoverage,
  parsePhase1TestSummary,
  PHASE1_COMMAND_SCHEMA,
  PHASE1_MANIFEST_SCHEMA,
  PHASE1_REQUIRED_LANES,
  phase1CandidateSpecs,
  phase1WorkloadSpecs,
} from '../lib/bytecode-vm-phase-1-contract.mjs';
import {
  PHASE1_DIRECTORY_IDENTITY_FILE,
  PHASE1_DIRECTORY_IDENTITY_SCHEMA,
} from '../lib/bytecode-vm-phase-1-evidence-root.mjs';

const ROOT = '/candidate';

test('Phase 1 schemas are independent from the accepted Phase 0 epoch', () => {
  assert.equal(PHASE1_COMMAND_SCHEMA, 'skiff-bytecode-vm-phase-1-command-v1');
  assert.equal(PHASE1_MANIFEST_SCHEMA, 'skiff-bytecode-vm-phase-1-gate-v1');
  assert.doesNotMatch(PHASE1_MANIFEST_SCHEMA, /phase-0/);
  assert.equal(PHASE1_DIRECTORY_IDENTITY_SCHEMA,
    'skiff-bytecode-vm-phase-1-directory-identity-v1');
  assert.equal(PHASE1_DIRECTORY_IDENTITY_FILE, 'phase-1-directory-identities.json');
});

test('day-one matrix freezes eight commands and every required Proof lane', () => {
  const specs = phase1WorkloadSpecs(ROOT);
  assert.equal(specs.length, 8);
  assert.doesNotThrow(() => assertPhase1LaneCoverage(specs));
  assert.deepEqual(
    [...new Set(specs.flatMap(({ lanes }) => lanes))]
      .filter((lane) => PHASE1_REQUIRED_LANES.includes(lane))
      .sort(),
    [...PHASE1_REQUIRED_LANES].sort(),
  );
  assert.deepEqual(specs[0].args, [
    '--test', '--test-reporter=tap',
    'scripts/tests/bytecode-vm-phase-0-gate-*.test.mjs',
    'scripts/tests/bytecode-vm-phase-1-gate-*.test.mjs',
  ]);
  assert.deepEqual(specs[1].args, [
    'test', '-p', 'skiff-compiler', '-p', 'skiff-compiler-emission', '--lib',
    'phase_1_bytecode_admission',
  ]);
  assert.equal(specs[1].testFormat, 'rust-suite-2');
  assert.equal(specs[2].id, 'k0b-tc-production-contract');
  assert.deepEqual(specs[2].lanes, ['K0B', 'T-C']);
  assert.equal(specs[4].id, 'tr-v1-production-proof');
  assert.deepEqual(specs[4].lanes, ['T-R', 'V1']);
  assert.equal(specs.filter(({ lanes }) => lanes.includes('phase-0-regression')).length, 4);
  assert.equal(specs.some(({ args }) => args.includes('scripts/verify.mjs')), false);
  assert.equal(
    specs.slice(5).filter(({ lanes }) => lanes.includes('phase-0-regression'))
      .every(({ command, testFormat }) => command === 'cargo' && testFormat === 'rust-exact'),
    true,
  );
});

test('candidate closure fixes four receipt-backed identity snapshots', () => {
  const specs = phase1CandidateSpecs(ROOT);
  assert.equal(specs.length, 12);
  assert.deepEqual(specs.slice(-3).map(({ id }) => id), [
    'fresh-head', 'fresh-tree', 'fresh-status',
  ]);
});

test('test summaries reject zero, skip, todo, cancel, ignore, and imprecise exact runs', () => {
  assert.equal(parsePhase1TestSummary('node-tap', tap()).valid, true);
  assert.equal(parsePhase1TestSummary('node-tap', tap({ total: 0, passed: 0 })).valid, false);
  assert.equal(parsePhase1TestSummary('node-tap', tap({ passed: 1, skipped: 1 })).valid, false);
  assert.equal(parsePhase1TestSummary('node-tap', tap({ passed: 1, todo: 1 })).valid, false);
  assert.equal(parsePhase1TestSummary('node-tap', tap({ passed: 1, cancelled: 1 })).valid, false);
  assert.equal(parsePhase1TestSummary('rust-suite', rust({ passed: 3 })).valid, true);
  assert.equal(parsePhase1TestSummary('rust-suite', rust({ passed: 0 })).valid, false);
  assert.equal(parsePhase1TestSummary('rust-suite', rust({ ignored: 1 })).valid, false);
  assert.equal(parsePhase1TestSummary('rust-exact', rust({ passed: 1 })).valid, true);
  assert.equal(parsePhase1TestSummary('rust-exact', rust({ passed: 2 })).valid, false);
  const twoGreen = `${rust({ passed: 3 })}${rust({ passed: 4, filtered: 7 })}`;
  assert.deepEqual(parsePhase1TestSummary('rust-suite-2', twoGreen), {
    format: 'rust',
    summaries: 2,
    total: 7,
    passed: 7,
    failed: 0,
    ignored: 0,
    measured: 0,
    filtered: 49,
    valid: true,
  });
  assert.equal(parsePhase1TestSummary('rust-suite-2', rust({ passed: 3 })).valid, false);
  assert.equal(
    parsePhase1TestSummary('rust-suite-2',
      `${rust({ passed: 3 })}${rust({ passed: 0 })}`).valid,
    false,
  );
});

function tap({ total = 2, passed = total, failed = 0, cancelled = 0, skipped = 0, todo = 0 } = {}) {
  return [
    'TAP version 13', `1..${total}`, `# tests ${total}`, `# pass ${passed}`,
    `# fail ${failed}`, `# cancelled ${cancelled}`, `# skipped ${skipped}`,
    `# todo ${todo}`, '',
  ].join('\n');
}

function rust({ passed = 1, failed = 0, ignored = 0, measured = 0, filtered = 42 } = {}) {
  return `test result: ok. ${passed} passed; ${failed} failed; ${ignored} ignored; ${measured} measured; ${filtered} filtered out; finished in 0.01s\n`;
}
