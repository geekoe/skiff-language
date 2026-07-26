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
  'internal-service-call-adapter',
  'ingress-service-call-adapter',
  'legacy-service-path-fence',
  'activation-context',
  'request-generation',
  'callback-table',
  'callback-carrier',
  'owned-context-carrier',
  'active-assembly-context-set',
  'active-assembly-route',
  'host-request-entry',
  'assembly-request-spawn',
  'recoverable-callback-encoder',
  'router-runtime-service-rejection',
]);

export const RUNTIME_EXECUTION_BOUNDARY_REGISTRY =
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
            'runtime/eval/src/assembly_execution/mod.rs',
            'runtime/eval/src/assembly_execution/ingress.rs',
          ],
          legacyServiceEdges: ['runtime/eval/src/eval_context.rs'],
        },
        requiredFiles: [
          'runtime/eval/src/assembly_execution/mod.rs',
          'runtime/eval/src/assembly_execution/ingress.rs',
          'runtime/eval/src/eval_context.rs',
        ],
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
        discoveryRoots: [
          'runtime/host/src/host/request_entry/assembly.rs',
          'runtime/host/src/loader/active_assembly_context.rs',
          'runtime/host/src/loader/assembly_admission.rs',
        ],
        requiredFiles: [
          'runtime/host/src/host/request_entry/assembly.rs',
          'runtime/host/src/loader/active_assembly_context.rs',
          'runtime/host/src/loader/assembly_admission.rs',
        ],
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
        symbol: 'dispatch_in_process_boundary',
        ownedRoots: ['runtime/eval/src/assembly_execution/mod.rs'],
        requiredFile: 'runtime/eval/src/assembly_execution/mod.rs',
        requiredAnchors: [
          'record_in_process_boundary_dispatch(',
          'BoundaryStreamContract',
          'async_stream_cancel::execute_service_call(',
        ],
      },
      {
        role: 'internal-service-call-adapter',
        subjectId: 'single-service-dispatcher',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'dispatch_service_call',
        ownedRoots: ['runtime/eval/src/assembly_execution/mod.rs'],
        requiredFile: 'runtime/eval/src/assembly_execution/mod.rs',
        requiredAnchors: ['resolve_service_call(', 'dispatch_in_process_boundary('],
      },
      {
        role: 'ingress-service-call-adapter',
        subjectId: 'single-service-dispatcher',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'dispatch_ingress_via_in_process_boundary',
        ownedRoots: ['runtime/eval/src/assembly_execution/ingress.rs'],
        requiredFile: 'runtime/eval/src/assembly_execution/ingress.rs',
        requiredAnchors: ['adapt_ingress_arguments(', 'dispatch_in_process_boundary('],
      },
      {
        role: 'legacy-service-path-fence',
        subjectId: 'single-service-dispatcher',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'ensure_legacy_service_path_allowed',
        ownedRoots: ['runtime/eval/src/eval_context.rs'],
        requiredFile: 'runtime/eval/src/eval_context.rs',
        requiredAnchors: ['projection.assembly().is_some()', 'return Err('],
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
        role: 'active-assembly-context-set',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'ActiveAssemblyContextSet',
        ownedRoots: ['runtime/host/src/loader/active_assembly_context.rs'],
        requiredFile: 'runtime/host/src/loader/active_assembly_context.rs',
        requiredAnchors: [
          'activations_by_deployment',
          'contracts',
          'operation_targets',
        ],
      },
      {
        role: 'active-assembly-route',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'struct',
        symbol: 'ActiveAssemblyRoute',
        ownedRoots: ['runtime/host/src/loader/assembly_admission.rs'],
        requiredFile: 'runtime/host/src/loader/assembly_admission.rs',
        requiredAnchors: [
          'active: Arc<ActiveAssembly>',
          'activation: Arc<ActivationContext>',
          'provider_target',
        ],
      },
      {
        role: 'host-request-entry',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'spawn_request_inner',
        ownedRoots: ['runtime/host/src/host/request_entry/assembly.rs'],
        requiredFile: 'runtime/host/src/host/request_entry/assembly.rs',
        requiredAnchors: [
          'lookup_active_assembly_request_route(',
          'route.request_target(',
          'spawn_assembly_request(',
        ],
      },
      {
        role: 'assembly-request-spawn',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'spawn_assembly_request',
        ownedRoots: ['runtime/host/src/host/request_entry/assembly.rs'],
        requiredFile: 'runtime/host/src/host/request_entry/assembly.rs',
        requiredAnchors: [
          '_pinned_route',
          'AssemblyRequestExecutionInput',
          'execute_runtime_assembly_request(',
        ],
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
        symbol: 'handleBinaryMessage',
        ownedRoots: ['router/src/router/runtimeEndpoint.ts'],
        requiredFile: 'router/src/router/runtimeEndpoint.ts',
        requiredAnchors: [],
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
