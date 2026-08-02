import assert from 'node:assert/strict';
import test from 'node:test';

import { compareObservations } from '../lib/router-differential/compare.mjs';

const scenario = {
  id: 'test-scenario',
  normalizations: [
    { kind: 'timestamp', path: 'frames.*.observedAt' },
  ],
  compare: {
    equal: [
      { path: 'status' },
      { path: 'frames' },
      { path: 'count' },
    ],
    sideExpected: [
      { path: 'root', sideKey: 'artifactsPath' },
    ],
    recordOnly: [
      { path: 'evidence' },
    ],
  },
};

const excludedScenario = {
  id: 'excluded-scenario',
  normalizations: [],
  compare: {
    equal: [
      {
        path: 'frames',
        exclude: [
          'frames.0.header.artifactsPath',
          'frames.0.header.serviceDb.mongoUrl',
        ],
      },
    ],
    sideExpected: [
      { path: 'frames.0.header.artifactsPath', sideKey: 'artifactsPath' },
      { path: 'frames.0.header.serviceDb.mongoUrl', sideKey: 'mongoUrl' },
    ],
    recordOnly: [],
  },
};

test('compare passes equal/sideExpected/recordOnly and reports no failures', () => {
  const report = compareObservations({
    scenario,
    tsObservation: {
      status: 200,
      frames: [
        { type: 'runtime.health', observedAt: '2026-08-02T00:00:00Z' },
      ],
      count: 0,
      root: '/tmp/ts-root',
      evidence: 'anything',
    },
    rustObservation: {
      status: 200,
      frames: [
        { type: 'runtime.health', observedAt: '2026-08-02T00:00:01Z' },
      ],
      count: 0,
      root: '/tmp/rust-root',
      evidence: 'anything',
    },
    tsSideContext: { artifactsPath: '/tmp/ts-root', ports: [] },
    rustSideContext: { artifactsPath: '/tmp/rust-root', ports: [] },
  });
  assert.deepEqual(report.failures, []);
  assert.ok(report.passed.length >= 5);
});

test('compare reports undeclared value differences as failures', () => {
  const report = compareObservations({
    scenario,
    tsObservation: { status: 200, frames: [], count: 0, root: '/a', evidence: 'x' },
    rustObservation: { status: 500, frames: [], count: 1, root: '/a', evidence: 'x' },
    tsSideContext: { artifactsPath: '/a', ports: [] },
    rustSideContext: { artifactsPath: '/a', ports: [] },
  });
  assert.deepEqual(report.failures, [
    'equal status: TS 200 !== Rust 500',
    'equal count: TS 0 !== Rust 1',
  ]);
});

test('sideExpected validates each side against its own configuration', () => {
  const report = compareObservations({
    scenario,
    tsObservation: { status: 200, frames: [], count: 0, root: '/a', evidence: 'x' },
    rustObservation: { status: 200, frames: [], count: 0, root: '/b', evidence: 'x' },
    tsSideContext: { artifactsPath: '/wrong', ports: [] },
    rustSideContext: { artifactsPath: '/b', ports: [] },
  });
  assert.deepEqual(report.failures, [
    'sideExpected root (TS): "/a" !== configured "/wrong"',
  ]);
});

test('missing equal paths are reported as missing', () => {
  const report = compareObservations({
    scenario,
    tsObservation: { status: 200, frames: [], count: 0, root: '/a', evidence: 'x' },
    rustObservation: { frames: [], count: 0, root: '/a', evidence: 'x' },
    tsSideContext: { artifactsPath: '/a', ports: [] },
    rustSideContext: { artifactsPath: '/a', ports: [] },
  });
  assert.deepEqual(report.failures, [
    'equal status: missing on Rust (ts=200, rust=undefined)',
  ]);
});

test('equal with exclude compares the remainder and sideExpected covers excluded paths', () => {
  const report = compareObservations({
    scenario: excludedScenario,
    tsObservation: {
      frames: [{
        type: 'router.bootstrap',
        header: {
          artifactsPath: '/tmp/ts',
          serviceDb: { mongoUrl: 'mongodb://ts' },
          activation: { generation: 1 },
        },
      }],
    },
    rustObservation: {
      frames: [{
        type: 'router.bootstrap',
        header: {
          artifactsPath: '/tmp/rust',
          serviceDb: { mongoUrl: 'mongodb://rust' },
          activation: { generation: 1 },
        },
      }],
    },
    tsSideContext: { artifactsPath: '/tmp/ts', mongoUrl: 'mongodb://ts', ports: [] },
    rustSideContext: { artifactsPath: '/tmp/rust', mongoUrl: 'mongodb://rust', ports: [] },
  });
  assert.deepEqual(report.failures, []);
  assert.equal(report.passed.length, 5);
});

test('equal exclude surfaces excluded-side value drift when sideExpected fails', () => {
  const report = compareObservations({
    scenario: excludedScenario,
    tsObservation: {
      frames: [{
        type: 'router.bootstrap',
        header: {
          artifactsPath: '/tmp/ts',
          serviceDb: { mongoUrl: 'mongodb://ts' },
          activation: { generation: 1 },
        },
      }],
    },
    rustObservation: {
      frames: [{
        type: 'router.bootstrap',
        header: {
          artifactsPath: '/tmp/rust',
          serviceDb: { mongoUrl: 'mongodb://rust' },
          activation: { generation: 1 },
        },
      }],
    },
    tsSideContext: { artifactsPath: '/tmp/wrong-ts', mongoUrl: 'mongodb://ts', ports: [] },
    rustSideContext: { artifactsPath: '/tmp/rust', mongoUrl: 'mongodb://rust', ports: [] },
  });
  assert.deepEqual(report.failures, [
    'sideExpected frames.0.header.artifactsPath (TS): "/tmp/ts" !== configured "/tmp/wrong-ts"',
  ]);
});
