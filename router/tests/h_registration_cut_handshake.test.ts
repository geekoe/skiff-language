// H-registration-cut shared-corpus consumer gate (TS Router side).
//
// Consumes the frozen C-model-registration corpus
// (`runtime/transport/testdata/registration-handshake/`) through the TS
// protocol codecs and the production per-connection handshake state machine
// (`RuntimeHandshakeState`), then drives the real `RuntimeEndpoint` with the
// corpus bytes for the wire-level handshake, strict terminals and health
// gating.

import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';

import { decodeAssemblyActivationFrame } from '../src/protocol/assemblyActivationFrame.js';
import {
  decodeBinaryFrame,
  encodeBinaryFrame
} from '../src/protocol/envelope.js';
import { AssemblyRuntimeRegistry } from '../src/router/assemblyRuntimeRegistry.js';
import { RuntimeEndpoint } from '../src/router/runtimeEndpoint.js';
import {
  RuntimeHandshakeState,
  type RuntimeHandshakeRegisterControl,
  type RuntimeHandshakeTerminalKind,
  type RuntimeRegisteredAssemblyTuple
} from '../src/router/runtimeHandshake.js';
import { RuntimeRegistry } from '../src/router/runtimeRegistry.js';
import {
  RouterActiveAssemblySnapshotStore,
  RuntimeAssemblyIngressIndex
} from '../src/router/runtimeAssemblySnapshot.js';

const TEST_ROOT = join(
  dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
  'runtime',
  'transport',
  'testdata',
  'registration-handshake'
);

const REQUIRED_FRAMES: readonly string[] = [
  'bootstrap.prod.42',
  'capabilities.runtime-a',
  'capabilities.runtime-b',
  'register.prod.42.a',
  'register.prod.42.b',
  'register.prod.41.a',
  'register.prod.42.other-assembly',
  'register.prod.43.a',
  'registered.runtime-a',
  'registered.runtime-b',
  'health.empty',
  'legacy.runtime.register'
];

const REQUIRED_SCENARIOS: readonly string[] = [
  'accept-sequence',
  'wrong-order-health-before-capabilities',
  'wrong-order-register-before-capabilities',
  'legacy-register-rejected',
  'identity-change-register-replica',
  'identity-change-capabilities-replica',
  'duplicate-register-pre-ack',
  'stale-register-old-generation',
  'tuple-mismatch-assembly',
  'new-generation-before-epoch-swap',
  'ack-loss',
  'health-before-ack-no-observation',
  'pre-auth-limit',
  'bootstrap-timeout',
  'capabilities-timeout',
  'register-timeout',
  'disconnect-mid-handshake',
  're-register-exact-idempotent',
  're-register-stale-after-ack'
];

const ASSEMBLY_IDENTITY =
  'skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const CONFIG_SNAPSHOT_ID =
  'skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const PROD_EPOCH: RuntimeRegisteredAssemblyTuple = {
  environment: 'prod',
  generation: 42,
  assembly: { assemblyIdentity: ASSEMBLY_IDENTITY },
  configSnapshot: { snapshotId: CONFIG_SNAPSHOT_ID }
};

interface FrameEntry {
  direction: string;
  frameType: string;
  decodeAs: string;
  frameHex: string;
  header: Record<string, unknown>;
}

interface Catalog {
  schemaVersion: number;
  corpus: string;
  frames: Record<string, FrameEntry>;
}

interface Scenario {
  schemaVersion: number;
  scenario: string;
  epoch: {
    environment: string;
    generation: number;
    assembly: { assemblyIdentity: string };
    configSnapshot: { snapshotId: string };
    pending?: {
      environment: string;
      generation: number;
      assembly: { assemblyIdentity: string };
      configSnapshot: { snapshotId: string };
    } | null;
  };
  preAuthLimit: number;
  events: Array<{
    kind: string;
    connection?: string;
    connectionGeneration?: number;
    frame?: string;
    timeoutKind?: string;
  }>;
  expect: {
    outcomes: Record<string, string>;
    refusedCount: number;
    preAuthCount: number;
    registeredSessions: string[];
    observedHealth: number;
    healthBeforeAck: number;
    routableRegistered: boolean;
    publishedPending: boolean;
    revision: number;
    failStop: boolean;
  };
}

function loadCatalog(): Catalog {
  const value = JSON.parse(
    readFileSync(join(TEST_ROOT, 'frames.json'), 'utf8')
  ) as Catalog;
  expect(value.schemaVersion).toBe(1);
  expect(value.corpus).toBe('registration-handshake-v1');
  return value;
}

function loadScenarios(): Scenario[] {
  const scenarios: Scenario[] = [];
  for (const name of readdirSync(join(TEST_ROOT, 'scenarios'))) {
    if (!name.endsWith('.json')) {
      continue;
    }
    scenarios.push(
      JSON.parse(
        readFileSync(join(TEST_ROOT, 'scenarios', name), 'utf8')
      ) as Scenario
    );
  }
  return scenarios.sort((left, right) =>
    left.scenario.localeCompare(right.scenario)
  );
}

function hexToBytes(hex: string): Buffer {
  expect(hex.length % 2).toBe(0);
  return Buffer.from(hex, 'hex');
}

function bytesToHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('hex');
}

/**
 * Corpus-event harness around the production `RuntimeHandshakeState`. The
 * machine owns phase/terminal decisions; this harness owns the per-connection
 * counters that the frozen reference model also tracks (pre-auth occupancy,
 * revision, observed health, registered replicas).
 */
class CorpusHandshakeHarness {
  private readonly preAuthLimit: number;
  private readonly conns = new Map<
    string,
    { state: RuntimeHandshakeState; revision: number }
  >();
  private readonly preAuthConns = new Set<string>();
  private readonly refusedConns: string[] = [];
  private refusedCount = 0;
  private observedHealth = 0;
  private readonly registeredReplicas: string[] = [];
  private readonly epoch: RuntimeRegisteredAssemblyTuple;
  private readonly pending: RuntimeRegisteredAssemblyTuple | undefined;

  constructor(
    epoch: RuntimeRegisteredAssemblyTuple,
    pending: RuntimeRegisteredAssemblyTuple | undefined,
    preAuthLimit: number
  ) {
    this.epoch = epoch;
    this.pending = pending;
    this.preAuthLimit = preAuthLimit;
  }

  acceptConnection(connection: string): boolean {
    if (this.refusedConns.includes(connection)) {
      // A later accept on the same connection supersedes the earlier refusal
      // (corpus pre-auth-limit scenario: c3 is refused once, then accepted).
      this.refusedConns.splice(this.refusedConns.indexOf(connection), 1);
    }
    if (this.preAuthConns.size >= this.preAuthLimit) {
      this.refusedConns.push(connection);
      this.refusedCount += 1;
      return false;
    }
    this.preAuthConns.add(connection);
    this.machine(connection);
    return true;
  }

  private machine(
    connection: string
  ): { state: RuntimeHandshakeState; revision: number } {
    let entry = this.conns.get(connection);
    if (entry === undefined) {
      entry = { state: new RuntimeHandshakeState(), revision: 0 };
      this.conns.set(connection, entry);
    }
    return entry;
  }

  private releasePreAuth(connection: string): void {
    this.preAuthConns.delete(connection);
  }

  private terminal(connection: string): void {
    const entry = this.machine(connection);
    entry.revision = 0;
    const replica = entry.state.replica();
    if (replica !== undefined) {
      const index = this.registeredReplicas.indexOf(replica);
      if (index !== -1) {
        this.registeredReplicas.splice(index, 1);
      }
    }
    this.releasePreAuth(connection);
  }

  write(connection: string, frameName: string): void {
    const entry = this.machine(connection);
    const state = entry.state;
    if (frameName.startsWith('bootstrap.')) {
      const terminal = state.onBootstrapWritten();
      if (terminal !== undefined) {
        this.terminal(connection);
      }
      return;
    }
    if (frameName.startsWith('registered.')) {
      const replica = state.replica();
      const terminal = state.onAckWritten();
      if (terminal === undefined) {
        this.releasePreAuth(connection);
        if (replica !== undefined) {
          this.registeredReplicas.push(replica);
        }
      } else {
        this.terminal(connection);
      }
      return;
    }
    throw new Error(`unexpected outbound frame ${frameName}`);
  }

  writeFail(connection: string, frameName: string): void {
    const entry = this.machine(connection);
    const terminal: RuntimeHandshakeTerminalKind =
      frameName.startsWith('bootstrap.')
        ? entry.state.onBootstrapWriteFailed()
        : frameName.startsWith('registered.')
          ? entry.state.onAckWriteFailed()
          : entry.state.terminalWith('AckLoss');
    void terminal;
    this.terminal(connection);
  }

  read(connection: string, frameName: string, bytes: Buffer): void {
    const entry = this.machine(connection);
    const state = entry.state;
    if (frameName === 'legacy.runtime.register') {
      this.terminalAfter(connection, state.onLegacyRegister());
      return;
    }
    if (frameName.startsWith('capabilities.')) {
      const frame = decodeBinaryFrame(bytes);
      const event = state.onCapabilities(
        frame.header.runtimeId as string
      );
      if (event.kind === 'terminal') {
        this.terminal(connection);
      }
      return;
    }
    if (frameName.startsWith('register.')) {
      const control = decodeAssemblyActivationFrame('runtimeToRouter', bytes);
      if (control.type !== 'register') {
        throw new Error(`corpus register frame decoded as ${control.type}`);
      }
      const register: RuntimeHandshakeRegisterControl = {
        environment: control.environment,
        generation: control.generation,
        assembly: { ...control.assembly },
        configSnapshot: { ...control.configSnapshot },
        replicaId: control.replicaId
      };
      const event = state.onRegister(register, {
        current: this.epoch,
        ...(this.pending === undefined ? {} : { pending: this.pending })
      });
      if (event.kind === 'validated') {
        entry.revision += 1;
      } else if (event.kind === 'terminal') {
        this.terminal(connection);
      }
      return;
    }
    if (frameName.startsWith('health.')) {
      const frame = decodeBinaryFrame(bytes);
      const event = state.onHealth(frame.header.runtimeId as string);
      if (event.kind === 'observed') {
        this.observedHealth += 1;
      } else if (event.kind === 'terminal') {
        this.terminal(connection);
      }
      return;
    }
    throw new Error(`unexpected inbound frame ${frameName}`);
  }

  timeout(connection: string, kind: string): void {
    const entry = this.machine(connection);
    this.terminalAfter(
      connection,
      entry.state.onTimeout(
        kind as 'bootstrap' | 'capabilities' | 'register'
      )
    );
  }

  disconnect(connection: string): void {
    const entry = this.machine(connection);
    this.terminalAfter(connection, entry.state.onDisconnect());
  }

  private terminalAfter(
    connection: string,
    terminal: RuntimeHandshakeTerminalKind
  ): void {
    void terminal;
    this.terminal(connection);
  }

  outcome(connection: string): string {
    if (this.refusedConns.includes(connection)) {
      return 'PreAuthLimitRejected';
    }
    const entry = this.conns.get(connection);
    if (entry === undefined) {
      return 'Accepted';
    }
    return entry.state.outcomeName();
  }

  snapshot(connection: string) {
    const entry = this.machine(connection);
    return {
      outcome: this.outcome(connection),
      refusedCount: this.refusedCount,
      preAuthCount: this.preAuthConns.size,
      registeredReplicas: [...this.registeredReplicas],
      observedHealth: this.observedHealth,
      healthBeforeAck: entry.state.healthBeforeAck(),
      revision: entry.revision,
      publishedPending: entry.state.phase() === 'register-validated'
    };
  }
}

function epochTuple(scenario: Scenario): RuntimeRegisteredAssemblyTuple {
  return {
    environment: scenario.epoch.environment,
    generation: scenario.epoch.generation,
    assembly: { ...scenario.epoch.assembly },
    configSnapshot: { ...scenario.epoch.configSnapshot }
  };
}

function pendingTuple(scenario: Scenario):
  | RuntimeRegisteredAssemblyTuple
  | undefined {
  if (scenario.epoch.pending === undefined || scenario.epoch.pending === null) {
    return undefined;
  }
  return {
    environment: scenario.epoch.pending.environment,
    generation: scenario.epoch.pending.generation,
    assembly: { ...scenario.epoch.pending.assembly },
    configSnapshot: { ...scenario.epoch.pending.configSnapshot }
  };
}

describe('H-registration-cut shared corpus (TS Router consumer)', () => {
  const catalog = loadCatalog();
  const scenarios = loadScenarios();
  const frameBytes = new Map<string, Buffer>();
  for (const name of REQUIRED_FRAMES) {
    expect(catalog.frames[name], `required frame ${name}`).toBeDefined();
    frameBytes.set(name, hexToBytes(catalog.frames[name]!.frameHex));
  }

  it('roundtrips every frozen handshake frame byte-exact through TS codecs', () => {
    for (const name of REQUIRED_FRAMES) {
      const entry = catalog.frames[name]!;
      const bytes = frameBytes.get(name)!;
      const decoded = decodeBinaryFrame(bytes);
      const reencoded = encodeBinaryFrame(decoded.header, decoded.payloadBytes);
      expect(bytesToHex(reencoded), `${name} must roundtrip`).toBe(
        bytesToHex(bytes)
      );
      expect(entry.frameType, `${name} frameType`).toBeDefined();
    }
  });

  it('replays all frozen scenarios through the production handshake machine', () => {
    expect(scenarios.map((scenario) => scenario.scenario)).toEqual(
      [...REQUIRED_SCENARIOS].sort((left, right) =>
        left.localeCompare(right)
      )
    );
    for (const scenario of scenarios) {
      const harness = new CorpusHandshakeHarness(
        epochTuple(scenario),
        pendingTuple(scenario),
        scenario.preAuthLimit
      );
      for (const event of scenario.events) {
        const connection = event.connection ?? 'c1';
        switch (event.kind) {
          case 'accept':
            harness.acceptConnection(connection);
            break;
          case 'write': {
            const frame = event.frame!;
            harness.write(connection, frame);
            break;
          }
          case 'writeFail':
            harness.writeFail(connection, event.frame!);
            break;
          case 'read': {
            const frame = event.frame!;
            harness.read(connection, frame, frameBytes.get(frame)!);
            break;
          }
          case 'timeout':
            harness.timeout(connection, event.timeoutKind!);
            break;
          case 'disconnect':
            harness.disconnect(connection);
            break;
          default:
            throw new Error(
              `scenario ${scenario.scenario} has unknown event ${event.kind}`
            );
        }
        void connection;
      }
      for (const [connection, expectedOutcome] of Object.entries(
        scenario.expect.outcomes
      )) {
        expect(
          harness.outcome(connection),
          `${scenario.scenario} outcome for ${connection}`
        ).toBe(expectedOutcome);
      }
      const snapshot = harness.snapshot('c1');
      expect(
        snapshot.refusedCount,
        `${scenario.scenario} refusedCount`
      ).toBe(scenario.expect.refusedCount);
      expect(
        snapshot.preAuthCount,
        `${scenario.scenario} preAuthCount`
      ).toBe(scenario.expect.preAuthCount);
      expect(
        snapshot.registeredReplicas,
        `${scenario.scenario} registeredSessions`
      ).toEqual(scenario.expect.registeredSessions);
      expect(
        snapshot.observedHealth,
        `${scenario.scenario} observedHealth`
      ).toBe(scenario.expect.observedHealth);
      expect(
        snapshot.healthBeforeAck,
        `${scenario.scenario} healthBeforeAck`
      ).toBe(scenario.expect.healthBeforeAck);
      expect(
        snapshot.registeredReplicas.length > 0,
        `${scenario.scenario} routableRegistered`
      ).toBe(scenario.expect.routableRegistered);
      expect(
        snapshot.publishedPending,
        `${scenario.scenario} publishedPending`
      ).toBe(scenario.expect.publishedPending);
      expect(snapshot.revision, `${scenario.scenario} revision`).toBe(
        scenario.expect.revision
      );
      expect(scenario.expect.failStop).toBe(false);
    }
  });
});

describe('H-registration-cut real RuntimeEndpoint handshake', () => {
  const endpoints: RuntimeEndpoint[] = [];
  const sockets: WebSocket[] = [];

  afterEach(async () => {
    for (const socket of sockets.splice(0)) {
      socket.close();
    }
    await Promise.all(endpoints.splice(0).map((endpoint) => endpoint.close()));
  });

  async function createFixture(preAuthMaxConcurrency?: number) {
    const snapshots = new RouterActiveAssemblySnapshotStore();
    snapshots.replace({
      environment: 'prod',
      generation: 42,
      assembly: { assemblyIdentity: ASSEMBLY_IDENTITY },
      configSnapshot: { snapshotId: CONFIG_SNAPSHOT_ID },
      ingress: new RuntimeAssemblyIngressIndex([])
    });
    const assemblyRegistry = new AssemblyRuntimeRegistry(snapshots);
    const registry = new RuntimeRegistry();
    const endpoint = new RuntimeEndpoint({
      registry,
      assemblyRegistry,
      bootstrap: {
        artifactsPath: '/tmp/skiff-h-registration-cut',
        serviceDb: { mongoUrl: 'mongodb://127.0.0.1:27017/skiff-test' },
        http: { maxResponseBytes: 67108864 },
        activation: {
          environment: 'prod',
          generation: 42,
          assembly: { assemblyIdentity: ASSEMBLY_IDENTITY },
          configSnapshot: { snapshotId: CONFIG_SNAPSHOT_ID }
        }
      },
      ...(preAuthMaxConcurrency === undefined
        ? {}
        : { preAuthMaxConcurrency }),
      handshakeTimeouts: {
        bootstrapMs: 2_000,
        capabilitiesMs: 2_000,
        registerMs: 2_000,
        ackWriteMs: 1_000
      }
    });
    endpoints.push(endpoint);
    const listening = await endpoint.listen({ port: 0 });
    return { assemblyRegistry, endpoint, registry, url: listening.url };
  }

  async function openSocket(url: string): Promise<WebSocket> {
    const ws = new WebSocket(url);
    sockets.push(ws);
    await new Promise<void>((resolve, reject) => {
      ws.once('open', resolve);
      ws.once('error', reject);
    });
    return ws;
  }

  function nextNonBootstrapMessage(ws: WebSocket): Promise<Buffer> {
    return new Promise<Buffer>((resolve, reject) => {
      const timeout = setTimeout(() => {
        cleanup();
        reject(new Error('timed out waiting for binary frame'));
      }, 2_000);
      const onMessage = (data: WebSocket.RawData, isBinary: boolean) => {
        if (!isBinary) {
          cleanup();
          reject(new Error('expected binary runtime frame'));
          return;
        }
        const buffer = Buffer.isBuffer(data)
          ? data
          : Buffer.from(data as ArrayBuffer);
        try {
          if (decodeBinaryFrame(buffer).header.type === 'router.bootstrap') {
            return;
          }
        } catch {
          // Not a decodable binary frame; pass through.
        }
        cleanup();
        resolve(buffer);
      };
      const cleanup = () => {
        clearTimeout(timeout);
        ws.off('message', onMessage);
      };
      ws.on('message', onMessage);
    });
  }

  function nextClose(ws: WebSocket): Promise<[number, Buffer]> {
    return new Promise<[number, Buffer]>((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('timed out waiting for socket close'));
      }, 2_000);
      ws.once('close', (code, reason) => {
        clearTimeout(timeout);
        resolve([code, Buffer.from(reason)]);
      });
    });
  }

  async function until(predicate: () => boolean): Promise<void> {
    const deadline = Date.now() + 2_000;
    while (!predicate()) {
      if (Date.now() >= deadline) {
        throw new Error('timed out waiting for predicate');
      }
      await new Promise<void>((resolve) => setImmediate(resolve));
    }
  }

  it('completes the corpus accept sequence with a byte-exact ACK and health observation', async () => {
    const catalogFixture = loadCatalog();
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    ws.send(hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex));
    ws.send(hexToBytes(catalogFixture.frames['register.prod.42.a']!.frameHex));

    const ack = await nextNonBootstrapMessage(ws);
    expect(bytesToHex(ack)).toBe(
      catalogFixture.frames['registered.runtime-a']!.frameHex
    );
    await until(() =>
      fixture.assemblyRegistry.healthyParticipantReplicaIds().includes('runtime-a')
    );

    ws.send(hexToBytes(catalogFixture.frames['health.empty']!.frameHex));
    await until(() =>
      fixture.assemblyRegistry.snapshot().some(
        (replica) => replica.replicaId === 'runtime-a' && replica.lastHealthAt !== undefined
      )
    );
    expect(ws.readyState).toBe(WebSocket.OPEN);
  });

  it.each([
    ['legacy-register-rejected', 'legacy.runtime.register'],
    ['wrong-order-health-before-capabilities', 'health.empty'],
    ['wrong-order-register-before-capabilities', 'register.prod.42.a']
  ])('terminates %s on the real endpoint', async (_name, frameName) => {
    const catalogFixture = loadCatalog();
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    ws.send(hexToBytes(catalogFixture.frames[frameName]!.frameHex));
    const [code] = await nextClose(ws);
    expect(code).toBe(1008);
    expect(fixture.assemblyRegistry.snapshot()).toEqual([]);
  });

  it('terminates identity changes on the real endpoint', async () => {
    const catalogFixture = loadCatalog();
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    ws.send(hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex));
    ws.send(hexToBytes(catalogFixture.frames['register.prod.42.b']!.frameHex));
    const [code] = await nextClose(ws);
    expect(code).toBe(1008);
    expect(fixture.assemblyRegistry.snapshot()).toEqual([]);

    const secondFixture = await createFixture();
    const second = await openSocket(secondFixture.url);
    second.send(
      hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex)
    );
    second.send(
      hexToBytes(catalogFixture.frames['capabilities.runtime-b']!.frameHex)
    );
    const [secondCode] = await nextClose(second);
    expect(secondCode).toBe(1008);
  });

  it('terminates stale and tuple-mismatched registers on the real endpoint', async () => {
    const catalogFixture = loadCatalog();
    const staleFixture = await createFixture();
    const stale = await openSocket(staleFixture.url);
    stale.send(
      hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex)
    );
    stale.send(hexToBytes(catalogFixture.frames['register.prod.41.a']!.frameHex));
    const [staleCode] = await nextClose(stale);
    expect(staleCode).toBe(1008);
    expect(staleFixture.assemblyRegistry.snapshot()).toEqual([]);

    const tupleFixture = await createFixture();
    const tupleSocket = await openSocket(tupleFixture.url);
    tupleSocket.send(
      hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex)
    );
    tupleSocket.send(
      hexToBytes(
        catalogFixture.frames['register.prod.42.other-assembly']!.frameHex
      )
    );
    const [tupleCode] = await nextClose(tupleSocket);
    expect(tupleCode).toBe(1008);
    expect(tupleFixture.assemblyRegistry.snapshot()).toEqual([]);
  });

  it('keeps exact re-register idempotent after the ACK and closes on stale re-register', async () => {
    const catalogFixture = loadCatalog();
    const fixture = await createFixture();
    const ws = await openSocket(fixture.url);
    ws.send(hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex));
    ws.send(hexToBytes(catalogFixture.frames['register.prod.42.a']!.frameHex));
    const ack = await nextNonBootstrapMessage(ws);
    expect(bytesToHex(ack)).toBe(
      catalogFixture.frames['registered.runtime-a']!.frameHex
    );
    await until(() =>
      fixture.assemblyRegistry.healthyParticipantReplicaIds().includes('runtime-a')
    );

    // Exact duplicate after ACK is idempotent: no second ACK, session stays.
    ws.send(hexToBytes(catalogFixture.frames['register.prod.42.a']!.frameHex));
    await new Promise<void>((resolve) => setTimeout(resolve, 50));
    expect(ws.readyState).toBe(WebSocket.OPEN);

    // Stale re-register after ACK is a strict terminal.
    ws.send(hexToBytes(catalogFixture.frames['register.prod.41.a']!.frameHex));
    const [code] = await nextClose(ws);
    expect(code).toBe(1008);
    expect(
      fixture.assemblyRegistry.healthyParticipantReplicaIds()
    ).toEqual([]);
  });

  it('refuses pre-auth connections at the cap and accepts after registration', async () => {
    const catalogFixture = loadCatalog();
    const fixture = await createFixture(2);
    const first = await openSocket(fixture.url);
    const second = await openSocket(fixture.url);
    const third = await openSocket(fixture.url);
    const [thirdCode] = await nextClose(third);
    expect(thirdCode).toBe(1008);

    first.send(
      hexToBytes(catalogFixture.frames['capabilities.runtime-a']!.frameHex)
    );
    first.send(
      hexToBytes(catalogFixture.frames['register.prod.42.a']!.frameHex)
    );
    await nextNonBootstrapMessage(first);
    await until(() =>
      fixture.assemblyRegistry.healthyParticipantReplicaIds().includes('runtime-a')
    );

    const retry = await openSocket(fixture.url);
    await new Promise<void>((resolve) => setTimeout(resolve, 50));
    expect(retry.readyState).toBe(WebSocket.OPEN);
    expect(second.readyState).toBe(WebSocket.OPEN);
  });
});
