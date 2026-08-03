import { lineNumberAt } from './runtime-artifact-boundary-rust-source.mjs';
import {
  escapeRuntimeExecutionBoundaryRegexp,
  pathIsWithin,
  runtimeExecutionBoundaryViolation,
} from './runtime-execution-boundary-registry.mjs';
const CALLBACK_CARRIER_REQUIRED_FIELDS = Object.freeze([
  'owner_runtime_replica_id',
  'owner_activation_id',
  'request_generation',
  'interface_or_adapter_contract',
  'opaque_capability_id',
]);

const USER_CODE_SPAWN_ANCHOR = /\b(?:execute_user_code|call_program_executable|execute_runtime_(?:assembly_)?request|run_(?:provider_)?stream|run_stream_producer_task)\b/;
const OWNED_CONTEXT_ANCHOR = /\b(?:OwnedProgramExecutionContext|RequestActivationContext|ActiveAssemblyRoute|ProviderStreamTask|RequestExecutionInput|AssemblyRequestExecutionInput|_pinned_route)\b/;

export function checkRuntimeExecutionBoundaryRules(
  registry,
  sources,
  ownerMatches,
  violations,
) {
  checkSingleDispatcher(registry, sources, ownerMatches, violations);
  checkActivationOwnership(registry, sources, ownerMatches, violations);
  checkOwnedContextSpawns(registry, sources, violations);
  checkHostRequestChain(ownerMatches, violations);
  checkRecoverableCallbackRejection(registry, sources, violations);
}

function checkSingleDispatcher(registry, sources, ownerMatches, violations) {
  const subject = subjectById(registry, 'single-service-dispatcher');
  if (!subject) {
    return;
  }
  const canonicalOwner = registry.owners.find(({ role }) => role === 'service-dispatcher');
  const canonical = ownerMatches.get('service-dispatcher') ?? [];
  const adapters = [
    ...(ownerMatches.get('internal-service-call-adapter') ?? []),
    ...(ownerMatches.get('ingress-service-call-adapter') ?? []),
  ];
  const candidateRegexp = /\b(?:struct|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*(?:InProcessBoundary|ServiceDispatcher))\b|\bfn\s+(dispatch_[A-Za-z0-9_]*(?:service|in_process_boundary)[A-Za-z0-9_]*)\b/g;
  for (const source of sourcesWithin(subject.discoveryRoots, subject.language, sources)) {
    for (const match of source.identifiers.matchAll(candidateRegexp)) {
      const symbol = match[1] ?? match[2];
      const functionOwnsBoundary = !match[2]
        || match[2].includes('in_process_boundary')
        || (
          canonicalOwner
          && rustFunctionCallsOwner(
            source.identifiers,
            match.index,
            canonicalOwner.symbol,
          )
        )
        || rustFunctionCallsOwner(
          source.identifiers,
          match.index,
          'execute_service_call',
        );
      if (!functionOwnsBoundary) {
        continue;
      }
      const isCanonical = canonicalOwner
        && symbol === canonicalOwner.symbol
        && canonical.some((entry) => entry.relPath === source.relPath && entry.index === match.index);
      const isAdapter = match[2]
        && canonicalOwner
        && adapters.some(
          (entry) => entry.relPath === source.relPath && entry.index === match.index,
        )
        && rustFunctionCallsOwner(source.identifiers, match.index, canonicalOwner.symbol);
      if (!isCanonical && !isAdapter) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'second-in-process-dispatcher',
          subject: subject.id,
          relPath: source.relPath,
          line: lineNumberAt(source.identifiers, match.index),
          matched: symbol,
          detail: 'a second service/in-process dispatcher owner is declared',
        }));
      }
    }
  }

  const remoteRule = /\b(?:select_remote(?:_boundary)?|dispatch_remote(?:_service)?|fallback_to_remote|remote_fallback)\b|\bBoundaryKind\s*::\s*Remote\b/g;
  for (const source of sourcesWithin(subject.discoveryRoots, subject.language, sources)) {
    addPatternViolations(
      source,
      remoteRule,
      'remote-boundary-selection',
      subject.id,
      'in-process service dispatch must fail closed instead of selecting or falling back remote',
      violations,
    );
  }

  checkRetiredServiceExecution(subject, sources, violations);

  if (canonicalOwner) {
    for (const root of subject.zones?.canonicalCallers ?? []) {
      const callRegexp = new RegExp(
        `\\b${escapeRuntimeExecutionBoundaryRegexp(canonicalOwner.symbol)}\\s*\\(`,
      );
      const callers = [...sources.values()].filter(
        (source) => source.language === subject.language && pathIsWithin(source.relPath, root),
      );
      const callsites = callers.flatMap((source) =>
        [...source.identifiers.matchAll(new RegExp(callRegexp.source, 'g'))]
          .filter((match) => !/\bfn\s+$/.test(source.identifiers.slice(
            Math.max(0, match.index - 12),
            match.index,
          )))
          .map((match) => ({ match, source })),
      );
      if (callsites.length === 0) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'dispatcher-callsite-missing',
          subject: subject.id,
          relPath: root,
          matched: canonicalOwner.symbol,
          detail: 'canonical ingress and internal call must both reference the single dispatcher',
        }));
      } else if (callsites.length > 1) {
        for (const { match, source } of callsites) {
          violations.push(runtimeExecutionBoundaryViolation({
            id: 'dispatcher-callsite-duplicate',
            subject: subject.id,
            relPath: source.relPath,
            line: lineNumberAt(source.identifiers, match.index),
            matched: canonicalOwner.symbol,
            detail: 'each canonical ingress/internal adapter must enter the dispatcher exactly once',
          }));
        }
      }
    }
  }
}

function checkRetiredServiceExecution(subject, sources, violations) {
  const retiredPatterns = [
    /\bensure_legacy_service_path_allowed\b/g,
    /\b(?:pub\s+)?mod\s+service_dispatch\b|\bservice_dispatch\s*::/g,
    /\b(?:trait|struct|type)\s+(?:OutboundServiceApi|OutboundServiceContext|ServiceDispatchContext)\b/g,
    /\b(?:RetiredAssemblyOutboundServiceContext|RuntimeOutboundServiceContext|outbound_service_context_from_request|retired_assembly_outbound)\b/g,
    /\bInterfaceCarrier\s*::\s*Remote\b/g,
    /\b(?:RequestStartControl|OutboundServiceRequestStart|OutboundStartedRequest)\b/g,
    /\bOutboundControlMessage\s*::\s*RequestStart\b/g,
    /\bresponse_(?:start|chunk|end|error)_to_outbound\b/g,
  ];
  for (const source of sourcesWithin(
    subject.zones?.retiredServiceExecution ?? [],
    'rust',
    sources,
  )) {
    for (const pattern of retiredPatterns) {
      addPatternViolations(
        source,
        pattern,
        'legacy-runtime-service-execution',
        subject.id,
        'retired outbound service execution and remote interface carriers must have zero runtime owners',
        violations,
      );
    }
  }
}

function checkActivationOwnership(registry, sources, ownerMatches, violations) {
  const subject = subjectById(registry, 'activation-request-callback-ownership');
  if (!subject) {
    return;
  }
  const mutableOwnerField = /\b[A-Za-z_][A-Za-z0-9_]*\s*:\s*(?:Arc\s*<\s*)?(?:ActivationContext|RequestActivationContext|RequestLifecycle|CallbackCapabilityTable)\b/g;
  for (const source of sourcesWithin(subject.zones?.sharedCode ?? [], 'rust', sources)) {
    addPatternViolations(
      source,
      mutableOwnerField,
      'shared-mutable-activation-owner',
      subject.id,
      'shared package code/image must not own activation, request, or callback mutable state',
      violations,
    );
  }

  const packageBuildCache = /\b(?:HashMap|BTreeMap|[A-Za-z0-9_]*Cache)\s*<\s*[^,;{}]{0,240}\bPackageBuildId\b[^,;{}]{0,80},\s*(?:Arc\s*<\s*)?(?:ActivationContext|RequestActivationContext|RequestLifecycle|CallbackCapabilityTable)\b/g;
  for (const source of sourcesWithin(subject.zones?.packageBuildCaches ?? [], 'rust', sources)) {
    addPatternViolations(
      source,
      packageBuildCache,
      'package-build-mutable-owner-cache',
      subject.id,
      'PackageBuildId keyed caches cannot own activation/request/callback mutable state',
      violations,
    );
  }

  for (const match of ownerMatches.get('callback-carrier') ?? []) {
    const item = match.item.identifiers;
    for (const field of CALLBACK_CARRIER_REQUIRED_FIELDS) {
      if (!new RegExp(`\\b${escapeRuntimeExecutionBoundaryRegexp(field)}\\b`).test(item)) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'callback-carrier-required-field-missing',
          subject: subject.id,
          ownerRole: 'callback-carrier',
          relPath: match.relPath,
          line: match.line,
          matched: field,
          detail: `opaque callback carrier is missing ${field}`,
        }));
      }
    }
    const forbidden = /\b(?:method_table|methodTable|native_object|nativeObject|native_address|nativeAddress|process_address|processAddress|raw_pointer|rawPointer|NonNull)\b|\*\s*(?:const|mut)\b|\bfn\s*\(/g;
    for (const forbiddenMatch of item.matchAll(forbidden)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'callback-carrier-native-address',
        subject: subject.id,
        ownerRole: 'callback-carrier',
        relPath: match.relPath,
        line: match.line + lineNumberAt(item, forbiddenMatch.index) - 1,
        matched: forbiddenMatch[0],
        detail: 'callback capability carrier must not contain method tables, native objects, or addresses',
      }));
    }
  }
}

function checkOwnedContextSpawns(registry, sources, violations) {
  const subject = subjectById(registry, 'owned-context-user-code-spawn');
  if (!subject) {
    return;
  }
  for (const source of sourcesWithin(subject.discoveryRoots, 'rust', sources)) {
    const tlsRule = /\b(?:task_local|thread_local)!|\bCURRENT_(?:SERVICE|ACTIVATION|REQUEST_CONTEXT|EXECUTION_CONTEXT)\b|\bcurrent_(?:service|activation)_(?:context|owner)\s*\(/g;
    addPatternViolations(
      source,
      tlsRule,
      'current-service-task-local',
      subject.id,
      'activation/request ownership must propagate explicitly, never through thread/task local state',
      violations,
    );
  }

  for (const source of sourcesWithin(subject.zones?.userCodeSpawn ?? [], 'rust', sources)) {
    for (const match of source.identifiers.matchAll(/\btokio\s*::\s*spawn\s*\(/g)) {
      const range = callRange(source.identifiers, match.index);
      const start = Math.max(0, match.index - 500);
      const end = range?.end ?? Math.min(source.identifiers.length, match.index + 800);
      const window = source.identifiers.slice(start, end);
      if (!USER_CODE_SPAWN_ANCHOR.test(window)) {
        continue;
      }
      if (!OWNED_CONTEXT_ANCHOR.test(window)) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'unowned-user-code-spawn',
          subject: subject.id,
          relPath: source.relPath,
          line: lineNumberAt(source.identifiers, match.index),
          matched: match[0],
          detail: 'tokio::spawn executing user code does not carry an owned activation/request context',
        }));
      }
    }
  }
}

function checkHostRequestChain(ownerMatches, violations) {
  const requestChainOwners = [
    ...(ownerMatches.get('host-request-route-lookup') ?? []),
    ...(ownerMatches.get('assembly-request-wire') ?? []),
    ...(ownerMatches.get('assembly-request-spawn') ?? []),
  ];
  for (const match of requestChainOwners) {
    const forbidden = /\b(?:lookup_operation_in_state|lookup_request_operation|route_registry|lazy_[A-Za-z0-9_]*|load_[A-Za-z0-9_]*artifact|legacy_[A-Za-z0-9_]*|fallback_[A-Za-z0-9_]*)\b/g;
    for (const entry of match.item.identifiers.matchAll(forbidden)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'host-request-fallback',
        subject: match.owner.subjectId,
        ownerRole: match.owner.role,
        relPath: match.relPath,
        line: match.line + lineNumberAt(match.item.identifiers, entry.index) - 1,
        matched: entry[0],
        detail: 'host request entry must use only its pinned active assembly route without lazy/legacy fallback',
      }));
    }
    const semanticFallback = /\b(?:build_id|operation_abi_id|display_name|display_target)\b/g;
    for (const entry of match.item.identifiers.matchAll(semanticFallback)) {
      violations.push(runtimeExecutionBoundaryViolation({
        id: 'host-request-semantic-fallback',
        subject: match.owner.subjectId,
        ownerRole: match.owner.role,
        relPath: match.relPath,
        line: match.line + lineNumberAt(match.item.identifiers, entry.index) - 1,
        matched: entry[0],
        detail: 'build/operation/display metadata cannot redirect canonical assembly ingress',
      }));
    }
  }
}

function checkRecoverableCallbackRejection(registry, sources, violations) {
  const subject = subjectById(registry, 'recoverable-callback-rejection');
  if (!subject) {
    return;
  }
  const production = sourcesWithin(subject.discoveryRoots, 'rust', sources);
  const rejection = /InterfaceCarrier\s*::\s*CallbackCapability\s*\([^)]*\)\s*=>\s*Err\s*\([\s\S]{0,480}callback_capability_not_recoverable_error\s*\(/;
  if (!production.some((source) => rejection.test(source.identifiers))) {
    const owner = registry.owners.find(({ role }) => role === 'recoverable-callback-encoder');
    violations.push(runtimeExecutionBoundaryViolation({
      id: 'recoverable-callback-not-rejected',
      subject: subject.id,
      ownerRole: owner?.role,
      relPath: owner?.requiredFile ?? subject.requiredFiles?.[0],
      matched: 'InterfaceCarrier::CallbackCapability',
      detail: 'production recoverable encoder must reject callback capability before hooks/fallback',
    }));
  }
}

function rustFunctionCallsOwner(text, declarationIndex, ownerSymbol) {
  const brace = text.indexOf('{', declarationIndex);
  if (brace === -1) {
    return false;
  }
  const close = matchingDelimiterIndex(text, brace, '{', '}');
  if (close === -1) {
    return false;
  }
  return new RegExp(
    `\\b${escapeRuntimeExecutionBoundaryRegexp(ownerSymbol)}\\s*\\(`,
  ).test(text.slice(brace + 1, close));
}

function callRange(text, index) {
  const open = text.indexOf('(', index);
  if (open === -1) {
    return undefined;
  }
  const close = matchingDelimiterIndex(text, open, '(', ')');
  return close === -1 ? undefined : { start: index, end: close + 1 };
}

function matchingDelimiterIndex(text, openIndex, open, close) {
  let depth = 0;
  for (let index = openIndex; index < text.length; index += 1) {
    if (text[index] === open) {
      depth += 1;
    } else if (text[index] === close) {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  return -1;
}

function sourcesWithin(roots, language, sources) {
  return [...sources.values()].filter(
    (source) => source.language === language && roots.some((root) => pathIsWithin(source.relPath, root)),
  );
}

function addPatternViolations(
  source,
  regexp,
  id,
  subject,
  detail,
  violations,
  view = 'identifiers',
) {
  const text = source[view];
  regexp.lastIndex = 0;
  for (const match of text.matchAll(regexp)) {
    violations.push(runtimeExecutionBoundaryViolation({
      id,
      subject,
      relPath: source.relPath,
      line: lineNumberAt(text, match.index),
      matched: match[0],
      detail,
    }));
  }
}

function subjectById(registry, id) {
  return registry.subjects.find((subject) => subject.id === id);
}
