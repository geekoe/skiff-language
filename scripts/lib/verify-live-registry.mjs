export const LIVE_OWNERSHIP = Object.freeze({
  NONE: 'none',
  EXTERNAL: 'external',
  MANAGED: 'managed',
});

export const LIVE_TIERS = Object.freeze({
  SELF_TEST: 'self-test',
  LIVE_MANUAL: 'live/manual',
});

export const LIVE_PLAN_TYPES = Object.freeze({
  RUNTIME_FIXTURES: 'runtime-fixtures',
  FIXED_COMMAND: 'fixed-command',
});

export const LIVE_DISCOVERIES = Object.freeze({
  RUNTIME_LIVE_TESTS: 'runtime-live-tests',
});

export const LIVE_INPUTS = deepFreeze({
  runtimeActivationUrl: {
    option: 'runtimeLiveActivationUrl',
    environment: 'SKIFF_RUNTIME_LIVE_ACTIVATION_URL',
    description: 'assembly activation URL (SKIFF_RUNTIME_LIVE_ACTIVATION_URL or --runtime-live-activation-url <url>)',
  },
  runtimeIngressUrl: {
    option: 'runtimeLiveIngressUrl',
    environment: 'SKIFF_RUNTIME_LIVE_INGRESS_URL',
    description: 'runtime ingress URL (SKIFF_RUNTIME_LIVE_INGRESS_URL or --runtime-live-ingress-url <url>)',
  },
  runtimeArtifactRoot: {
    option: 'runtimeLiveArtifactRoot',
    environment: 'SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT',
    description:
      'artifact root (SKIFF_RUNTIME_LIVE_ARTIFACT_ROOT or --runtime-live-artifact-root <dir>)',
  },
  runtimeEnvironment: {
    option: 'runtimeLiveEnvironment',
    environment: 'SKIFF_RUNTIME_LIVE_ENVIRONMENT',
    description: 'activation environment (SKIFF_RUNTIME_LIVE_ENVIRONMENT or --runtime-live-environment <id>)',
  },
  runtimeExpectedGeneration: {
    option: 'runtimeLiveExpectedGeneration',
    environment: 'SKIFF_RUNTIME_LIVE_EXPECTED_GENERATION',
    description: 'expected generation (SKIFF_RUNTIME_LIVE_EXPECTED_GENERATION or --runtime-live-expected-generation <n>)',
  },
  loopRiskConfig: {
    option: 'loopRiskConfig',
    environment: 'SKIFF_LOOP_RISK_CONFIG',
    description:
      'loop-risk config (SKIFF_LOOP_RISK_CONFIG or --loop-risk-config <path>)',
  },
});

export const LIVE_REGISTRY = deepFreeze([
  {
    key: 'runtime-live-fixtures',
    source: {
      type: 'discovery',
      discovery: LIVE_DISCOVERIES.RUNTIME_LIVE_TESTS,
    },
    invocations: [
      {
        selector: 'runtime-live',
        description:
          'explicit live fixtures; requires canonical activation, ingress, artifact, environment, and generation targets',
        plan: LIVE_PLAN_TYPES.RUNTIME_FIXTURES,
        idPrefix: 'live:runtime:',
        ownership: LIVE_OWNERSHIP.EXTERNAL,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [
          'runtimeActivationUrl',
          'runtimeIngressUrl',
          'runtimeArtifactRoot',
          'runtimeEnvironment',
          'runtimeExpectedGeneration',
        ],
        requiredExecutables: ['cargo', 'node'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: true,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'db-encrypted-storage-live',
    source: {
      type: 'script',
      path: 'scripts/check-db-encrypted-storage-live.mjs',
    },
    invocations: [
      {
        selector: 'db-encrypted-storage-live',
        description: 'explicit managed Mongo/runtime/keyring live check',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:db-encrypted-storage',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'cargo', 'pnpm', 'mongod', 'mongosh'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'router-rust-bootstrap-live',
    source: {
      type: 'script',
      path: 'scripts/check-router-bootstrap-live.mjs',
    },
    invocations: [
      {
        selector: 'router-live:bootstrap',
        description:
          'real compiler artifact through committed reader to initial epoch; missing/malformed/pending/identity mismatch/loader saturation/shutdown fail closed (managed CI, isolated instance + explicit Rust process)',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:router-rust-bootstrap',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'cargo', 'mongod', 'mongosh'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'router-rust-session-live',
    source: {
      type: 'script',
      path: 'scripts/check-router-session-live.mjs',
    },
    invocations: [
      {
        selector: 'router-live:session',
        description:
          'real Rust Router binary + real Rust Runtime process bootstrap/register/health/reconnect/shutdown roundtrip with session barrier, pre-auth limit/timeout, ingress saturation and zero residue (managed CI, isolated instance + explicit Rust processes)',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:router-rust-session',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'cargo', 'mongod', 'mongosh'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'router-rust-dispatch-live',
    source: {
      type: 'script',
      path: 'scripts/check-router-dispatch-live.mjs',
    },
    invocations: [
      {
        selector: 'router-live:dispatch',
        description:
          'real production Router composition + real Rust Runtime process: fake ingress through the production HttpDispatchPort -> epoch capture -> exact candidate -> permit -> revalidate -> enqueue -> terminal; missing/invalid selector, wrong deployment/entry, duplicate id, timeout, disconnect and selection/replacement races fail closed with exact pending/permit zeroing (managed CI, isolated instance + explicit Rust process)',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:router-rust-dispatch',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'cargo', 'mongod', 'mongosh'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'router-rust-activation-full-chain-live',
    source: {
      type: 'script',
      path: 'scripts/check-router-activation-live.mjs',
    },
    invocations: [
      {
        selector: 'router-live:activation-full-chain',
        description:
          'real Router + temporary Mongo replica set + real compiler artifact + real Runtime: activate HTTP -> durable prepare -> real Runtime prepared -> durable commit -> epoch swap -> Runtime commit -> same-session re-register -> new-generation HTTP request; old captured-epoch request under its original lease; pre-decision disconnect abort / post-decision durable reconcile; cold recovery committed-first + rebind + candidate-load failure durable abort; audit/CAS/retry non-duplication (managed CI, isolated instance + explicit Rust processes)',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:router-rust-activation-full-chain',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'cargo', 'mongod', 'mongosh'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'router-rust-differential-live',
    source: {
      type: 'script',
      path: 'scripts/check-router-differential-live.mjs',
    },
    invocations: [
      {
        selector: 'router-live:differential',
        description:
          'implementation-neutral differential harness: isolated TS/Rust Router instances with independent ports/artifact roots/runtime homes/Mongo namespaces, real Runtime per side through a test-only relay; compares HTTP/Runtime frames/Mongo state+audit/terminal with uuid/timestamp/ephemeral-port/log-order normalization only (managed, explicit selector)',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:router-rust-differential',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'pnpm', 'cargo', 'mongod', 'mongosh'],
        requiredModules: [{ specifier: 'ws', from: 'router/package.json' }],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'router-rust-http-live',
    source: {
      type: 'script',
      path: 'scripts/check-router-http-live.mjs',
    },
    invocations: [
      {
        selector: 'router-live:http',
        description:
          'real HTTP->Router->Runtime unary/stream with trusted selectors, service-scoped ingress, typed/raw opaque payloads, stream sequencing, cumulative response ceiling, backpressure, disconnect/cancel/deadline, CORS preflight/service-managed and platform errors; every race one external terminal with at most one cancel and zero residue, plus the first TS->Rust->TS unary rollback roundtrip through the canonical RouterProcessSpec (managed CI, isolated instance + explicit process commands)',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:router-rust-http',
        args: [],
        ownership: LIVE_OWNERSHIP.MANAGED,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: [],
        requiredExecutables: ['node', 'cargo', 'pnpm', 'mongod', 'mongosh', 'python3'],
        requiredModules: [{ specifier: 'ws', from: 'router/package.json' }],
        canonicalPolicy: {
          forbidSkips: false,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'loop-risk-health',
    source: {
      type: 'script',
      path: 'scripts/check-loop-risk-health.mjs',
    },
    invocations: [
      {
        selector: 'checks-default',
        description: 'hermetic loop-risk health evaluator self-test',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'checks:loop-risk-health:self-test',
        args: ['--self-test'],
        ownership: LIVE_OWNERSHIP.NONE,
        tier: LIVE_TIERS.SELF_TEST,
        requiredInputs: [],
        requiredExecutables: ['node'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: true,
          forbidUnchecked: true,
        },
      },
      {
        selector: 'loop-risk-health-live',
        description: 'explicit external loop-risk health check from canonical config',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:loop-risk-health',
        args: [],
        inputArgs: { loopRiskConfig: '--config' },
        configProfile: 'health',
        ownership: LIVE_OWNERSHIP.EXTERNAL,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: ['loopRiskConfig'],
        requiredExecutables: ['node'],
        requiredModules: [],
        canonicalPolicy: {
          forbidSkips: true,
          forbidUnchecked: true,
        },
      },
    ],
  },
  {
    key: 'loop-risk-stress',
    source: {
      type: 'script',
      path: 'scripts/check-loop-risk-stress-live.mjs',
    },
    invocations: [
      {
        selector: 'loop-risk-stress-live',
        description: 'explicit external loop-risk stress check from canonical config',
        plan: LIVE_PLAN_TYPES.FIXED_COMMAND,
        id: 'live:loop-risk-stress',
        args: [],
        inputArgs: { loopRiskConfig: '--config' },
        configProfile: 'stress',
        ownership: LIVE_OWNERSHIP.EXTERNAL,
        tier: LIVE_TIERS.LIVE_MANUAL,
        requiredInputs: ['loopRiskConfig'],
        requiredExecutables: ['node', 'ps'],
        requiredModules: [{ specifier: 'ws', from: 'router/package.json' }],
        canonicalPolicy: {
          forbidSkips: true,
          forbidUnchecked: true,
        },
      },
    ],
  },
]);

assertLiveRegistryIntegrity(LIVE_REGISTRY);

export const LIVE_SELECTORS = Object.freeze(
  liveInvocationSelectors(LIVE_REGISTRY),
);

export function liveInvocationSelectors(
  registry,
  { tier = LIVE_TIERS.LIVE_MANUAL } = {},
) {
  assertLiveRegistryIntegrity(registry);
  return liveInvocationRecords(registry)
    .filter(({ invocation }) => invocation.tier === tier)
    .map(({ invocation }) => invocation.selector);
}

export function renderLiveSelectorHelp(registry = LIVE_REGISTRY) {
  assertLiveRegistryIntegrity(registry);
  return liveInvocationRecords(registry)
    .filter(({ invocation }) => invocation.tier === LIVE_TIERS.LIVE_MANUAL)
    .map(({ invocation }) =>
      `  ${invocation.selector.padEnd(29)} ${invocation.description}`)
    .join('\n');
}

export function assertLiveRegistryIntegrity(registry) {
  if (!Array.isArray(registry) || registry.length === 0) {
    throw new Error('live registry must contain at least one entry');
  }
  const entryKeys = new Set();
  const sourceOwners = new Set();
  const selectors = new Set();
  const identifiers = [];

  for (const entry of registry) {
    if (!isNonEmptyString(entry?.key)) {
      throw new Error(`invalid live registry entry key: ${JSON.stringify(entry)}`);
    }
    if (entryKeys.has(entry.key)) {
      throw new Error(`duplicate live registry entry key: ${entry.key}`);
    }
    entryKeys.add(entry.key);
    const sourceOwner = validateEntrySource(entry);
    if (sourceOwners.has(sourceOwner)) {
      throw new Error(`duplicate live registry source owner: ${sourceOwner}`);
    }
    sourceOwners.add(sourceOwner);
    if (!Array.isArray(entry.invocations) || entry.invocations.length === 0) {
      throw new Error(`live registry entry ${entry.key} must declare at least one invocation`);
    }

    for (const invocation of entry.invocations) {
      validateInvocation(entry, invocation);
      if (selectors.has(invocation.selector)) {
        throw new Error(`duplicate live registry selector: ${invocation.selector}`);
      }
      selectors.add(invocation.selector);
      identifiers.push(liveIdentifierDefinition(entry, invocation));
    }
  }
  assertIdentifierDefinitionsUnique(identifiers);
}

export function assertOwnershipTier(ownership, tier, source) {
  if (!Object.values(LIVE_OWNERSHIP).includes(ownership)) {
    throw new Error(`${source} has invalid ownership ${ownership}`);
  }
  if (!Object.values(LIVE_TIERS).includes(tier)) {
    throw new Error(`${source} has invalid tier ${tier}`);
  }
  if (tier === LIVE_TIERS.SELF_TEST && ownership !== LIVE_OWNERSHIP.NONE) {
    throw new Error(`${source} must use ownership none for tier self-test`);
  }
  if (
    tier === LIVE_TIERS.LIVE_MANUAL
    && ![LIVE_OWNERSHIP.EXTERNAL, LIVE_OWNERSHIP.MANAGED].includes(ownership)
  ) {
    throw new Error(`${source} must use external or managed ownership for tier live/manual`);
  }
}

export function liveInvocationRecords(registry) {
  return registry.flatMap((entry) =>
    entry.invocations.map((invocation) => ({ entry, invocation })));
}

export function liveIdentifierDefinition(entry, invocation) {
  return invocation.plan === LIVE_PLAN_TYPES.RUNTIME_FIXTURES
    ? {
      type: 'prefix',
      value: invocation.idPrefix,
      label: `${entry.key}:${invocation.idPrefix}`,
    }
    : {
      type: 'id',
      value: invocation.id,
      label: `${entry.key}:${invocation.id}`,
    };
}

export function assertIdentifierDefinitionsUnique(definitions) {
  for (let leftIndex = 0; leftIndex < definitions.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < definitions.length; rightIndex += 1) {
      const left = definitions[leftIndex];
      const right = definitions[rightIndex];
      if (identifierDefinitionsConflict(left, right)) {
        throw new Error(
          `registry task id/idPrefix conflict: ${left.label} and ${right.label}`,
        );
      }
    }
  }
}

function validateEntrySource(entry) {
  if (!entry.source || typeof entry.source !== 'object') {
    throw new Error(`live registry entry ${entry.key} requires a source owner`);
  }
  if (entry.source.type === 'script') {
    if (!isNonEmptyString(entry.source.path) || entry.source.discovery !== undefined) {
      throw new Error(`live registry entry ${entry.key} has an invalid script source`);
    }
    return `script:${entry.source.path}`;
  }
  if (entry.source.type === 'discovery') {
    if (
      !Object.values(LIVE_DISCOVERIES).includes(entry.source.discovery)
      || entry.source.path !== undefined
    ) {
      throw new Error(`live registry entry ${entry.key} has an invalid discovery source`);
    }
    return `discovery:${entry.source.discovery}`;
  }
  throw new Error(`live registry entry ${entry.key} has an invalid source type`);
}

function validateInvocation(entry, invocation) {
  if (!isNonEmptyString(invocation?.selector)) {
    throw new Error(`live registry entry ${entry.key} has an invocation without a selector`);
  }
  if (!isNonEmptyString(invocation.description)) {
    throw new Error(`live invocation ${invocation.selector} requires a help description`);
  }
  if (!Object.values(LIVE_PLAN_TYPES).includes(invocation.plan)) {
    throw new Error(`live invocation ${invocation.selector} has invalid plan ${invocation.plan}`);
  }
  assertOwnershipTier(invocation.ownership, invocation.tier, `invocation ${invocation.selector}`);
  assertUniqueStringArray(
    invocation.requiredExecutables,
    `live invocation ${invocation.selector} requiredExecutables`,
    { requireNonEmpty: true },
  );
  assertUniqueStringArray(
    invocation.requiredInputs,
    `live invocation ${invocation.selector} requiredInputs`,
  );
  assertRequiredModules(invocation.requiredModules, invocation.selector);
  for (const input of invocation.requiredInputs) {
    if (LIVE_INPUTS[input] === undefined) {
      throw new Error(`live invocation ${invocation.selector} has unknown required input ${input}`);
    }
  }
  if (
    !invocation.canonicalPolicy
    || typeof invocation.canonicalPolicy.forbidSkips !== 'boolean'
    || typeof invocation.canonicalPolicy.forbidUnchecked !== 'boolean'
  ) {
    throw new Error(`live invocation ${invocation.selector} requires a canonical policy`);
  }

  if (invocation.inputArgs !== undefined) {
    if (
      !invocation.inputArgs
      || typeof invocation.inputArgs !== 'object'
      || Array.isArray(invocation.inputArgs)
      || Object.keys(invocation.inputArgs).length === 0
      || Object.entries(invocation.inputArgs).some(([input, option]) =>
        !invocation.requiredInputs.includes(input)
        || !isNonEmptyString(option)
        || !option.startsWith('--'))
      || new Set(Object.values(invocation.inputArgs)).size
        !== Object.values(invocation.inputArgs).length
    ) {
      throw new Error(`live invocation ${invocation.selector} has invalid inputArgs`);
    }
  }
  if (invocation.configProfile !== undefined) {
    if (
      !['health', 'stress'].includes(invocation.configProfile)
      || !invocation.requiredInputs.includes('loopRiskConfig')
      || invocation.inputArgs?.loopRiskConfig !== '--config'
    ) {
      throw new Error(`live invocation ${invocation.selector} has invalid configProfile`);
    }
  }

  if (invocation.plan === LIVE_PLAN_TYPES.RUNTIME_FIXTURES) {
    if (
      entry.source.type !== 'discovery'
      || !isNonEmptyString(invocation.idPrefix)
      || invocation.id !== undefined
      || invocation.args !== undefined
    ) {
      throw new Error(`runtime fixture invocation ${invocation.selector} has an invalid shape`);
    }
    return;
  }
  if (
    entry.source.type !== 'script'
    || !isNonEmptyString(invocation.id)
    || invocation.idPrefix !== undefined
    || !Array.isArray(invocation.args)
    || !invocation.args.every((arg) => typeof arg === 'string')
  ) {
    throw new Error(`fixed command invocation ${invocation.selector} has an invalid shape`);
  }
}

function assertRequiredModules(value, selector) {
  if (!Array.isArray(value)) {
    throw new Error(`live invocation ${selector} requiredModules must be an array`);
  }
  const identities = [];
  for (const requirement of value) {
    if (
      !requirement
      || typeof requirement !== 'object'
      || Array.isArray(requirement)
      || !isNonEmptyString(requirement.specifier)
      || !isNonEmptyString(requirement.from)
      || Object.keys(requirement).some((key) => !['specifier', 'from'].includes(key))
    ) {
      throw new Error(`live invocation ${selector} has invalid requiredModules`);
    }
    identities.push(`${requirement.specifier}\0${requirement.from}`);
  }
  if (new Set(identities).size !== identities.length) {
    throw new Error(`live invocation ${selector} requiredModules must be unique`);
  }
}

function assertUniqueStringArray(value, source, { requireNonEmpty = false } = {}) {
  if (
    !Array.isArray(value)
    || (requireNonEmpty && value.length === 0)
    || !value.every(isNonEmptyString)
    || new Set(value).size !== value.length
  ) {
    throw new Error(`${source} must be a unique string array`);
  }
}

function identifierDefinitionsConflict(left, right) {
  if (left.type === 'id' && right.type === 'id') {
    return left.value === right.value;
  }
  if (left.type === 'prefix' && right.type === 'prefix') {
    return left.value.startsWith(right.value) || right.value.startsWith(left.value);
  }
  const prefix = left.type === 'prefix' ? left : right;
  const fixed = left.type === 'id' ? left : right;
  return fixed.value.startsWith(prefix.value);
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
