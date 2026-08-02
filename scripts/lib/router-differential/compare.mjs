// Differential comparison engine (plan §9).
//
// Each scenario declares:
// - normalizations: explicit kinds applied to explicit observation paths
//   (uuid / timestamp / port / logOrder only);
// - compare.equal: paths that must deep-equal across TS and Rust after
//   normalization;
// - compare.sideExpected: paths that must equal the side's own configured
//   value (artifact root, mongo URL, ports, runtime home), which is how the
//   harness honors "independent artifact root / runtime home / Mongo
//   namespace" without normalizing those differences away;
// - compare.recordOnly: evidence paths captured but not asserted.

import assert from 'node:assert/strict';

import {
  normalizeObservationPath,
} from './normalize.mjs';

export function normalizeObservations(observation, scenario, sideContext) {
  let normalized = observation;
  for (const normalization of scenario.normalizations ?? []) {
    normalized = normalizeObservationPath(normalized, normalization.path, {
      kind: normalization.kind,
      ports: sideContext.ports ?? [],
    });
  }
  return normalized;
}

export function compareObservations({
  scenario,
  tsObservation,
  rustObservation,
  tsSideContext,
  rustSideContext,
}) {
  const tsNormalized = normalizeObservations(tsObservation, scenario, tsSideContext);
  const rustNormalized = normalizeObservations(rustObservation, scenario, rustSideContext);
  const failures = [];
  const passed = [];
  const contract = scenario.compare ?? {};

  for (const entry of contract.equal ?? []) {
    const tsValue = readPath(tsNormalized, entry.path);
    const rustValue = readPath(rustNormalized, entry.path);
    const tsPresent = tsValue !== undefined;
    const rustPresent = rustValue !== undefined;
    if (!tsPresent || !rustPresent) {
      failures.push(
        `equal ${entry.path}: missing on ${!tsPresent ? 'TS' : ''}${!rustPresent ? 'Rust' : ''}`
        + ` (ts=${JSON.stringify(tsValue)}, rust=${JSON.stringify(rustValue)})`,
      );
      continue;
    }
    let tsCompared = tsValue;
    let rustCompared = rustValue;
    for (const exclude of entry.exclude ?? []) {
      const relative = exclude.slice(entry.path.length + 1).split('.');
      tsCompared = deletePath(tsCompared, relative);
      rustCompared = deletePath(rustCompared, relative);
    }
    try {
      assert.deepStrictEqual(tsCompared, rustCompared);
      passed.push(`equal ${entry.path}`);
    } catch {
      failures.push(
        `equal ${entry.path}: TS ${JSON.stringify(tsCompared)} !== Rust ${JSON.stringify(rustCompared)}`,
      );
    }
  }

  for (const entry of contract.sideExpected ?? []) {
    for (const [label, observation, sideContext] of [
      ['TS', tsNormalized, tsSideContext],
      ['Rust', rustNormalized, rustSideContext],
    ]) {
      const actual = readPath(observation, entry.path);
      const expected = sideContext[entry.sideKey];
      if (actual === undefined) {
        failures.push(`sideExpected ${entry.path} (${label}): missing`);
        continue;
      }
      if (actual !== expected) {
        failures.push(
          `sideExpected ${entry.path} (${label}): ${JSON.stringify(actual)} !== configured ${JSON.stringify(expected)}`,
        );
      } else {
        passed.push(`sideExpected ${entry.path} (${label})`);
      }
    }
  }

  for (const entry of contract.recordOnly ?? []) {
    for (const [label, observation] of [
      ['TS', tsNormalized],
      ['Rust', rustNormalized],
    ]) {
      try {
        readPath(observation, entry.path);
        passed.push(`recordOnly ${entry.path} (${label})`);
      } catch (error) {
        failures.push(`recordOnly ${entry.path} (${label}): ${error.message}`);
      }
    }
  }

  return {
    scenarioId: scenario.id,
    passed,
    failures,
    tsObservation: tsNormalized,
    rustObservation: rustNormalized,
  };
}

export function renderDifferentialReport(report) {
  const lines = [
    `differential scenario ${report.scenarioId}:`,
    `  passed: ${report.passed.length}`,
    `  failed: ${report.failures.length}`,
  ];
  for (const failure of report.failures) {
    lines.push(`  FAIL ${failure}`);
  }
  return lines.join('\n');
}

export function readPath(observation, path) {
  const segments = path.split('.').filter((segment) => segment.length > 0);
  if (segments.length === 0) {
    return observation;
  }
  let value = observation;
  for (const segment of segments) {
    if (value === null || typeof value !== 'object') {
      throw new Error(`path ${path} cannot descend into ${typeof value} at ${segment}`);
    }
    if (Array.isArray(value)) {
      const index = Number(segment);
      if (!Number.isInteger(index) || index < 0 || index >= value.length) {
        throw new Error(`path ${path} array index ${segment} is out of range`);
      }
      value = value[index];
      continue;
    }
    if (!Object.hasOwn(value, segment)) {
      return undefined;
    }
    value = value[segment];
  }
  return value;
}

function deletePath(value, segments) {
  if (segments.length === 0) {
    return undefined;
  }
  const [head, ...rest] = segments;
  if (Array.isArray(value)) {
    const index = Number(head);
    if (!Number.isInteger(index) || index < 0 || index >= value.length) {
      return value;
    }
    const next = [...value];
    next[index] = deletePath(value[index], rest);
    return next;
  }
  if (value === null || typeof value !== 'object') {
    return value;
  }
  if (!Object.hasOwn(value, head)) {
    return value;
  }
  if (rest.length === 0) {
    const { [head]: _removed, ...next } = value;
    return next;
  }
  return {
    ...value,
    [head]: deletePath(value[head], rest),
  };
}
