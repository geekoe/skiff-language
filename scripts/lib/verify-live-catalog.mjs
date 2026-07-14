import { CHECKER_REGISTRY } from './verify-checkers.mjs';
import { discoverCheckerScripts } from './verify-discovery.mjs';
import {
  LIVE_REGISTRY,
  LIVE_TIERS,
  assertIdentifierDefinitionsUnique,
  assertLiveRegistryIntegrity,
  liveIdentifierDefinition,
  liveInvocationRecords,
} from './verify-live-registry.mjs';
import {
  ORDINARY_LEAF_SELECTORS,
  ORDINARY_SELECTOR_NAMES,
} from './verify-selector-graph.mjs';

export async function assertVerifyCatalogComplete(root, {
  checkerRegistry = CHECKER_REGISTRY,
  liveRegistry = LIVE_REGISTRY,
} = {}) {
  assertLiveRegistryIntegrity(liveRegistry);
  assertSelectorNamespaceDisjoint(checkerRegistry, liveRegistry);
  await assertCheckerPathCatalogComplete(root, checkerRegistry, liveRegistry);
  assertInvocationIdentifiersUnique(checkerRegistry, liveRegistry);
}

function assertSelectorNamespaceDisjoint(checkerRegistry, liveRegistry) {
  const ordinarySelectors = new Set(ORDINARY_SELECTOR_NAMES);
  for (const entry of checkerRegistry) {
    if (!Array.isArray(entry.invocations)) {
      throw new Error(`checker registry entry ${entry.path} has invalid invocations`);
    }
    for (const invocation of entry.invocations) {
      if (!ordinarySelectors.has(invocation.selector)) {
        throw new Error(
          `checker invocation ${entry.path}:${invocation.id} uses unknown ordinary selector ${invocation.selector}`,
        );
      }
    }
  }
  const conflicts = liveInvocationRecords(liveRegistry)
    .filter(({ invocation }) => invocation.tier === LIVE_TIERS.LIVE_MANUAL)
    .map(({ invocation }) => invocation.selector)
    .filter((selector) => ordinarySelectors.has(selector));
  if (conflicts.length > 0) {
    throw new Error(
      `live selector conflicts with ordinary verify selector namespace: ${conflicts.join(', ')}`,
    );
  }
  const invalidSelfTests = liveInvocationRecords(liveRegistry)
    .filter(({ invocation }) => invocation.tier === LIVE_TIERS.SELF_TEST)
    .map(({ invocation }) => invocation.selector)
    .filter((selector) => !ORDINARY_LEAF_SELECTORS.includes(selector));
  if (invalidSelfTests.length > 0) {
    throw new Error(
      `registry self-test must target ordinary leaf selector: ${invalidSelfTests.join(', ')}`,
    );
  }
}

async function assertCheckerPathCatalogComplete(root, checkerRegistry, liveRegistry) {
  const discovered = await discoverCheckerScripts(root);
  const pathCounts = new Map();
  for (const entry of checkerRegistry) {
    if (!isNonEmptyString(entry?.path)) {
      throw new Error(`invalid checker registry path: ${JSON.stringify(entry)}`);
    }
    incrementCount(pathCounts, entry.path);
  }
  for (const entry of liveRegistry) {
    if (entry.source.type === 'script') {
      incrementCount(pathCounts, entry.source.path);
    }
  }

  const duplicate = [...pathCounts]
    .filter(([, count]) => count !== 1)
    .map(([path, count]) => `${path} (${count})`);
  const missing = discovered.filter((path) => !pathCounts.has(path));
  const stale = [...pathCounts.keys()].filter((path) => !discovered.includes(path));
  if (duplicate.length > 0 || missing.length > 0 || stale.length > 0) {
    throw new Error([
      duplicate.length > 0
        ? `checker path count must be exactly one: ${duplicate.join(', ')}`
        : '',
      missing.length > 0 ? `unclassified checker(s): ${missing.join(', ')}` : '',
      stale.length > 0 ? `missing registered checker(s): ${stale.join(', ')}` : '',
    ].filter(Boolean).join('; '));
  }
}

function assertInvocationIdentifiersUnique(checkerRegistry, liveRegistry) {
  const identifiers = [];
  for (const entry of checkerRegistry) {
    for (const invocation of entry.invocations) {
      if (!isNonEmptyString(invocation?.id)) {
        throw new Error(`checker invocation for ${entry.path} requires a non-empty id`);
      }
      identifiers.push({
        type: 'id',
        value: invocation.id,
        label: `${entry.path}:${invocation.id}`,
      });
    }
  }
  for (const { entry, invocation } of liveInvocationRecords(liveRegistry)) {
    identifiers.push(liveIdentifierDefinition(entry, invocation));
  }
  assertIdentifierDefinitionsUnique(identifiers);
}

function incrementCount(counts, value) {
  counts.set(value, (counts.get(value) ?? 0) + 1);
}

function isNonEmptyString(value) {
  return typeof value === 'string' && value.trim().length > 0;
}
