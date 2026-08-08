export const REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS = Object.freeze([
  'single-service-dispatcher',
  'activation-request-callback-ownership',
  'owned-context-user-code-spawn',
  'required-host-request-entry',
  'recoverable-callback-rejection',
]);

export const REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES = Object.freeze([
  'service-dispatcher',
  'internal-service-call-adapter',
  'ingress-service-call-adapter',
  'activation-context',
  'request-generation',
  'callback-table',
  'callback-carrier',
  'owned-context-carrier',
  'active-assembly-context-set',
  'active-assembly-route',
  'host-request-route-lookup',
  'assembly-request-wire',
  'assembly-request-spawn',
  'recoverable-callback-encoder',
]);

export const RUNTIME_EXECUTION_BOUNDARY_REGISTRY =
  defineRuntimeExecutionBoundaryRegistry({
    sourceRoots: [
      { id: 'runtime-rust', language: 'rust', root: 'runtime' },
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
          retiredServiceExecution: [
            'runtime/eval/src',
            'runtime/host/src',
            'runtime/model/src',
            'runtime/boundary/src',
            'runtime/capability-context/src',
            'runtime/request-contract/src',
            'runtime/request/src',
            'runtime/transport/src',
          ],
        },
        requiredFiles: [
          'runtime/eval/src/assembly_execution/mod.rs',
          'runtime/eval/src/assembly_execution/ingress.rs',
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
          'runtime/host/src/host/request_entry.rs',
          'runtime/host/src/host/request_entry/assembly.rs',
          'runtime/host/src/host/request_entry/assembly_wire.rs',
          'runtime/host/src/loader/active_assembly_context.rs',
          'runtime/host/src/loader/assembly_admission.rs',
        ],
        requiredFiles: [
          'runtime/host/src/host/request_entry.rs',
          'runtime/host/src/host/request_entry/assembly.rs',
          'runtime/host/src/host/request_entry/assembly_wire.rs',
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
          'ingress_key: ServiceIngressKey',
          'entry: Arc<LinkedGatewayEntry>',
          'activation: Arc<ActivationContext>',
        ],
      },
      {
        role: 'host-request-route-lookup',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'resolve_active_assembly_request_route',
        ownedRoots: ['runtime/host/src/host/request_entry/assembly_wire.rs'],
        requiredFile: 'runtime/host/src/host/request_entry/assembly_wire.rs',
        requiredAnchors: [
          'route_or_lazy_load(',
          'ActiveAssemblyRoute',
        ],
      },
      {
        role: 'assembly-request-wire',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'spawn_runtime_assembly_request',
        ownedRoots: ['runtime/host/src/host/request_entry/assembly_wire.rs'],
        requiredFile: 'runtime/host/src/host/request_entry/assembly_wire.rs',
        requiredAnchors: [
          'http_gateway_request_from_wire(',
          'task_request_on_active_assembly_route(',
        ],
      },
      {
        role: 'assembly-request-spawn',
        subjectId: 'required-host-request-entry',
        language: 'rust',
        declarationKind: 'function',
        symbol: 'task_request_on_active_assembly_route',
        ownedRoots: ['runtime/host/src/host/request_entry/assembly.rs'],
        requiredFile: 'runtime/host/src/host/request_entry/assembly.rs',
        requiredAnchors: [
          'route.request_target(',
          'RuntimeHttpGatewayExecutionInput',
          'execute_runtime_http_gateway_request(',
          'drop(route)',
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
