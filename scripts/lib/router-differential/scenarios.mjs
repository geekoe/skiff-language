// Scenario inventory owner for the differential harness.
//
// The machine-readable inventory lives at
// `scripts/fixtures/router-differential/scenario-inventory.json` and is the
// single source of truth for scenario ids/status/lanes/observation types.
// The compare contract for each scenario is declared in the same document
// (normalizations + equal + sideExpected + recordOnly paths), so the
// persisted inventory fully describes what the harness asserts.

import { readFile } from 'node:fs/promises';

import {
  DIFFERENTIAL_SCHEMA_VERSION,
  scenarioInventoryPath,
} from './constants.mjs';
import {
  NORMALIZATION_KINDS,
  assertNormalizationKind,
} from './normalize.mjs';

export const SCENARIO_STATUSES = Object.freeze(['runnable', 'planned']);
export const OBSERVATION_TYPES = Object.freeze([
  'http',
  'clientWs',
  'runtimeFrames',
  'health',
  'mongoState',
  'mongoAudit',
  'terminal',
  'logs',
]);
export const SIDE_KEYS = Object.freeze([
  'artifactsPath',
  'mongoUrl',
  'httpPort',
  'runtimePort',
  'relayPort',
  'devHome',
  'runtimeHome',
]);

export async function loadScenarioInventory({
  skiffRoot,
  read = readFile,
} = {}) {
  const path = scenarioInventoryPath(skiffRoot);
  const text = await read(path, 'utf8');
  const inventory = JSON.parse(text);
  assertScenarioInventory(inventory);
  return inventory;
}

export function assertScenarioInventory(inventory) {
  if (
    inventory === null
    || typeof inventory !== 'object'
    || Array.isArray(inventory)
    || inventory.schemaVersion !== DIFFERENTIAL_SCHEMA_VERSION
  ) {
    throw new Error(`scenario inventory requires schemaVersion ${DIFFERENTIAL_SCHEMA_VERSION}`);
  }
  if (typeof inventory.baseline !== 'string' || !/^[0-9a-f]{40}$/.test(inventory.baseline)) {
    throw new Error('scenario inventory requires an exact 40-hex baseline commit');
  }
  if (!Array.isArray(inventory.scenarios) || inventory.scenarios.length === 0) {
    throw new Error('scenario inventory must declare at least one scenario');
  }
  const ids = new Set();
  for (const scenario of inventory.scenarios) {
    assertScenario(scenario);
    if (ids.has(scenario.id)) {
      throw new Error(`duplicate scenario id ${scenario.id}`);
    }
    ids.add(scenario.id);
  }
}

function assertScenario(scenario) {
  if (
    scenario === null
    || typeof scenario !== 'object'
    || Array.isArray(scenario)
    || typeof scenario.id !== 'string'
    || scenario.id.trim().length === 0
  ) {
    throw new Error('scenario requires a non-empty id');
  }
  if (!SCENARIO_STATUSES.includes(scenario.status)) {
    throw new Error(
      `scenario ${scenario.id} status must be one of ${SCENARIO_STATUSES.join(', ')}`,
    );
  }
  if (typeof scenario.lane !== 'string' || scenario.lane.trim().length === 0) {
    throw new Error(`scenario ${scenario.id} requires a lane`);
  }
  if (typeof scenario.description !== 'string' || scenario.description.length === 0) {
    throw new Error(`scenario ${scenario.id} requires a description`);
  }
  if (
    !Array.isArray(scenario.observationTypes)
    || scenario.observationTypes.length === 0
    || scenario.observationTypes.some((type) => !OBSERVATION_TYPES.includes(type))
  ) {
    throw new Error(
      `scenario ${scenario.id} observationTypes must be a non-empty subset of ${OBSERVATION_TYPES.join(', ')}`,
    );
  }
  if (scenario.normalizations !== undefined) {
    if (!Array.isArray(scenario.normalizations)) {
      throw new Error(`scenario ${scenario.id} normalizations must be an array`);
    }
    for (const normalization of scenario.normalizations) {
      if (
        normalization === null
        || typeof normalization !== 'object'
        || typeof normalization.kind !== 'string'
        || typeof normalization.path !== 'string'
        || normalization.path.length === 0
      ) {
        throw new Error(`scenario ${scenario.id} has an invalid normalization`);
      }
      assertNormalizationKind(normalization.kind);
    }
  }
  if (scenario.compare === undefined) {
    throw new Error(`scenario ${scenario.id} requires a compare contract`);
  }
  const { equal, sideExpected, recordOnly } = scenario.compare;
  for (const [label, entries] of [
    ['equal', equal],
    ['sideExpected', sideExpected],
    ['recordOnly', recordOnly],
  ]) {
    if (entries === undefined) {
      continue;
    }
    if (
      !Array.isArray(entries)
      || entries.some((entry) =>
        entry === null
        || typeof entry !== 'object'
        || typeof entry.path !== 'string'
        || entry.path.length === 0)
    ) {
      throw new Error(`scenario ${scenario.id} compare.${label} must be a path array`);
    }
    if (label === 'sideExpected') {
      for (const entry of entries) {
        if (!SIDE_KEYS.includes(entry.sideKey)) {
          throw new Error(
            `scenario ${scenario.id} sideExpected sideKey must be one of ${SIDE_KEYS.join(', ')}`,
          );
        }
      }
    }
  }
  for (const entry of equal ?? []) {
    for (const exclude of entry.exclude ?? []) {
      if (typeof exclude !== 'string' || !exclude.startsWith(`${entry.path}.`)) {
        throw new Error(
          `scenario ${scenario.id} equal ${entry.path} exclude must be a descendant path`,
        );
      }
      const covered = (sideExpected ?? []).some((candidate) => candidate.path === exclude)
        || (recordOnly ?? []).some((candidate) => candidate.path === exclude);
      if (!covered) {
        throw new Error(
          `scenario ${scenario.id} equal ${entry.path} excludes ${exclude} `
          + 'which is not covered by compare.sideExpected or compare.recordOnly',
        );
      }
    }
  }
}

export function assertSelectedScenarioRunnable(scenario) {
  if (scenario.status !== 'runnable') {
    throw new Error(
      `scenario ${scenario.id} is ${scenario.status}; only runnable scenarios can execute`,
    );
  }
}
