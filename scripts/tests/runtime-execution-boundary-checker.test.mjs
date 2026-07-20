import assert from 'node:assert/strict';
import test from 'node:test';

import {
  RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS,
  runRuntimeExecutionBoundarySelfTest,
} from '../lib/runtime-execution-boundary-self-test.mjs';
import {
  PROPOSED_RUNTIME_EXECUTION_BOUNDARY_REGISTRY,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS,
} from '../lib/runtime-execution-boundary-subjects.mjs';

test('runtime execution boundary checker rejects every hermetic mutation', async () => {
  const matrix = await runRuntimeExecutionBoundarySelfTest();
  assert.deepEqual(matrix, RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS);
  assert.equal(matrix.length, 24);
  assert.deepEqual(
    new Set(matrix.map(({ expectedId }) => expectedId)),
    new Set([
      'remote-boundary-selection',
      'required-owner-missing',
      'owner-outside-registered-root',
      'duplicate-required-owner',
      'subject-registry-omission',
      'required-subject-file-missing',
      'owner-registry-omission',
      'required-owner-file-registry-omission',
      'owner-root-registry-omission',
      'forbidden-registry-exception-field',
      'current-service-task-local',
      'second-in-process-dispatcher',
      'shared-mutable-activation-owner',
      'package-build-mutable-owner-cache',
      'callback-carrier-native-address',
      'unowned-user-code-spawn',
      'recoverable-callback-not-rejected',
      'host-request-fallback',
      'legacy-outbound-service-edge',
      'router-service-relay',
      'router-rejection-enters-relay-owner',
    ]),
  );
});

test('proposed registry names every required subject and owner role exactly once', () => {
  assert.deepEqual(
    PROPOSED_RUNTIME_EXECUTION_BOUNDARY_REGISTRY.subjects.map(({ id }) => id).sort(),
    [...REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS].sort(),
  );
  assert.deepEqual(
    PROPOSED_RUNTIME_EXECUTION_BOUNDARY_REGISTRY.owners.map(({ role }) => role).sort(),
    [...REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES].sort(),
  );
});
