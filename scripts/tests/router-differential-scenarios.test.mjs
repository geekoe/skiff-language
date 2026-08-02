import assert from 'node:assert/strict';
import test from 'node:test';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  assertSelectedScenarioRunnable,
  loadScenarioInventory,
} from '../lib/router-differential/scenarios.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

test('checked-in differential inventory is complete and consistent', async () => {
  const inventory = await loadScenarioInventory({ skiffRoot: repoRoot });
  assert.equal(
    inventory.baseline,
    'd228b613eafeba5e2275bf830f5770f21b931e81',
  );
  assert.ok(inventory.scenarios.length >= 1);
  const ids = new Set(inventory.scenarios.map((scenario) => scenario.id));
  assert.equal(ids.size, inventory.scenarios.length);

  const runnable = inventory.scenarios.filter((scenario) => scenario.status === 'runnable');
  assert.deepEqual(
    runnable.map((scenario) => scenario.id),
    ['session-handshake-basic'],
  );
  for (const scenario of inventory.scenarios) {
    assert.ok(scenario.observationTypes.length > 0);
    assert.ok(scenario.compare !== undefined);
    if (scenario.status === 'planned') {
      assert.ok(
        typeof scenario.blockedOn === 'string' && scenario.blockedOn.length > 0,
        `planned scenario ${scenario.id} must declare blockedOn`,
      );
    }
  }
});

test('planned scenarios cannot execute', async () => {
  const inventory = await loadScenarioInventory({ skiffRoot: repoRoot });
  const planned = inventory.scenarios.find((scenario) => scenario.status === 'planned');
  assert.ok(planned !== undefined);
  assert.throws(
    () => assertSelectedScenarioRunnable(planned),
    /only runnable scenarios can execute/,
  );
});
