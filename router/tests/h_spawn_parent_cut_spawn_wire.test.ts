import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

import {
  decodeBinaryFrame,
  encodeBinaryFrame,
} from '../src/protocol/envelope.js';

const CORPUS_DIR = join(
  __dirname,
  '../../runtime/transport/testdata/spawn-wire'
);

const REQUIRED_FRAMES = [
  'spawn.submit.request.function',
  'spawn.submit.request.actorMethod',
  'spawn.submit.request.legacy-no-caller-kind',
  'spawn.submit.response',
  'spawn.submit.error.parentNotFound',
] as const;

const REQUIRED_SCENARIOS = [
  'resolve-function-parent-exact',
  'resolve-actor-invocation-parent-exact',
  'same-request-id-both-namespaces-no-collision',
  'missing-caller-kind-legacy-cut-rejected',
  'parent-terminal-before-submit-rejected',
  'parent-replaced-before-submit-rejected',
  'parent-connection-mismatch-rejected',
  'authority-mismatch-rejected',
  'accepted-spawn-outlives-parent-terminal',
  'target-kind-mismatch-rejected',
] as const;

interface FrameEntry {
  direction: string;
  frameType: string;
  decodeAs: string;
  payloadPresence: string;
  payloadBase64: string;
  frameHex: string;
  legacyCut: boolean;
  header: Record<string, unknown>;
}

interface FrameCatalog {
  schemaVersion: number;
  corpus: string;
  frames: Record<string, FrameEntry>;
}

interface ParentJson {
  id: string;
  runtimeId: string;
  connection: string;
  assemblyGeneration: number;
  testCaseCapability: string | null;
}

interface ScenarioParents {
  request: ParentJson[];
  actorInvocation: ParentJson[];
}

interface ScenarioEvent {
  op: string;
  legacy?: boolean;
  callerKind?: string;
  callerRequestId?: string;
  targetKind?: string;
  actorMethod?: boolean;
  connection?: string;
  assemblyGeneration?: number;
  testCaseCapability?: string;
  newConnection?: string;
  newRuntimeId?: string;
}

interface ScenarioExpect {
  accepted: string[];
  rejected: string[];
  errors: Record<string, string>;
  acceptedSpawns?: number;
}

interface SpawnScenario {
  schemaVersion: number;
  scenario: string;
  parents: ScenarioParents;
  events: ScenarioEvent[];
  expect: ScenarioExpect;
}

function hexBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < hex.length; index += 2) {
    bytes[index / 2] = Number.parseInt(hex.slice(index, index + 2), 16);
  }
  return bytes;
}

function readCatalog(): FrameCatalog {
  return JSON.parse(
    readFileSync(join(CORPUS_DIR, 'frames.json'), 'utf8')
  ) as FrameCatalog;
}

// ---------------------------------------------------------------------------
// Resolver / router reference model (C-model-spawn §4, C-spawn §2-§4)
// ---------------------------------------------------------------------------

interface ParentRecord {
  runtimeId: string;
  connection: string;
  assemblyGeneration: number;
  testCaseCapability: string | null;
  active: boolean;
  replaced: boolean;
}

class ParentStores {
  readonly request = new Map<string, ParentRecord>();
  readonly actorInvocation = new Map<string, ParentRecord>();

  get(callerKind: string, id: string): ParentRecord | undefined {
    if (callerKind === 'request') return this.request.get(id);
    if (callerKind === 'actorInvocation') return this.actorInvocation.get(id);
    return undefined;
  }

  getMut(callerKind: string, id: string): ParentRecord | undefined {
    if (callerKind === 'request') return this.request.get(id);
    if (callerKind === 'actorInvocation') return this.actorInvocation.get(id);
    return undefined;
  }
}

function submitEventKey(event: ScenarioEvent): string {
  if (event.legacy) {
    return `legacy:${event.callerRequestId ?? '?'}`;
  }
  return `${event.callerKind ?? '?'}:${event.callerRequestId ?? '?'}`;
}

function referenceSubmit(
  stores: ParentStores,
  event: ScenarioEvent
): string {
  if (event.legacy || event.callerKind === undefined) {
    throw new Error('CallerKindRejected');
  }
  if (
    event.callerKind !== 'request' &&
    event.callerKind !== 'actorInvocation'
  ) {
    throw new Error('CallerKindRejected');
  }
  const parent = stores.get(event.callerKind, event.callerRequestId ?? '');
  if (parent === undefined) {
    throw new Error('ParentNotFound');
  }
  if (!parent.active) {
    throw new Error('ParentTerminal');
  }
  if (parent.replaced) {
    throw new Error('ParentReplaced');
  }
  if (parent.connection !== (event.connection ?? '')) {
    throw new Error('ParentConnectionMismatch');
  }
  if (
    parent.testCaseCapability !== null &&
    event.testCaseCapability !== parent.testCaseCapability
  ) {
    throw new Error('TestCapabilityMismatch');
  }
  if (
    event.assemblyGeneration !== undefined &&
    event.assemblyGeneration !== parent.assemblyGeneration
  ) {
    throw new Error('AuthorityMismatch');
  }
  const targetKind = event.targetKind;
  if (targetKind === undefined) {
    throw new Error('TargetKindMismatch');
  }
  if (targetKind === 'actorMethod' && event.actorMethod !== true) {
    throw new Error('TargetKindMismatch');
  }
  if (targetKind === 'function' && event.actorMethod === true) {
    throw new Error('TargetKindMismatch');
  }
  return submitEventKey(event);
}

function replayScenario(raw: string): SpawnScenario {
  const scenario = JSON.parse(raw) as SpawnScenario;
  expect(scenario.schemaVersion).toBe(1);
  const stores = new ParentStores();
  for (const parent of scenario.parents.request) {
    stores.request.set(parent.id, {
      runtimeId: parent.runtimeId,
      connection: parent.connection,
      assemblyGeneration: parent.assemblyGeneration,
      testCaseCapability: parent.testCaseCapability,
      active: true,
      replaced: false,
    });
  }
  for (const parent of scenario.parents.actorInvocation) {
    stores.actorInvocation.set(parent.id, {
      runtimeId: parent.runtimeId,
      connection: parent.connection,
      assemblyGeneration: parent.assemblyGeneration,
      testCaseCapability: parent.testCaseCapability,
      active: true,
      replaced: false,
    });
  }
  const accepted: string[] = [];
  const rejected: string[] = [];
  const errors: Record<string, string> = {};
  let acceptedSpawns = 0;
  for (const event of scenario.events) {
    switch (event.op) {
      case 'submit': {
        const key = submitEventKey(event);
        try {
          referenceSubmit(stores, event);
          accepted.push(key);
          acceptedSpawns += 1;
        } catch (error) {
          rejected.push(key);
          errors[key] = error instanceof Error ? error.message : String(error);
        }
        break;
      }
      case 'parentTerminal': {
        const parent = stores.getMut(
          event.callerKind ?? '',
          event.callerRequestId ?? ''
        );
        expect(parent).toBeDefined();
        parent!.active = false;
        break;
      }
      case 'replace': {
        const parent = stores.getMut(
          event.callerKind ?? '',
          event.callerRequestId ?? ''
        );
        expect(parent).toBeDefined();
        parent!.replaced = true;
        if (event.newConnection !== undefined) {
          parent!.connection = event.newConnection;
        }
        if (event.newRuntimeId !== undefined) {
          parent!.runtimeId = event.newRuntimeId;
        }
        break;
      }
      default:
        throw new Error(`unknown spawn scenario op ${event.op}`);
    }
  }
  expect(accepted).toEqual(scenario.expect.accepted);
  expect(rejected).toEqual(scenario.expect.rejected);
  expect(errors).toEqual(scenario.expect.errors);
  expect(acceptedSpawns).toBe(scenario.expect.acceptedSpawns ?? 0);
  return scenario;
}

describe('H-spawn-parent-cut shared spawn-wire corpus (TS consumer)', () => {
  it('roundtrips every frozen frame byte-exact through the TS binary codec', () => {
    const catalog = readCatalog();
    expect(catalog.schemaVersion).toBe(1);
    expect(catalog.corpus).toBe('spawn-wire-v1');
    for (const required of REQUIRED_FRAMES) {
      expect(catalog.frames[required]).toBeDefined();
    }
    expect(Object.keys(catalog.frames).length).toBe(REQUIRED_FRAMES.length);
    for (const [name, entry] of Object.entries(catalog.frames)) {
      expect(entry.direction).toBe('RouterToRuntime');
      expect(entry.header.schemaVersion).toBe('skiff-runtime-frame-v3');
      expect(entry.header.type).toBe(entry.frameType);
      const decoded = decodeBinaryFrame(hexBytes(entry.frameHex));
      expect(decoded.header).toEqual(entry.header);
      if (entry.payloadPresence === 'required') {
        expect(decoded.payloadBytes.byteLength).toBeGreaterThan(0);
      } else {
        expect(decoded.payloadBytes.byteLength).toBe(0);
      }
      const reencoded = encodeBinaryFrame(
        decoded.header as Record<string, unknown>,
        decoded.payloadBytes
      );
      expect(reencoded.toString('hex')).toBe(entry.frameHex);
    }
  });

  it('freezes the target request shape with a required closed callerKind', () => {
    const catalog = readCatalog();
    const functionFrame = catalog.frames['spawn.submit.request.function'];
    const actorFrame = catalog.frames['spawn.submit.request.actorMethod'];
    expect(functionFrame.header.callerKind).toBe('request');
    expect(functionFrame.header.callerRequestId).toBe('parent-1');
    expect(actorFrame.header.callerKind).toBe('actorInvocation');
    expect(actorFrame.header.actorMethod).toBeDefined();
    expect(actorFrame.legacyCut).toBe(false);
    expect(functionFrame.legacyCut).toBe(false);

    const legacy = catalog.frames['spawn.submit.request.legacy-no-caller-kind'];
    expect(legacy.legacyCut).toBe(true);
    expect('callerKind' in legacy.header).toBe(false);
    expect(legacy.header.callerRequestId).toBe('parent-1');
    const decodedLegacy = decodeBinaryFrame(hexBytes(legacy.frameHex));
    expect('callerKind' in decodedLegacy.header).toBe(false);
  });

  it('replays every frozen parent-resolution scenario through the reference model', () => {
    const scenarioNames = readdirSync(join(CORPUS_DIR, 'scenarios'))
      .filter((name) => name.endsWith('.json'))
      .sort();
    expect(scenarioNames.length).toBe(REQUIRED_SCENARIOS.length);
    const replayed: string[] = [];
    for (const name of scenarioNames) {
      const scenario = replayScenario(
        readFileSync(join(CORPUS_DIR, 'scenarios', name), 'utf8')
      );
      replayed.push(scenario.scenario);
    }
    for (const required of REQUIRED_SCENARIOS) {
      expect(replayed).toContain(required);
    }
  });
});
