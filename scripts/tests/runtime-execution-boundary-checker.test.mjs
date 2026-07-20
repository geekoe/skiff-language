import assert from 'node:assert/strict';
import test from 'node:test';

import {
  RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS,
  runRuntimeExecutionBoundarySelfTest,
} from '../lib/runtime-execution-boundary-self-test.mjs';
import {
  RUNTIME_EXECUTION_BOUNDARY_REGISTRY,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS,
} from '../lib/runtime-execution-boundary-subjects.mjs';

test('runtime execution boundary checker rejects every hermetic mutation', async () => {
  const matrix = await runRuntimeExecutionBoundarySelfTest();
  assert.deepEqual(matrix, RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS);
  assert.equal(matrix.length, 28);
  assert.deepEqual(
    new Set(matrix.map(({ expectedId }) => expectedId)),
    new Set([
      'remote-boundary-selection',
      'required-owner-missing',
      'owner-outside-registered-root',
      'duplicate-required-owner',
      'dispatcher-callsite-missing',
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
      'required-owner-anchor-missing',
      'legacy-outbound-service-edge',
      'router-service-relay',
      'router-service-rejection-incomplete',
      'router-rejection-enters-relay-owner',
    ]),
  );
});

test('production registry names every required subject and owner role exactly once', () => {
  assert.deepEqual(
    RUNTIME_EXECUTION_BOUNDARY_REGISTRY.subjects.map(({ id }) => id).sort(),
    [...REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS].sort(),
  );
  assert.deepEqual(
    RUNTIME_EXECUTION_BOUNDARY_REGISTRY.owners.map(({ role }) => role).sort(),
    [...REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES].sort(),
  );
  assert.deepEqual(
    RUNTIME_EXECUTION_BOUNDARY_REGISTRY.owners.map(
      ({ role, symbol, requiredFile = null }) => ({ requiredFile, role, symbol }),
    ),
    [
      owner(
        'service-dispatcher',
        'dispatch_in_process_boundary',
        'runtime/eval/src/assembly_execution/mod.rs',
      ),
      owner(
        'internal-service-call-adapter',
        'dispatch_service_call',
        'runtime/eval/src/assembly_execution/mod.rs',
      ),
      owner(
        'ingress-service-call-adapter',
        'dispatch_ingress_via_in_process_boundary',
        'runtime/eval/src/assembly_execution/ingress.rs',
      ),
      owner(
        'legacy-service-path-fence',
        'ensure_legacy_service_path_allowed',
        'runtime/eval/src/eval_context.rs',
      ),
      owner('activation-context', 'ActivationContext'),
      owner('request-generation', 'RequestLifecycle'),
      owner('callback-table', 'CallbackCapabilityTable'),
      owner('callback-carrier', 'CallbackCapabilityCarrier'),
      owner('owned-context-carrier', 'OwnedProgramExecutionContext'),
      owner(
        'active-assembly-context-set',
        'ActiveAssemblyContextSet',
        'runtime/host/src/loader/active_assembly_context.rs',
      ),
      owner(
        'active-assembly-route',
        'ActiveAssemblyRoute',
        'runtime/host/src/loader/assembly_admission.rs',
      ),
      owner(
        'host-request-entry',
        'spawn_request_inner',
        'runtime/host/src/host/request_entry/assembly.rs',
      ),
      owner(
        'assembly-request-spawn',
        'spawn_assembly_request',
        'runtime/host/src/host/request_entry/assembly.rs',
      ),
      owner(
        'recoverable-callback-encoder',
        'RecoverableBoundaryCodec',
        'runtime/boundary/src/recoverable.rs',
      ),
      owner(
        'router-runtime-service-rejection',
        'handleBinaryMessage',
        'router/src/router/runtimeEndpoint.ts',
      ),
    ],
  );
});

function owner(role, symbol, requiredFile = null) {
  return { requiredFile, role, symbol };
}
