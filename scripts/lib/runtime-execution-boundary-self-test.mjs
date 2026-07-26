import {
  appendFile,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
  collectRuntimeExecutionBoundaryViolations,
  formatRuntimeExecutionBoundaryViolation,
} from './runtime-execution-boundary-checker.mjs';
import { RUNTIME_EXECUTION_BOUNDARY_REGISTRY } from './runtime-execution-boundary-subjects.mjs';

export const RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS = Object.freeze([
  expectation('remote boundary violation injection', 'remote-boundary-selection'),
  expectation('remote fallback selection', 'remote-boundary-selection'),
  expectation('canonical dispatcher renamed', 'required-owner-missing'),
  expectation('canonical dispatcher moved', 'owner-outside-registered-root'),
  expectation('canonical dispatcher duplicated', 'duplicate-required-owner'),
  expectation('canonical ingress dispatcher call omitted', 'dispatcher-callsite-missing'),
  expectation('required subject omitted', 'subject-registry-omission'),
  expectation('required production file omitted', 'required-subject-file-missing'),
  expectation('required owner role omitted', 'owner-registry-omission'),
  expectation('required owner file registry omitted', 'required-owner-file-registry-omission'),
  expectation('owner discovery root registry omitted', 'owner-root-registry-omission'),
  expectation('broad allowlist registry escape', 'forbidden-registry-exception-field'),
  expectation('test-named production camouflage', 'remote-boundary-selection'),
  expectation('test-support cfg camouflage', 'current-service-task-local'),
  expectation('second in-process dispatcher', 'second-in-process-dispatcher'),
  expectation('current service TLS', 'current-service-task-local'),
  expectation('shared callback table', 'shared-mutable-activation-owner'),
  expectation('PackageBuildId callback cache', 'package-build-mutable-owner-cache'),
  expectation('callback carrier native address', 'callback-carrier-native-address'),
  expectation('unowned user-code spawn', 'unowned-user-code-spawn'),
  expectation('recoverable callback acceptance', 'recoverable-callback-not-rejected'),
  expectation('host request old-route fallback', 'host-request-fallback'),
  expectation('host active anchors hidden in literals', 'host-active-assembly-entry-missing'),
  expectation('assembly request route pin omitted', 'required-owner-anchor-missing'),
  expectation('legacy outbound service edge', 'legacy-outbound-service-edge'),
  expectation('legacy outbound fence omitted', 'legacy-outbound-service-edge'),
  expectation('router runtime service relay', 'router-service-relay'),
  expectation('router rejection payload omitted', 'router-service-rejection-incomplete'),
  expectation('router rejection enters registry', 'router-rejection-enters-relay-owner'),
  expectation('router template case camouflage enters selection', 'router-rejection-enters-relay-owner'),
]);

export async function runRuntimeExecutionBoundarySelfTest() {
  await withFixture(async (root) => {
    const baseline = await collectRuntimeExecutionBoundaryViolations(root);
    assertNoViolations(
      'baseline with a genuine exact #[cfg(test)] external module and inline item',
      baseline,
    );
  });

  const matrix = mutationMatrix();
  for (const entry of matrix) {
    await withFixture(async (root) => {
      const registry = cloneRegistry();
      await entry.mutate?.(root, registry);
      entry.mutateRegistry?.(registry);
      const violations = await collectRuntimeExecutionBoundaryViolations(root, registry);
      if (!violations.some(({ id }) => id === entry.expectedId)) {
        throw new Error(
          `${entry.name}: expected ${entry.expectedId}; got\n${formatViolations(violations)}`,
        );
      }
    });
  }

  return RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS;
}

function mutationMatrix() {
  return [
    mutation(
      'remote boundary violation injection',
      'remote-boundary-selection',
      (root) => append(
        root,
        'runtime/eval/src/assembly_execution/mod.rs',
        '\nenum RemoteBoundary { Network }\n',
      ),
    ),
    mutation(
      'remote fallback selection',
      'remote-boundary-selection',
      (root) => append(
        root,
        'runtime/eval/src/assembly_execution/mod.rs',
        '\nfn fallback_to_remote() {}\n',
      ),
    ),
    mutation(
      'canonical dispatcher renamed',
      'required-owner-missing',
      (root) => replace(
        root,
        'runtime/eval/src/assembly_execution/mod.rs',
        'fn dispatch_in_process_boundary',
        'fn renamed_dispatch_in_process_boundary',
      ),
    ),
    mutation(
      'canonical dispatcher moved',
      'owner-outside-registered-root',
      async (root) => {
        await mkdir(join(root, 'runtime/model/src'), { recursive: true });
        await rename(
          join(root, 'runtime/eval/src/assembly_execution/mod.rs'),
          join(root, 'runtime/model/src/moved_dispatcher.rs'),
        );
      },
    ),
    mutation(
      'canonical dispatcher duplicated',
      'duplicate-required-owner',
      async (root) => {
        await mkdir(join(root, 'runtime/model/src'), { recursive: true });
        await copyFile(
          join(root, 'runtime/eval/src/assembly_execution/mod.rs'),
          join(root, 'runtime/model/src/copied_dispatcher.rs'),
        );
      },
    ),
    mutation(
      'canonical ingress dispatcher call omitted',
      'dispatcher-callsite-missing',
      (root) => replace(
        root,
        'runtime/eval/src/assembly_execution/ingress.rs',
        '    dispatch_in_process_boundary(context).await;',
        '    adapt_ingress_only(context).await;',
      ),
    ),
    mutation(
      'required subject omitted',
      'subject-registry-omission',
      undefined,
      (registry) => {
        registry.subjects = registry.subjects.filter(
          ({ id }) => id !== 'router-runtime-service-rejection',
        );
      },
    ),
    mutation(
      'required production file omitted',
      'required-subject-file-missing',
      (root) => rm(join(root, 'runtime/host/src/host/request_entry/assembly.rs')),
    ),
    mutation(
      'required owner role omitted',
      'owner-registry-omission',
      undefined,
      (registry) => {
        registry.owners = registry.owners.filter(({ role }) => role !== 'callback-table');
      },
    ),
    mutation(
      'required owner file registry omitted',
      'required-owner-file-registry-omission',
      undefined,
      (registry) => {
        const subject = registry.subjects.find(({ id }) => id === 'required-host-request-entry');
        subject.requiredFiles = [];
      },
    ),
    mutation(
      'owner discovery root registry omitted',
      'owner-root-registry-omission',
      undefined,
      (registry) => {
        const subject = registry.subjects.find(({ id }) => id === 'required-host-request-entry');
        subject.discoveryRoots = [];
      },
    ),
    mutation(
      'broad allowlist registry escape',
      'forbidden-registry-exception-field',
      undefined,
      (registry) => {
        registry.subjects[0].allowlist = ['runtime/**'];
      },
    ),
    mutation(
      'test-named production camouflage',
      'remote-boundary-selection',
      (root) => write(
        root,
        'runtime/eval/src/runtime_execution_tests.rs',
        'pub struct TestHelperRemoteBoundary;\n',
      ),
    ),
    mutation(
      'test-support cfg camouflage',
      'current-service-task-local',
      (root) => append(
        root,
        'runtime/eval/src/program_stream.rs',
        '\n#[cfg(any(test, feature = "test-support"))]\nthread_local! { static CURRENT_SERVICE: u8 = 0; }\n',
      ),
    ),
    mutation(
      'second in-process dispatcher',
      'second-in-process-dispatcher',
      (root) => append(
        root,
        'runtime/eval/src/assembly_execution/mod.rs',
        '\npub struct BackupInProcessBoundary;\n',
      ),
    ),
    mutation(
      'current service TLS',
      'current-service-task-local',
      (root) => write(
        root,
        'runtime/eval/src/current_service.rs',
        'tokio::task_local! { static CURRENT_ACTIVATION: String; }\n',
      ),
    ),
    mutation(
      'shared callback table',
      'shared-mutable-activation-owner',
      (root) => write(
        root,
        'runtime/linked-program/src/shared_image.rs',
        'pub struct SharedPackageLinkedImage { callback_table: Arc<CallbackCapabilityTable> }\n',
      ),
    ),
    mutation(
      'PackageBuildId callback cache',
      'package-build-mutable-owner-cache',
      (root) => write(
        root,
        'runtime/activation/src/cache.rs',
        'struct SharedCache { entries: HashMap<PackageBuildId, Arc<ActivationContext>> }\n',
      ),
    ),
    mutation(
      'callback carrier native address',
      'callback-carrier-native-address',
      (root) => replace(
        root,
        'runtime/model/src/value.rs',
        '    opaque_capability_id: String,\n}',
        '    opaque_capability_id: String,\n    native_address: usize,\n}',
      ),
    ),
    mutation(
      'unowned user-code spawn',
      'unowned-user-code-spawn',
      (root) => write(
        root,
        'runtime/eval/src/assembly_execution/unowned.rs',
        'fn start() { tokio::spawn(async move { execute_user_code().await; }); }\n',
      ),
    ),
    mutation(
      'recoverable callback acceptance',
      'recoverable-callback-not-rejected',
      (root) => replace(
        root,
        'runtime/boundary/src/recoverable.rs',
        'InterfaceCarrier::CallbackCapability(carrier) => Err(callback_capability_not_recoverable_error(carrier)),',
        'InterfaceCarrier::CallbackCapability(_carrier) => Ok(()),',
      ),
    ),
    mutation(
      'host request old-route fallback',
      'host-request-fallback',
      (root) => replace(
        root,
        'runtime/host/src/host/request_entry/assembly.rs',
        'lookup_active_assembly_request_route',
        'lookup_operation_in_state',
      ),
    ),
    mutation(
      'host active anchors hidden in literals',
      'host-active-assembly-entry-missing',
      (root) => replace(
        root,
        'runtime/host/src/host/request_entry/assembly.rs',
        [
          '        let route: ActiveAssemblyRoute = self.lookup_active_assembly_request_route();',
          '        let target = route.request_target();',
          '        self.spawn_assembly_request(route, target).await;',
        ].join('\n'),
        [
          '        let lookup_active_assembly_request_route_similar = 1;',
          '        let _ordinary = "lookup_active_assembly_request_route(";',
          '        let _raw = r#"route.request_target("#;',
          '        let _bytes = b"spawn_assembly_request(";',
          '        let _ = lookup_active_assembly_request_route_similar;',
        ].join('\n'),
      ),
    ),
    mutation(
      'assembly request route pin omitted',
      'required-owner-anchor-missing',
      (root) => replace(
        root,
        'runtime/host/src/host/request_entry/assembly.rs',
        'let _pinned_route = route;',
        'let _unpinned_route = route;',
      ),
    ),
    mutation(
      'legacy outbound service edge',
      'legacy-outbound-service-edge',
      (root) => append(
        root,
        'runtime/eval/src/eval_context.rs',
        '\nfn old_edge() { service_dispatch::call_outbound_service(); }\n',
      ),
    ),
    mutation(
      'legacy outbound fence omitted',
      'legacy-outbound-service-edge',
      (root) => replace(
        root,
        'runtime/eval/src/eval_context.rs',
        '    self.ensure_legacy_service_path_allowed();',
        '    let _legacy_path_is_unfenced = self;',
      ),
    ),
    mutation(
      'router runtime service relay',
      'router-service-relay',
      (root) => append(
        root,
        'router/src/router/runtimeDispatcher.ts',
        '\nfunction handleRuntimeRequestStart() { return { kind: \'forward\' }; }\n',
      ),
    ),
    mutation(
      'router rejection payload omitted',
      'router-service-rejection-incomplete',
      (root) => replace(
        root,
        'router/src/router/runtimeEndpoint.ts',
        "          code: 'InProcessServiceCallRequired',",
        "          code: 'UnexpectedRuntimeRequest',",
      ),
    ),
    mutation(
      'router rejection enters registry',
      'router-rejection-enters-relay-owner',
      (root) => replace(
        root,
        'router/src/router/runtimeEndpoint.ts',
        '        this.sendFrame(ws, {',
        '        this.options.registry.pickDispatchConnection(header);\n        this.sendFrame(ws, {',
      ),
    ),
    mutation(
      'router template case camouflage enters selection',
      'router-rejection-enters-relay-owner',
      (root) => replace(
        root,
        'router/src/router/runtimeEndpoint.ts',
        [
          "      case 'request.start':",
          "        if (header.caller.kind !== 'service') throw new Error('invalid caller');",
          '        this.sendFrame(ws, {',
          "          type: 'response.error',",
          '          requestId: header.requestId,',
          '          error: {',
          "            code: 'InProcessServiceCallRequired',",
          "            message: 'service calls require an in-process binding'",
          '          }',
          '        });',
          '        return;',
        ].join('\n'),
        [
          "      case 'request.start': {",
          '        const camouflage = `',
          "          case 'request.start':",
          "            if (header.caller.kind !== 'service') throw new Error('invalid caller');",
          '            this.sendFrame(ws, {',
          "              type: 'response.error',",
          '              requestId: header.requestId,',
          '              error: {',
          "                code: 'InProcessServiceCallRequired',",
          "                message: 'service calls require an in-process binding'",
          '              }',
          '            });',
          '            return;',
          '          ${this.options.registry.pickDispatchConnection(header)}',
          '        `;',
          '        void camouflage;',
          '        return;',
          '      }',
        ].join('\n'),
      ),
    ),
  ];
}

async function withFixture(run) {
  const root = await mkdtemp(join(tmpdir(), 'skiff-runtime-execution-boundary-'));
  try {
    await writeSafeFixture(root);
    await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

async function writeSafeFixture(root) {
  await Promise.all([
    write(
      root,
      'runtime/eval/src/lib.rs',
      '#[cfg(test)]\nmod execution_tests;\n',
    ),
    write(
      root,
      'runtime/eval/src/execution_tests.rs',
      'struct HiddenInProcessBoundary;\nthread_local! { static CURRENT_SERVICE: u8 = 0; }\n',
    ),
    write(
      root,
      'runtime/eval/src/assembly_execution/mod.rs',
      [
        'pub async fn dispatch_service_call(context: &Context) {',
        '    let _target = context.resolve_service_call();',
        '    dispatch_in_process_boundary(context).await;',
        '}',
        'async fn dispatch_in_process_boundary(context: &Context) {',
        '    record_in_process_boundary_dispatch(context);',
        '    match BoundaryStreamContract::Unary {',
        '        BoundaryStreamContract::Unary => async_stream_cancel::execute_service_call(context).await,',
        '    }',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/eval/src/assembly_execution/ingress.rs',
      [
        'pub async fn dispatch_ingress_via_in_process_boundary(context: &Context) {',
        '    let _args = adapt_ingress_arguments(context);',
        '    dispatch_in_process_boundary(context).await;',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/eval/src/assembly_execution/async_stream_cancel.rs',
      [
        'struct ProviderStreamTask;',
        'fn spawn_provider_stream(producer: ProviderStreamTask) {',
        '    tokio::spawn(async move { run_provider_stream(producer).await; });',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/eval/src/eval_context.rs',
      [
        'fn ensure_legacy_service_path_allowed(&self) -> Result<()> {',
        '    if self.projection.assembly().is_some() {',
        '        return Err("assembly execution cannot use legacy service path");',
        '    }',
        '    Ok(())',
        '}',
        'fn legacy_consumer(&self) {',
        '    self.ensure_legacy_service_path_allowed();',
        '    service_dispatch::call_outbound_service();',
        '}',
        '#[cfg(test)]',
        'fn hidden_tls() { tokio::task_local! { static CURRENT_ACTIVATION: u8; } }',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/eval/src/program_execution.rs',
      'pub struct OwnedProgramExecutionContext;\n',
    ),
    write(
      root,
      'runtime/eval/src/program_stream.rs',
      [
        'fn spawn_stream(owned_context: Arc<OwnedProgramExecutionContext>) {',
        '    tokio::spawn(async move { run_stream_producer_task(owned_context).await; });',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/activation/src/context.rs',
      'pub struct ActivationContext;\n',
    ),
    write(
      root,
      'runtime/activation/src/request_context.rs',
      'pub struct RequestLifecycle { generation: u64 }\n',
    ),
    write(
      root,
      'runtime/activation/src/capability.rs',
      'pub struct CallbackCapabilityTable;\n',
    ),
    write(
      root,
      'runtime/model/src/value.rs',
      [
        'pub struct CallbackCapabilityCarrier {',
        '    owner_runtime_replica_id: String,',
        '    owner_activation_id: String,',
        '    request_generation: u64,',
        '    interface_or_adapter_contract: String,',
        '    opaque_capability_id: String,',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/linked-program/src/shared_image.rs',
      'pub struct SharedPackageLinkedImage { immutable_code: Vec<u8> }\n',
    ),
    write(
      root,
      'runtime/boundary/src/recoverable.rs',
      [
        'pub struct RecoverableBoundaryCodec;',
        'fn encode(value: &InterfaceValue) -> Result<()> {',
        '    match value.carrier() {',
        '        InterfaceCarrier::CallbackCapability(carrier) => Err(callback_capability_not_recoverable_error(carrier)),',
        '        _ => Ok(()),',
        '    }',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/host/src/host/request_entry/assembly.rs',
      [
        'impl RuntimeHost {',
        '    async fn spawn_request_inner(&self) {',
        '        let route: ActiveAssemblyRoute = self.lookup_active_assembly_request_route();',
        '        let target = route.request_target();',
        '        self.spawn_assembly_request(route, target).await;',
        '    }',
        '    async fn spawn_assembly_request(&self, route: ActiveAssemblyRoute, target: Target) {',
        '        tokio::spawn(async move {',
        '            let _pinned_route = route;',
        '            execute_runtime_assembly_request(AssemblyRequestExecutionInput { target }).await;',
        '        });',
        '    }',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/host/src/host/request_entry.rs',
      [
        'fn spawn_owned_legacy_request(input: RequestExecutionInput) {',
        '    tokio::spawn(async move {',
        '        execute_runtime_request(input).await;',
        '    });',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/host/src/loader/active_assembly_context.rs',
      [
        'struct ActiveAssemblyContextSet {',
        '    activations_by_deployment: Map,',
        '    contracts: Map,',
        '    operation_targets: Map,',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'runtime/host/src/loader/assembly_admission.rs',
      [
        'struct ActiveAssemblyRoute {',
        '    active: Arc<ActiveAssembly>,',
        '    activation: Arc<ActivationContext>,',
        '    provider_target: OperationTargetRef,',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'router/src/router/runtimeDispatcher.ts',
      'export class RuntimeDispatcher {}\n',
    ),
    write(
      root,
      'router/src/router/runtimeEndpoint.ts',
      [
        'export class RuntimeEndpoint {',
        '  private async handleBinaryMessage(ws: WebSocket, data: Uint8Array): Promise<void> {',
        '    const header = decode(data);',
        '    switch (header.type) {',
        "      case 'request.start':",
        "        if (header.caller.kind !== 'service') throw new Error('invalid caller');",
        '        this.sendFrame(ws, {',
          "          type: 'response.error',",
          '          requestId: header.requestId,',
          '          error: {',
          "            code: 'InProcessServiceCallRequired',",
          "            message: 'service calls require an in-process binding'",
          '          }',
        '        });',
        '        return;',
        "      case 'response.end':",
        '        return;',
        '    }',
        '  }',
        '}',
        '',
      ].join('\n'),
    ),
    write(
      root,
      'router/src/router/runtimeRegistry.ts',
      'export class RuntimeRegistry { registerRuntime(): void {} }\n',
    ),
  ]);
}

function cloneRegistry() {
  return {
    sourceRoots: RUNTIME_EXECUTION_BOUNDARY_REGISTRY.sourceRoots.map((entry) => ({
      ...entry,
    })),
    subjects: RUNTIME_EXECUTION_BOUNDARY_REGISTRY.subjects.map((entry) => ({
      ...entry,
      discoveryRoots: [...entry.discoveryRoots],
      requiredFiles: [...entry.requiredFiles],
      zones: Object.fromEntries(
        Object.entries(entry.zones).map(([name, roots]) => [name, [...roots]]),
      ),
    })),
    owners: RUNTIME_EXECUTION_BOUNDARY_REGISTRY.owners.map((entry) => ({
      ...entry,
      ownedRoots: [...entry.ownedRoots],
      requiredAnchors: [...entry.requiredAnchors],
    })),
  };
}

async function write(root, relPath, contents) {
  const path = join(root, relPath);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents);
}

async function append(root, relPath, contents) {
  await appendFile(join(root, relPath), contents);
}

async function replace(root, relPath, from, to) {
  const path = join(root, relPath);
  const source = await readFile(path, 'utf8');
  if (!source.includes(from)) {
    throw new Error(`self-test fixture replacement anchor not found in ${relPath}: ${from}`);
  }
  await writeFile(path, source.replace(from, to));
}

function mutation(name, expectedId, mutate, mutateRegistry) {
  return { expectedId, mutate, mutateRegistry, name };
}

function expectation(name, expectedId) {
  return Object.freeze({ expectedId, name });
}

function assertNoViolations(label, violations) {
  if (violations.length > 0) {
    throw new Error(`${label}: expected no violations; got\n${formatViolations(violations)}`);
  }
}

function formatViolations(violations) {
  return violations.length === 0
    ? '<none>'
    : violations.map(formatRuntimeExecutionBoundaryViolation).join('\n');
}
