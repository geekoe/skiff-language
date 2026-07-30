import { readFile } from 'node:fs/promises';

import { afterEach, describe, expect, it } from 'vitest';

import {
  InMemoryTelemetryStore,
  type TelemetryStore
} from '../src/mongoStore.js';
import {
  validateTelemetryBatch,
  type TelemetryBatchEnvelope,
  type TelemetryEvent
} from '../src/protocol.js';
import { TelemetryServer } from '../src/server.js';

interface ScenarioFixture {
  corpusCase: string;
  traceId: string;
  errorId: string;
  privateSentinel: string;
  hops: HopExpectation[];
  externalSafeMessage: string;
}

interface HopExpectation {
  name: 'A' | 'B' | 'C';
  serviceId: string;
  activationId: string;
  operationId: string;
  source: unknown;
  localStack: unknown[];
}

interface WireCorpus {
  schemaVersion: number;
  validCases: Array<{
    name: string;
    expected: Record<string, unknown>;
  }>;
}

let activeServer: TelemetryServer | undefined;
const { scenario, corpus } = await readFixtures();

afterEach(async () => {
  await activeServer?.close();
  activeServer = undefined;
});

describe('P5-F345 restricted telemetry convergence', () => {
  it('keeps ordinary routes operational and exposes exactly three redacted hops store-only', async () => {
    const expected = corpus.validCases.find(
      (candidate) => candidate.name === scenario.corpusCase
    )?.expected;
    expect(expected).toMatchObject({
      kind: 'internalError',
      traceId: scenario.traceId,
      errorId: scenario.errorId
    });

    const batch = telemetryBatch();
    const validation = validateTelemetryBatch(batch, ['log', 'trace']);
    expect(validation).toMatchObject({ ok: true });
    if (!validation.ok) {
      throw new Error(validation.error);
    }

    const store: TelemetryStore = new InMemoryTelemetryStore();
    const server = new TelemetryServer({ port: 0, store });
    activeServer = server;
    const listen = await server.listen();
    await expect(store.insertBatch(validation.value)).resolves.toEqual({
      inserted: 4,
      duplicate: false
    });

    const storeLogs = await store.queryLogs({
      traceId: scenario.traceId,
      errorId: scenario.errorId
    });
    const storeTrace = await store.queryTrace(scenario.traceId, scenario.errorId);
    const storeTraces = await store.queryTraces({
      traceId: scenario.traceId,
      errorId: scenario.errorId
    });
    for (const ordinary of [storeLogs, storeTrace, storeTraces]) {
      expect(ordinary).toHaveLength(1);
      expect(ordinary[0]).toMatchObject({
        visibility: 'operational',
        traceId: scenario.traceId,
        errorId: scenario.errorId,
        error: {
          kind: 'fixedService',
          causeKind: 'internalError'
        }
      });
      expect(JSON.stringify(ordinary)).not.toContain('stack');
      expect(JSON.stringify(ordinary)).not.toContain(scenario.privateSentinel);
    }

    const logs = readEvents(
      await fetchJson(
        `${listen.httpUrl}/logs?traceId=${encodeURIComponent(scenario.traceId)}&errorId=${encodeURIComponent(scenario.errorId)}`
      )
    );
    const traces = readEvents(
      await fetchJson(
        `${listen.httpUrl}/traces?traceId=${encodeURIComponent(scenario.traceId)}&errorId=${encodeURIComponent(scenario.errorId)}`
      )
    );
    const trace = readEvents(
      await fetchJson(
        `${listen.httpUrl}/traces/${encodeURIComponent(scenario.traceId)}?errorId=${encodeURIComponent(scenario.errorId)}`
      )
    );
    for (const ordinary of [logs, traces, trace]) {
      expect(ordinary).toHaveLength(1);
      expect(ordinary[0]).toMatchObject({
        visibility: 'operational',
        traceId: scenario.traceId,
        errorId: scenario.errorId
      });
      expect(JSON.stringify(ordinary)).not.toContain('stack');
      expect(JSON.stringify(ordinary)).not.toContain(scenario.privateSentinel);
    }

    const restrictedRoute = await fetch(
      `${listen.httpUrl}/restricted-diagnostics?traceId=${encodeURIComponent(scenario.traceId)}`
    );
    expect(restrictedRoute.status).toBe(404);

    const restricted = await store.queryRestrictedDiagnostics({
      traceId: scenario.traceId,
      errorId: scenario.errorId
    });
    expect(restricted).toHaveLength(3);
    expect(restricted.map((event) => event.serviceId)).toEqual(
      scenario.hops.map((hop) => hop.serviceId)
    );
    for (const [index, hop] of scenario.hops.entries()) {
      const event = restricted[index]!;
      expect(event).toMatchObject({
        visibility: 'restricted',
        serviceId: hop.serviceId,
        activationIdentity: hop.activationId,
        target: hop.operationId,
        traceId: scenario.traceId,
        errorId: scenario.errorId,
        error: {
          kind: 'restrictedServiceDiagnostic',
          causeKind: 'internalError',
          source: hop.source,
          stack: expectedStack(index),
          secret: '[REDACTED]'
        }
      });
      const serialized = JSON.stringify(event);
      expect(serialized).not.toContain(scenario.privateSentinel);
      expect(serialized).not.toContain('modulePath');
      expect(serialized).not.toContain('function');
      expect(serialized).not.toContain('encodedPayload');
    }
    await expect(store.queryRestrictedDiagnostics({})).rejects.toThrow(
      'requires a non-empty traceId or errorId'
    );
  });
});

function telemetryBatch(): TelemetryBatchEnvelope {
  const operational: TelemetryEvent = {
    topic: 'log',
    ts: '2026-07-26T01:00:00.000Z',
    source: 'runtime',
    visibility: 'operational',
    serviceId: scenario.hops[2]!.serviceId,
    activationIdentity: scenario.hops[2]!.activationId,
    target: scenario.hops[2]!.operationId,
    traceId: scenario.traceId,
    errorId: scenario.errorId,
    level: 'error',
    name: 'request.error',
    message: scenario.externalSafeMessage,
    attrs: {
      instructionCount: 17
    },
    error: {
      kind: 'fixedService',
      causeKind: 'internalError'
    }
  };
  const restricted = scenario.hops.map<TelemetryEvent>((hop, index) => ({
    topic: 'trace',
    ts: `2026-07-26T01:00:00.00${index + 1}Z`,
    source: 'runtime',
    visibility: 'restricted',
    serviceId: hop.serviceId,
    activationIdentity: hop.activationId,
    target: hop.operationId,
    traceId: scenario.traceId,
    errorId: scenario.errorId,
    level: 'error',
    name: 'service.error.restricted',
    error: {
      kind: 'restrictedServiceDiagnostic',
      causeKind: 'internalError',
      source: hop.source,
      stack: expectedStack(index),
      secret: scenario.privateSentinel
    }
  }));
  return {
    type: 'telemetry.batch',
    producerId: 'p5-f345-convergence',
    seq: 1,
    events: [operational, ...restricted]
  };
}

function expectedStack(index: number): unknown[] {
  const hop = scenario.hops[index]!;
  const stack: unknown[] = hop.localStack.map((site) => ({
    kind: 'local',
    site
  }));
  if (index > 0) {
    const remote = scenario.hops[index - 1]!;
    stack.push({
      kind: 'remoteBoundary',
      serviceId: remote.serviceId,
      operationId: remote.operationId,
      errorId: scenario.errorId
    });
  }
  return stack;
}

async function fetchJson(url: string): Promise<unknown> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`request failed with HTTP ${response.status}`);
  }
  return response.json();
}

function readEvents(payload: unknown): Record<string, unknown>[] {
  if (
    typeof payload === 'object' &&
    payload !== null &&
    'events' in payload &&
    Array.isArray((payload as { events?: unknown }).events)
  ) {
    return (payload as { events: Record<string, unknown>[] }).events;
  }
  return [];
}

async function readFixtures(): Promise<{
  scenario: ScenarioFixture;
  corpus: WireCorpus;
}> {
  const [scenarioText, corpusText] = await Promise.all([
    readFile(
      new URL(
        '../../testdata/package-service-contract-deployment/service-error-convergence.json',
        import.meta.url
      ),
      'utf8'
    ),
    readFile(
      new URL(
        '../../runtime/transport/testdata/service-error-response-v2.json',
        import.meta.url
      ),
      'utf8'
    )
  ]);
  const parsedScenario = JSON.parse(scenarioText) as ScenarioFixture;
  const parsedCorpus = JSON.parse(corpusText) as WireCorpus;
  expect(parsedCorpus.schemaVersion).toBe(1);
  expect(parsedScenario.hops.map((hop) => hop.name)).toEqual(['A', 'B', 'C']);
  return { scenario: parsedScenario, corpus: parsedCorpus };
}
