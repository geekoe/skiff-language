const selectorGraph = {
  publicSelectors: [
    'verify',
    'node',
    'rust',
    'router',
    'telemetry',
    'scripts',
    'scripts-dev-sync',
    'scripts-syntax',
    'vscode',
    'checks',
    'type-check',
    'compiler-boundaries',
  ],
  expansions: {
    verify: ['rust', 'node', 'checks'],
    checks: ['compiler-boundaries', 'checks-default'],
    node: ['router', 'telemetry', 'scripts', 'vscode'],
    router: ['router-type-check', 'router-test'],
    telemetry: ['telemetry-type-check', 'telemetry-test'],
    scripts: ['scripts-syntax', 'scripts-tests', 'scripts-dev-sync'],
    vscode: ['vscode-type-check', 'vscode-grammar'],
    'type-check': [
      'router-type-check',
      'telemetry-type-check',
      'scripts-syntax',
      'vscode-type-check',
    ],
  },
};

export const VERIFY_SELECTOR_GRAPH = deepFreeze(selectorGraph);

assertSelectorGraphIntegrity(VERIFY_SELECTOR_GRAPH);

export const ORDINARY_PUBLIC_SELECTORS = VERIFY_SELECTOR_GRAPH.publicSelectors;

export const ORDINARY_SELECTOR_NAMES = Object.freeze(
  selectorNames(VERIFY_SELECTOR_GRAPH),
);

export const ORDINARY_LEAF_SELECTORS = Object.freeze(
  leafSelectors(VERIFY_SELECTOR_GRAPH),
);

export function assertOrdinaryPhaseBuilderCoverage(builders) {
  if (!builders || typeof builders !== 'object' || Array.isArray(builders)) {
    throw new Error('ordinary verify phase builders must be an object');
  }
  const actual = Object.keys(builders).sort();
  const expected = [...ORDINARY_LEAF_SELECTORS].sort();
  const missing = expected.filter((selector) => !actual.includes(selector));
  const unexpected = actual.filter((selector) => !expected.includes(selector));
  if (missing.length > 0 || unexpected.length > 0) {
    throw new Error([
      missing.length > 0
        ? `missing ordinary verify phase builder(s): ${missing.join(', ')}`
        : '',
      unexpected.length > 0
        ? `unexpected ordinary verify phase builder(s): ${unexpected.join(', ')}`
        : '',
    ].filter(Boolean).join('; '));
  }
}

function assertSelectorGraphIntegrity(graph) {
  if (
    !graph
    || !Array.isArray(graph.publicSelectors)
    || graph.publicSelectors.length === 0
    || !graph.expansions
    || typeof graph.expansions !== 'object'
    || Array.isArray(graph.expansions)
  ) {
    throw new Error('invalid ordinary verify selector graph');
  }
  assertUniqueSelectors(graph.publicSelectors, 'ordinary public selectors');
  for (const [selector, children] of Object.entries(graph.expansions)) {
    if (!isNonEmptyString(selector)) {
      throw new Error('ordinary selector expansion requires a non-empty selector');
    }
    assertUniqueSelectors(children, `ordinary selector expansion ${selector}`);
    if (children.length === 0) {
      throw new Error(`ordinary selector expansion ${selector} must not be empty`);
    }
  }
  for (const selector of graph.publicSelectors) {
    collectLeaves(selector, graph.expansions, new Set(), new Set());
  }
  const reachable = new Set();
  for (const selector of graph.publicSelectors) {
    collectReachable(selector, graph.expansions, reachable);
  }
  const orphaned = Object.keys(graph.expansions)
    .filter((selector) => !reachable.has(selector));
  if (orphaned.length > 0) {
    throw new Error(`orphaned ordinary selector expansion(s): ${orphaned.join(', ')}`);
  }
}

function selectorNames(graph) {
  const names = new Set(graph.publicSelectors);
  for (const [selector, children] of Object.entries(graph.expansions)) {
    names.add(selector);
    for (const child of children) {
      names.add(child);
    }
  }
  return [...names];
}

function leafSelectors(graph) {
  const leaves = new Set();
  for (const selector of graph.publicSelectors) {
    collectLeaves(selector, graph.expansions, leaves, new Set());
  }
  return [...leaves];
}

function collectLeaves(selector, expansions, leaves, active) {
  const expansion = expansions[selector];
  if (expansion === undefined) {
    leaves.add(selector);
    return;
  }
  if (active.has(selector)) {
    throw new Error(
      `cyclic ordinary verify selector expansion: ${[...active, selector].join(' -> ')}`,
    );
  }
  const nextActive = new Set(active).add(selector);
  for (const child of expansion) {
    collectLeaves(child, expansions, leaves, nextActive);
  }
}

function collectReachable(selector, expansions, reachable) {
  if (reachable.has(selector)) {
    return;
  }
  reachable.add(selector);
  for (const child of expansions[selector] ?? []) {
    collectReachable(child, expansions, reachable);
  }
}

function assertUniqueSelectors(selectors, source) {
  if (
    !Array.isArray(selectors)
    || !selectors.every(isNonEmptyString)
    || new Set(selectors).size !== selectors.length
  ) {
    throw new Error(`${source} must be a unique non-empty string array`);
  }
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
