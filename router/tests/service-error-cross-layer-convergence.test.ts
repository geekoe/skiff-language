import { readFile } from 'node:fs/promises';
import { request as httpRequest } from 'node:http';

import WebSocket from 'ws';
import { afterEach, describe, expect, it } from 'vitest';

import type { ResponseErrorFrameHeader } from '../src/protocol/envelope.js';
import {
  runtimeFrameHeaderFixtures,
  validateResponseErrorFrame
} from '../src/protocol/runtimeProtocol.js';
import { AssemblyHttpGateway } from '../src/router/assemblyHttpGateway.js';
import {
  FixedServiceResponseError,
  RuntimeResponseError
} from '../src/router/errors.js';
import {
  RuntimeDispatcher,
  type RuntimeDispatchRegistry,
  type RuntimeFrameSender
} from '../src/router/runtimeDispatcher.js';
import type { RuntimeUnaryDispatchFrameHeader } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex,
  type RuntimeAssemblyIngressBinding
} from '../src/router/runtimeAssemblySnapshot.js';

const ASSEMBLY = `skiff-runtime-assembly-v3:sha256:${'a'.repeat(64)}`;
const DEPLOYMENT = `skiff-deployment-artifact-v4:sha256:${'c'.repeat(64)}`;
const CURRENT_HTTP_GATEWAY_ENTRY_IDENTITY =
  `skiff-gateway-entry-v2:sha256:${'e'.repeat(64)}`;
const HTTP_HOST = 'p5-f345-http.example.test';
const HTTP_PATH = '/convergence';
const CONTROL_CASE = 'generic-control-same-safe-values-as-internal';

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
  validCases: WireCase[];
}

interface WireCase {
  name: string;
  header: ResponseErrorFrameHeader;
  payloadUtf8: string;
  expected: Record<string, unknown>;
}

interface DispatcherHarness {
  dispatcher: RuntimeDispatcher;
  socket: WebSocket;
}

const activeClosers: Array<() => Promise<void>> = [];
const { scenario, corpus } = await readFixtures();

afterEach(async () => {
  while (activeClosers.length > 0) {
    await activeClosers.pop()!();
  }
});

describe('P5-F345 service error Router convergence', () => {
  it('keeps the C0 object and bytes through actual unaryFrame dispatch and uses only fixed mapping', async () => {
    const fixedCase = wireCase(scenario.corpusCase);
    const payloadBytes = Buffer.from(fixedCase.payloadUtf8, 'utf8');
    const fixed = validateResponseErrorFrame(fixedCase.header, payloadBytes);
    expect(fixed).toMatchObject({ ok: true });
    if (!fixed.ok || !('serviceError' in fixed.envelope)) {
      throw new Error(fixed.ok ? 'fixed view is missing' : fixed.error);
    }
    expect(fixed.envelope.payloadBytes).toBe(payloadBytes);
    expect(fixed.envelope.header).toBe(fixedCase.header);
    expect(fixed.envelope.serviceError).toEqual({
      kind: 'internalError',
      payload: {
        message: 'The service could not complete the request.',
        traceId: scenario.traceId,
        errorId: scenario.errorId
      }
    });

    const harness = dispatcherHarness();
    const request = requestHeader(String(fixedCase.header.requestId));
    const forwardedPromise = harness.dispatcher.dispatchBinaryFrame(
      { header: request, payloadBytes: new Uint8Array([3, 4, 5]) },
      1_000
    );
    harness.dispatcher.rejectRequest(harness.socket, fixed.envelope);
    const forwarded = await forwardedPromise;
    expect(forwarded.header).toBe(fixedCase.header);
    expect(forwarded.payloadBytes).toBe(payloadBytes);
    expect(Buffer.from(forwarded.payloadBytes)).toEqual(payloadBytes);

    const mappedPromise = harness.dispatcher
      .dispatchBinary(
        { header: request, payloadBytes: new Uint8Array() },
        1_000
      )
      .catch((error: unknown) => error);
    harness.dispatcher.rejectRequest(harness.socket, fixed.envelope);
    const mapped = await mappedPromise;
    expect(mapped).toBeInstanceOf(FixedServiceResponseError);
    expect(mapped).not.toBeInstanceOf(RuntimeResponseError);
    expect(mapped).toMatchObject({
      traceId: scenario.traceId,
      errorId: scenario.errorId,
      serviceErrorKind: 'internalError'
    });

    const controlCase = wireCase(CONTROL_CASE);
    const controlBytes = Buffer.from(controlCase.payloadUtf8, 'utf8');
    const control = validateResponseErrorFrame(controlCase.header, controlBytes);
    expect(control).toMatchObject({ ok: true });
    if (!control.ok || 'serviceError' in control.envelope) {
      throw new Error(control.ok ? 'control was upgraded to fixed' : control.error);
    }
    const controlRequest = requestHeader(String(controlCase.header.requestId));
    const controlPromise = harness.dispatcher
      .dispatchBinary(
        { header: controlRequest, payloadBytes: new Uint8Array() },
        1_000
      )
      .catch((error: unknown) => error);
    harness.dispatcher.rejectRequest(harness.socket, control.envelope);
    const mappedControl = await controlPromise;
    expect(mappedControl).toBeInstanceOf(RuntimeResponseError);
    expect(mappedControl).not.toBeInstanceOf(FixedServiceResponseError);
  });

  it('projects the same strict fixed fact through the actual HTTP gateway', async () => {
    const fixedCase = wireCase(scenario.corpusCase);
    const payloadBytes = Buffer.from(fixedCase.payloadUtf8, 'utf8');
    const fixed = validateResponseErrorFrame(fixedCase.header, payloadBytes);
    if (!fixed.ok || !('serviceError' in fixed.envelope)) {
      throw new Error(fixed.ok ? 'fixed view is missing' : fixed.error);
    }
    const fixedError = new FixedServiceResponseError(fixed.envelope.serviceError);
    const snapshots = snapshotsForScenario();
    const rejectingDispatcher = {
      dispatchBinary: async () => {
        throw fixedError;
      }
    } as unknown as RuntimeDispatcher;

    const httpGateway = new AssemblyHttpGateway({
      snapshots,
      dispatcher: rejectingDispatcher,
      port: 0,
      maxRequestBytes: 1024,
      maxResponseBytes: 1024,
      requestTimeoutMs: 1_000
    });
    const httpListen = await httpGateway.listen();
    activeClosers.push(() => httpGateway.close());
    const http = await sendHttp(httpListen.url);
    expect(http.status).toBe(500);
    expect(JSON.parse(http.body)).toEqual({
      error: {
        code: 'FixedServiceError',
        message: scenario.externalSafeMessage,
        details: {
          traceId: scenario.traceId,
          errorId: scenario.errorId
        }
      }
    });
    assertExternalSafe(http.body);
  });
});

function dispatcherHarness(): DispatcherHarness {
  const socket = {
    readyState: WebSocket.OPEN
  } as WebSocket;
  const registry: RuntimeDispatchRegistry = {
    setInFlightCounter: () => undefined,
    pickDispatchConnection: () => ({
      runtimeId: 'runtime-p5-f345',
      ws: socket
    }),
    refreshAllRuntimeStates: () => undefined,
    refreshRuntimeStatesForRequest: () => undefined
  };
  const frameSender: RuntimeFrameSender = {
    sendFrame: () => undefined
  };
  return {
    dispatcher: new RuntimeDispatcher({ registry, frameSender }),
    socket
  };
}

function requestHeader(requestId: string): RuntimeUnaryDispatchFrameHeader {
  return {
    ...runtimeFrameHeaderFixtures['request.start'],
    requestId
  };
}

function snapshotsForScenario(): RouterActiveAssemblySnapshotStore {
  const outer = scenario.hops[2]!;
  const bindings: RuntimeAssemblyIngressBinding[] = [binding(outer)];
  const snapshots = new RouterActiveAssemblySnapshotStore();
  snapshots.replace({
    environment: 'test',
    generation: 345,
    assembly: { assemblyIdentity: ASSEMBLY },
    ingress: new RuntimeAssemblyIngressIndex(bindings)
  });
  return snapshots;
}

function binding(
  hop: HopExpectation
): RuntimeAssemblyIngressBinding {
  return {
    selector: {
      protocol: 'http',
      method: 'POST',
      path: HTTP_PATH
    },
    deployment: {
      serviceId: hop.serviceId,
      contractVersion: '1.0.0',
      deploymentRevision: hop.activationId,
      deploymentArtifactIdentity: DEPLOYMENT
    },
    gatewayEntryKey: 'convergence',
    gatewayEntryIdentity: CURRENT_HTTP_GATEWAY_ENTRY_IDENTITY,
    adapterKind: 'typedJson',
    operationMode: 'unary'
  };
}

async function sendHttp(baseUrl: string): Promise<{ status: number; body: string }> {
  const base = new URL(baseUrl);
  return await new Promise((resolve, reject) => {
    const request = httpRequest(
      {
        hostname: base.hostname,
        port: base.port,
        path: HTTP_PATH,
        method: 'POST',
        headers: {
          host: HTTP_HOST,
          'content-length': '0'
        }
      },
      (response) => {
        const chunks: Buffer[] = [];
        response.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
        response.on('end', () => {
          resolve({
            status: response.statusCode ?? 0,
            body: Buffer.concat(chunks).toString('utf8')
          });
        });
      }
    );
    request.once('error', reject);
    request.end();
  });
}

function assertExternalSafe(output: string): void {
  for (const forbidden of [
    scenario.privateSentinel,
    'sourceId',
    'sourceFrame',
    'sourceFrames',
    'frames',
    'stack',
    'function',
    'path',
    'encodedPayload'
  ]) {
    expect(output).not.toContain(forbidden);
  }
}

function wireCase(name: string): WireCase {
  const selected = corpus.validCases.find((candidate) => candidate.name === name);
  if (selected === undefined) {
    throw new Error(`missing C0 corpus case ${name}`);
  }
  return selected;
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
