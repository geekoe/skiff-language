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
    'edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a',
  );
  assert.ok(inventory.scenarios.length >= 1);
  const ids = new Set(inventory.scenarios.map((scenario) => scenario.id));
  assert.equal(ids.size, inventory.scenarios.length);

  const runnable = inventory.scenarios.filter((scenario) => scenario.status === 'runnable');
  assert.deepEqual(
    runnable.map((scenario) => scenario.id),
    [
      'session-handshake-basic',
      'differential_ext_http_unary',
      'differential_ext_http_stream',
      'differential_ext_http_error',
      'differential_ext_http_cors',
      'differential_ext_ws_generation',
      'differential_ext_ws_replacement',
      'differential_ext_ws_id_lexical',
      'differential_ext_actor_call',
      'differential_ext_actor_control',
    ],
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
