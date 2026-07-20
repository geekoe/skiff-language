export const REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS = Object.freeze([
  'single-service-dispatcher',
  'activation-request-callback-ownership',
  'owned-context-user-code-spawn',
  'required-host-request-entry',
  'recoverable-callback-rejection',
  'router-runtime-service-rejection',
]);

export const REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES = Object.freeze([
  'service-dispatcher',
  'activation-context',
  'request-generation',
  'callback-table',
  'callback-carrier',
  'owned-context-carrier',
  'host-request-entry',
  'recoverable-callback-encoder',
  'router-runtime-service-rejection',
]);

export const PROPOSED_RUNTIME_EXECUTION_BOUNDARY_REGISTRY =
  defineRuntimeExecutionBoundaryRegistry({
    sourceRoots: [
      { id: 'runtime-rust', language: 'rust', root: 'runtime' },
      { id: 'router-typescript', language: 'typescript', root: 'router/src' },
    ],
    subjects: [
      {
        id: 'single-service-dispatcher',
        language: 'rust',
        discoveryRoots: ['runtime/eval/src', 'runtime/host/src', 'runtime/request/src'],
        zones: {
          canonicalCallers: [
            'runtime/eval/src/eval_context.rs',
            'runtime/host/src/host/request_entry.rs',
          ],
        },
      },
      {
        id: 'activation-request-callback-ownership',
        language: 'rust',
        discoveryRoots: [
          'runtime/activation/src',
          'runtime/model/src',
          'runtime/linked-program/src',
          'runtime/boundary/src',
        ],
        zones: {
          sharedCode: ['runtime/linked-program/src'],
          packageBuildCaches: ['runtime'],
          callbackCarrier: ['runtime/model/src/value.rs'],
        },
      },
      {
        id: 'owned-context-user-code-spawn',
        language: 'rust',
        discoveryRoots: ['runtime/eval/src', 'runtime/request/src', 'runtime/host/src'],
        zones: {
          userCodeSpawn: ['runtime/eval/src', 'runtime/request/src', 'runtime/host/src'],
        },
      },
      {
        id: 'required-host-request-entry',
        language: 'rust',
        discoveryRoots: ['runtime/host/src/host/request_entry.rs'],
        requiredFiles: ['runtime/host/src/host/request_entry.rs'],
      },
      {
        id: 'recoverable-callback-rejection',
        language: 'rust',
        discoveryRoots: [
          'runtime/boundary/src/recoverable.rs',
          'runtime/boundary/src/persistent.rs',
        ],
        requiredFiles: ['runtime/boundary/src/recoverable.rs'],
      },
      {
        id: 'router-runtime-service-rejection',
        language: 'typescript',
        discoveryRoots: ['router/src'],
        requiredFiles: [
          'router/src/router/runtimeDispatcher.ts',
          'router/src/router/runtimeEndpoint.ts',
        ],
      },
    ],
    owners: [
      {
        role: 'service-dispatcher',
        subjectId: 'single-service-dispatcher',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'dispatch_service_call',
        ownedRoots: ['runtime/eval/src/assembly_execution/mod.rs'],
        requiredAnchors: ['resolve_service_call'],
      },
      {
        role: 'activation-context',
        subjectId: 'activation-request-callback-ownership',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'ActivationContext',
        ownedRoots: ['runtime/activation/src/context.rs'],
      },
      {
        role: 'request-generation',
        subjectId: 'activation-request-callback-ownership',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'RequestLifecycle',
        ownedRoots: ['runtime/activation/src/request_context.rs'],
      },
      {
        role: 'callback-table',
        subjectId: 'activation-request-callback-ownership',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'CallbackCapabilityTable',
        ownedRoots: ['runtime/activation/src/capability.rs'],
      },
      {
        role: 'callback-carrier',
        subjectId: 'activation-request-callback-ownership',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'CallbackCapabilityCarrier',
        ownedRoots: ['runtime/model/src/value.rs'],
      },
      {
        role: 'owned-context-carrier',
        subjectId: 'owned-context-user-code-spawn',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'OwnedProgramExecutionContext',
        ownedRoots: ['runtime/eval/src/program_execution.rs'],
      },
      {
        role: 'host-request-entry',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'spawn_request_inner',
        ownedRoots: ['runtime/host/src/host/request_entry.rs'],
        requiredFile: 'runtime/host/src/host/request_entry.rs',
        requiredAnchors: ['lookup_active_assembly_request_route', 'dispatch_service_call'],
      },
      {
        role: 'recoverable-callback-encoder',
        subjectId: 'recoverable-callback-rejection',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'RecoverableBoundaryCodec',
        ownedRoots: ['runtime/boundary/src/recoverable.rs'],
        requiredFile: 'runtime/boundary/src/recoverable.rs',
      },
      {
        role: 'router-runtime-service-rejection',
        subjectId: 'router-runtime-service-rejection',
        language: 'typescript',
        declarationKind: 'method',
        symbol: 'rejectRuntimeServiceRequestStart',
        ownedRoots: ['router/src/router/runtimeDispatcher.ts'],
        requiredFile: 'router/src/router/runtimeDispatcher.ts',
        requiredAnchors: ['RemoteServiceRelayDisabled', 'in-process binding'],
      },
    ],
  });

export function defineRuntimeExecutionBoundaryRegistry(registry) {
  return Object.freeze({
    ...registry,
    sourceRoots: Object.freeze(
      (registry.sourceRoots ?? []).map((entry) => Object.freeze({ ...entry })),
    ),
    subjects: Object.freeze(
      (registry.subjects ?? []).map((entry) => Object.freeze({
        ...entry,
        discoveryRoots: frozenArray(entry.discoveryRoots),
        requiredFiles: frozenArray(entry.requiredFiles),
        zones: Object.freeze(
          Object.fromEntries(
            Object.entries(entry.zones ?? {}).map(([name, roots]) => [name, frozenArray(roots)]),
          ),
        ),
      })),
    ),
    owners: Object.freeze(
      (registry.owners ?? []).map((entry) => Object.freeze({
        ...entry,
        ownedRoots: frozenArray(entry.ownedRoots),
        requiredAnchors: frozenArray(entry.requiredAnchors),
      })),
    ),
  });
}

function frozenArray(value) {
  return Object.freeze([...(value ?? [])]);
}
