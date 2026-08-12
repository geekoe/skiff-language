import assert from 'node:assert/strict';
import test from 'node:test';

import {
  parseTestSummary,
  phase0WorkloadSpecs,
} from '../lib/bytecode-vm-phase-0-contract.mjs';

const ROOT = '/candidate';

test('canonical workload fixes all eight exact commands', () => {
  const specs = phase0WorkloadSpecs(ROOT);
  assert.equal(specs.length, 8);
  assert.deepEqual(specs[0].args, [
    '--test', '--test-reporter=tap', 'scripts/tests/bytecode-vm-phase-0-gate-*.test.mjs',
  ]);
  assert.match(specs[1].args.join(' '), /phase_0_vcp_production_composition -- --exact/);
  assert.match(specs[2].args.join(' '), /phase_0_negative_production_boundaries -- --exact/);
  assert.equal(specs.slice(1).every(({ testFormat }) => testFormat === 'rust-exact'), true);
});

test('TAP summary requires declared positive total and every test passed', () => {
  assert.equal(parseTestSummary('node-tap', tap()).valid, true);
  assert.equal(parseTestSummary('node-tap', tap({ total: 0, passed: 0 })).valid, false);
  assert.equal(parseTestSummary('node-tap', tap({ total: 2, passed: 1, skipped: 1 })).valid, false);
  assert.equal(parseTestSummary('node-tap', tap({ total: 2, passed: 1, todo: 1 })).valid, false);
  assert.equal(parseTestSummary('node-tap', tap({ total: 2, passed: 1, cancelled: 1 })).valid, false);
  assert.equal(parseTestSummary('node-tap', tap({ total: 2, passed: 1, failed: 1 })).valid, false);
  assert.equal(parseTestSummary('node-tap', tap().replace('# pass 2', '# pass 1')).valid, false);
});

test('Rust summary requires one exact pass while allowing filtered tests', () => {
  assert.equal(parseTestSummary('rust-exact', rust()).valid, true);
  assert.equal(parseTestSummary('rust-exact', rust({ passed: 0 })).valid, false);
  assert.equal(parseTestSummary('rust-exact', rust({ ignored: 1 })).valid, false);
  assert.equal(parseTestSummary('rust-exact', `${rust()}${rust()}`).valid, false);
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
