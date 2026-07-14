import assert from 'node:assert/strict';
import test from 'node:test';

import { checkPublicApi } from '../lib/crate-public-api-graph.mjs';
import {
  GRAPH_CASES,
  GRAPH_MATRIX_EXPECTED_IDS,
} from './helpers/crate-public-api-graph-cases.mjs';

const config = {
  crateName: 'matrix-crate',
  allowedCrates: ['matrix-crate', 'std', 'core', 'alloc', 'allowed-dep'],
};

test('graph module consumes the same exhaustive matrix characterized against the monolith', () => {
  assert.deepEqual(GRAPH_CASES.map(({ id }) => id), GRAPH_MATRIX_EXPECTED_IDS);
  assert.equal(new Set(GRAPH_MATRIX_EXPECTED_IDS).size, GRAPH_MATRIX_EXPECTED_IDS.length);
});

for (const caseDefinition of GRAPH_CASES) {
  test(`graph module matrix: ${caseDefinition.id}`, () => {
    const result = checkPublicApi(caseDefinition.rustdoc, config);
    assert.equal(result.crateName, config.crateName);
    assert.deepEqual(
      result.violations,
      caseDefinition.expectedViolations,
      caseDefinition.id,
    );
  });
}

test('graph fails closed when rustdoc JSON has no root item', () => {
  assert.throws(
    () => checkPublicApi({ index: {}, paths: {} }, config),
    /rustdoc JSON is missing root item id/,
  );
});
