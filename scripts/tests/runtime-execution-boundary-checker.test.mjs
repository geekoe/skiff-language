import assert from 'node:assert/strict';
import test from 'node:test';

import {
  RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS,
  runRuntimeExecutionBoundarySelfTest,
} from '../lib/runtime-execution-boundary-self-test.mjs';
import { inspectRuntimeExecutionBoundaryOwners } from '../lib/runtime-execution-boundary-registry.mjs';
import { checkRuntimeExecutionBoundaryRules } from '../lib/runtime-execution-boundary-rules.mjs';
import {
  productionTypeScriptViews,
  scanRuntimeExecutionBoundarySource,
} from '../lib/runtime-execution-boundary-source.mjs';
import {
  RUNTIME_EXECUTION_BOUNDARY_REGISTRY,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_OWNER_ROLES,
  REQUIRED_RUNTIME_EXECUTION_BOUNDARY_SUBJECT_IDS,
} from '../lib/runtime-execution-boundary-subjects.mjs';

test('runtime execution boundary checker rejects every hermetic mutation', async () => {
  const matrix = await runRuntimeExecutionBoundarySelfTest();
  assert.deepEqual(matrix, RUNTIME_EXECUTION_BOUNDARY_MUTATION_EXPECTATIONS);
  assert.equal(matrix.length, 33);
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
      'host-active-assembly-entry-missing',
      'required-owner-anchor-missing',
      'legacy-outbound-service-edge',
      'router-service-relay',
      'router-service-rejection-incomplete',
      'router-rejection-enters-relay-owner',
    ]),
  );
});

test('source scanner masks camouflage literals and preserves typed spans', () => {
  const rust = [
    'real_host_call();',
    '// comment_host_call();',
    '/* outer /* nested_comment_call(); */ block_comment_call(); */',
    "let character = 'x';",
    "let byte_character = b'x';",
    'let ordinary = "🦀 ordinary_host_call(";',
    'let bytes = b"byte_host_call(";',
    'let raw = r#"raw_host_call("#;',
    'let byte_raw = br##"byte_raw_host_call("##;',
  ].join('\n');
  const rustView = scanRuntimeExecutionBoundarySource(rust, 'rust');
  assertStableCodeView(rust, rustView.code);
  assert.deepEqual(
    rustView.tokens.filter(({ kind }) => kind === 'comment').map(({ tokenKind }) => tokenKind),
    ['line-comment', 'block-comment'],
  );
  assert.deepEqual(
    rustView.tokens.filter(({ kind }) => kind === 'literal').map(({ literalKind }) => literalKind),
    ['char', 'byte-char', 'string', 'byte-string', 'raw-string', 'b-raw-string'],
  );
  assert.match(rustView.code, /real_host_call\s*\(/);
  assert.doesNotMatch(rustView.code, /comment_host_call|ordinary_host_call|byte_host_call|raw_host_call/);
  assertTokenSpans(rust, rustView.tokens);

  const typescript = [
    'realRouterCall();',
    '// case \'request.start\': fakeLineComment();',
    '/* case "request.start": fakeBlockComment(); */',
    "const single = '🧭 case request.start fakeSingle()';",
    'const double = "case request.start fakeDouble()";',
    "const regexp = /case 'request.start': registry.pickDispatchConnection/;",
    "const template = `case 'request.start': fakeTemplate()`;",
    'const interpolation = `fakeOuter() ${(/registry/.test(value), this.options.registry.pickDispatchConnection(header))} ${`fakeNested() ${realNestedCall()}`} fakeTail()`;',
  ].join('\n');
  const typeScriptView = scanRuntimeExecutionBoundarySource(typescript, 'typescript');
  assertStableCodeView(typescript, typeScriptView.code);
  assert.deepEqual(
    typeScriptView.tokens.filter(({ kind }) => kind === 'comment').map(({ tokenKind }) => tokenKind),
    ['line-comment', 'block-comment'],
  );
  assert.deepEqual(
    typeScriptView.tokens.filter(({ kind }) => kind === 'literal').map(({ literalKind }) => literalKind),
    ['string', 'string', 'regexp', 'template', 'template', 'regexp', 'template'],
  );
  assert.match(typeScriptView.code, /realRouterCall\s*\(/);
  assert.match(typeScriptView.code, /registry\s*\.\s*pickDispatchConnection\s*\(/);
  assert.match(typeScriptView.code, /realNestedCall\s*\(/);
  assert.doesNotMatch(
    typeScriptView.code,
    /fakeLineComment|fakeBlockComment|fakeSingle|fakeDouble|fakeTemplate|fakeOuter|fakeNested|fakeTail/,
  );
  assert.equal(
    typeScriptView.tokens.some(({ kind, value }) => kind === 'keyword' && value === 'case'),
    false,
  );
  assertTokenSpans(typescript, typeScriptView.tokens);
});

test('independent R03 in-memory camouflage mutations fail closed', () => {
  const hostPath = 'runtime/host/src/host/request_entry.rs';
  const hostOwner = {
    declarationKind: 'function',
    language: 'rust',
    ownedRoots: [hostPath],
    requiredAnchors: [
      'active_runtime_assembly_route(',
      'ActiveAssemblyRoute',
    ],
    requiredFile: hostPath,
    role: 'host-request-route-lookup',
    subjectId: 'active-only-host-request-entry',
    symbol: 'lookup_active_assembly_request_route',
  };
  const hostSafe = [
    'fn lookup_active_assembly_request_route(&self, key: &Key) -> Result<ActiveAssemblyRoute> {',
    '    self.active_runtime_assembly_route(key)',
    '}',
  ].join('\n');
  const hostMutation = replaceProbe(
    hostSafe,
    [
      '-> Result<ActiveAssemblyRoute> {',
      '    self.active_runtime_assembly_route(key)',
    ].join('\n'),
    [
      '-> Result<Route> {',
      '    let _ordinary = "active_runtime_assembly_route( ActiveAssemblyRoute";',
      '    let _ = key;',
      '    unreachable!()',
    ].join('\n'),
  );
  const hostViolations = [];
  const hostMatches = inspectRuntimeExecutionBoundaryOwners(
    { owners: [hostOwner] },
    new Map([[hostPath, rustSource(hostPath, hostMutation)]]),
    hostViolations,
  );
  assert.equal(hostMatches.get(hostOwner.role)?.length, 1);
  assert.deepEqual(
    hostViolations.map(({ id, matched }) => ({ id, matched })),
    hostOwner.requiredAnchors.map((matched) => ({
      id: 'host-active-assembly-entry-missing',
      matched,
    })),
  );

  const routerPath = 'router/src/router/runtimeEndpoint.ts';
  const routerOwner = {
    declarationKind: 'method',
    language: 'typescript',
    ownedRoots: [routerPath],
    requiredAnchors: [],
    requiredFile: routerPath,
    role: 'router-runtime-service-rejection',
    subjectId: 'router-runtime-service-rejection',
    symbol: 'handleBinaryMessage',
  };
  const safeCase = [
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
  ].join('\n');
  const routerSafe = [
    'class RuntimeEndpoint {',
    '  private async handleBinaryMessage(ws: WebSocket, data: Uint8Array): Promise<void> {',
    '    const header = decode(data);',
    '    switch (header.type) {',
    safeCase,
    "      case 'response.end':",
    '        return;',
    '    }',
    '  }',
    '}',
  ].join('\n');
  const routerMutation = replaceProbe(routerSafe, safeCase, [
    "      case 'request.start': {",
    '        const camouflage = `',
    safeCase,
    '        `;',
    '        void camouflage;',
    '        this.options.registry.pickDispatchConnection(header);',
    '        return;',
    '      }',
  ].join('\n'));
  const routerSources = new Map([
    [routerPath, typeScriptSource(routerPath, routerMutation)],
  ]);
  const routerViolations = [];
  const routerRegistry = {
    owners: [routerOwner],
    subjects: [{
      discoveryRoots: ['router/src/router'],
      id: 'router-runtime-service-rejection',
    }],
  };
  const routerMatches = inspectRuntimeExecutionBoundaryOwners(
    routerRegistry,
    routerSources,
    routerViolations,
  );
  checkRuntimeExecutionBoundaryRules(
    routerRegistry,
    routerSources,
    routerMatches,
    routerViolations,
  );
  assert.equal(routerMatches.get(routerOwner.role)?.length, 1);
  assert.equal(
    routerViolations.some(({ id }) => id === 'router-service-rejection-incomplete'),
    true,
  );
  assert.equal(
    routerViolations.some(({ id }) => id === 'router-rejection-enters-relay-owner'),
    true,
  );
});

test('diagnostic remote frames and test-effect dispatch are not service routing owners', () => {
  const subject = {
    discoveryRoots: ['runtime/eval/src'],
    id: 'single-service-dispatcher',
    language: 'rust',
    zones: { canonicalCallers: [], legacyServiceEdges: [] },
  };
  const sources = new Map([
    [
      'runtime/eval/src/assembly_execution/service_error_channel.rs',
      rustSource(
        'runtime/eval/src/assembly_execution/service_error_channel.rs',
        [
          'fn import_error() {',
          '    stack.push(ExceptionStackFrame::RemoteBoundary { service_id, operation_id });',
          '}',
        ].join('\n'),
      ),
    ],
    [
      'runtime/eval/src/test_effect_registry.rs',
      rustSource(
        'runtime/eval/src/test_effect_registry.rs',
        'fn dispatch_service(&self, target: &TestEffectTarget) -> TestEffect { todo!() }\n',
      ),
    ],
  ]);
  const violations = [];
  checkRuntimeExecutionBoundaryRules(
    { owners: [], subjects: [subject] },
    sources,
    new Map(),
    violations,
  );
  assert.deepEqual(violations, []);
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
        'host-request-route-lookup',
        'lookup_active_assembly_request_route',
        'runtime/host/src/host/request_entry.rs',
      ),
      owner(
        'assembly-request-wire',
        'spawn_runtime_assembly_request',
        'runtime/host/src/host/request_entry/assembly_wire.rs',
      ),
      owner(
        'assembly-request-spawn',
        'spawn_request_on_active_assembly_route',
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

function assertStableCodeView(source, code) {
  assert.equal(code.length, source.length);
  assert.deepEqual(newlineOffsets(code), newlineOffsets(source));
}

function newlineOffsets(source) {
  const offsets = [];
  for (let index = 0; index < source.length; index += 1) {
    if (source[index] === '\n') offsets.push(index);
  }
  return offsets;
}

function assertTokenSpans(source, tokens) {
  for (const token of tokens) {
    assert.equal(token.value === undefined, false);
    if (token.kind === 'literal') {
      assert.equal(token.raw, source.slice(token.start, token.end));
    }
  }
}

function replaceProbe(source, before, after) {
  assert.notEqual(before, after);
  assert.equal(
    source.split(before).length - 1,
    1,
    'in-memory mutation filter must match exactly once',
  );
  const mutated = source.replace(before, after);
  assert.notEqual(mutated, source);
  return mutated;
}

function rustSource(relPath, source) {
  const lexical = scanRuntimeExecutionBoundarySource(source, 'rust');
  return {
    ...lexical,
    commentless: lexical.code,
    identifiers: lexical.code,
    language: 'rust',
    relPath,
    source,
  };
}

function typeScriptSource(relPath, source) {
  return {
    ...productionTypeScriptViews(source),
    language: 'typescript',
    relPath,
    source,
  };
}
