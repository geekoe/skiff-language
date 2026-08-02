// Scenario inventory owner for the actor parity differential (plan §9).
//
// The machine-readable inventory lives at
// `scripts/fixtures/router-differential/actor_parity_inventory.json` and uses
// the same schema/compare contract as the shared W-differential inventory.
// The shared inventory document itself stays owned by the differential
// extension node; this module only reads/validates the actor_parity_* file.

import { readFile } from 'node:fs/promises';

import {
  ACTOR_PARITY_BASELINE,
  ACTOR_PARITY_INVENTORY_REPO_PATH,
  ACTOR_PARITY_SCHEMA_VERSION,
} from './actor_parity_constants.mjs';
import { assertNormalizationKind } from './normalize.mjs';
import { SIDE_KEYS } from './scenarios.mjs';

export const ACTOR_PARITY_SCENARIO_STATUSES = Object.freeze([
  'runnable',
  'planned',
]);
export const ACTOR_PARITY_OBSERVATION_TYPES = Object.freeze([
  'http',
  'clientWs',
  'runtimeFrames',
  'health',
  'mongoState',
  'mongoAudit',
  'terminal',
  'logs',
]);

export async function loadActorParityInventory({
  skiffRoot,
  read = readFile,
} = {}) {
  const path = `${skiffRoot}/${ACTOR_PARITY_INVENTORY_REPO_PATH}`;
  const text = await read(path, 'utf8');
  const inventory = JSON.parse(text);
  assertActorParityInventory(inventory);
  return inventory;
}

export function assertActorParityInventory(inventory) {
  if (
    inventory === null
    || typeof inventory !== 'object'
    || Array.isArray(inventory)
    || inventory.schemaVersion !== ACTOR_PARITY_SCHEMA_VERSION
  ) {
    throw new Error(
      `actor parity inventory requires schemaVersion ${ACTOR_PARITY_SCHEMA_VERSION}`,
    );
  }
  if (inventory.baseline !== ACTOR_PARITY_BASELINE) {
    throw new Error(
      `actor parity inventory requires exact baseline ${ACTOR_PARITY_BASELINE}`,
    );
  }
  if (!Array.isArray(inventory.scenarios) || inventory.scenarios.length === 0) {
    throw new Error('actor parity inventory must declare at least one scenario');
  }
  const ids = new Set();
  for (const scenario of inventory.scenarios) {
    assertActorParityScenario(scenario);
    if (ids.has(scenario.id)) {
      throw new Error(`duplicate actor parity scenario id ${scenario.id}`);
    }
    ids.add(scenario.id);
  }
}

function assertActorParityScenario(scenario) {
  if (
    scenario === null
    || typeof scenario !== 'object'
    || Array.isArray(scenario)
    || typeof scenario.id !== 'string'
    || scenario.id.trim().length === 0
  ) {
    throw new Error('actor parity scenario requires a non-empty id');
  }
  if (!ACTOR_PARITY_SCENARIO_STATUSES.includes(scenario.status)) {
    throw new Error(
      `actor parity scenario ${scenario.id} status must be one of `
      + ACTOR_PARITY_SCENARIO_STATUSES.join(', '),
    );
  }
  if (typeof scenario.lane !== 'string' || scenario.lane.trim().length === 0) {
    throw new Error(`actor parity scenario ${scenario.id} requires a lane`);
  }
  if (
    typeof scenario.description !== 'string'
    || scenario.description.length === 0
  ) {
    throw new Error(`actor parity scenario ${scenario.id} requires a description`);
  }
  if (
    !Array.isArray(scenario.observationTypes)
    || scenario.observationTypes.length === 0
    || scenario.observationTypes.some(
      (type) => !ACTOR_PARITY_OBSERVATION_TYPES.includes(type),
    )
  ) {
    throw new Error(
      `actor parity scenario ${scenario.id} observationTypes must be a non-empty subset of `
      + ACTOR_PARITY_OBSERVATION_TYPES.join(', '),
    );
  }
  for (const normalization of scenario.normalizations ?? []) {
    if (
      normalization === null
      || typeof normalization !== 'object'
      || typeof normalization.kind !== 'string'
      || typeof normalization.path !== 'string'
      || normalization.path.length === 0
    ) {
      throw new Error(
        `actor parity scenario ${scenario.id} has an invalid normalization`,
      );
    }
    assertNormalizationKind(normalization.kind);
  }
  if (scenario.compare === undefined) {
    throw new Error(`actor parity scenario ${scenario.id} requires a compare contract`);
  }
  assertActorParityKnownDifferences(scenario);
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
      throw new Error(
        `actor parity scenario ${scenario.id} compare.${label} must be a path array`,
      );
    }
    if (label === 'sideExpected') {
      for (const entry of entries) {
        if (!SIDE_KEYS.includes(entry.sideKey)) {
          throw new Error(
            `actor parity scenario ${scenario.id} sideExpected sideKey must be one of `
            + SIDE_KEYS.join(', '),
          );
        }
      }
    }
  }
  for (const entry of equal ?? []) {
    for (const exclude of entry.exclude ?? []) {
      if (typeof exclude !== 'string' || !exclude.startsWith(`${entry.path}.`)) {
        throw new Error(
          `actor parity scenario ${scenario.id} equal ${entry.path} exclude must be a descendant path`,
        );
      }
      const covered = (sideExpected ?? []).some((candidate) => candidate.path === exclude)
        || (recordOnly ?? []).some((candidate) => candidate.path === exclude);
      if (!covered) {
        throw new Error(
          `actor parity scenario ${scenario.id} equal ${entry.path} excludes ${exclude} `
          + 'which is not covered by compare.sideExpected or compare.recordOnly',
        );
      }
    }
  }
}

function assertActorParityKnownDifferences(scenario) {
  const known = scenario.knownDifferences ?? [];
  if (!Array.isArray(known)) {
    throw new Error(`actor parity scenario ${scenario.id} knownDifferences must be an array`);
  }
  for (const difference of known) {
    if (
      difference === null
      || typeof difference !== 'object'
      || typeof difference.id !== 'string'
      || difference.id.trim().length === 0
    ) {
      throw new Error(
        `actor parity scenario ${scenario.id} knownDifferences entries require a non-empty id`,
      );
    }
    if (
      !Array.isArray(difference.paths)
      || difference.paths.length === 0
      || difference.paths.some((path) => typeof path !== 'string' || path.length === 0)
    ) {
      throw new Error(
        `actor parity scenario ${scenario.id} knownDifferences ${difference.id} requires non-empty paths`,
      );
    }
    if (typeof difference.accepted !== 'boolean') {
      throw new Error(
        `actor parity scenario ${scenario.id} knownDifferences ${difference.id} requires an accepted boolean`,
      );
    }
    if (typeof difference.description !== 'string' || difference.description.length === 0) {
      throw new Error(
        `actor parity scenario ${scenario.id} knownDifferences ${difference.id} requires a description`,
      );
    }
  }
  const followUps = scenario.nonBlockingFollowUps ?? [];
  if (!Array.isArray(followUps)) {
    throw new Error(
      `actor parity scenario ${scenario.id} nonBlockingFollowUps must be an array`,
    );
  }
  for (const followUp of followUps) {
    if (
      followUp === null
      || typeof followUp !== 'object'
      || typeof followUp.id !== 'string'
      || followUp.id.trim().length === 0
      || typeof followUp.description !== 'string'
      || followUp.description.length === 0
    ) {
      throw new Error(
        `actor parity scenario ${scenario.id} nonBlockingFollowUps entries require an id and description`,
      );
    }
  }
}

export function assertActorParityScenarioRunnable(scenario) {
  if (scenario.status !== 'runnable') {
    throw new Error(
      `actor parity scenario ${scenario.id} is ${scenario.status}; `
      + 'only runnable scenarios can execute',
    );
  }
}
