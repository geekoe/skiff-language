import { describe, expect, it } from 'vitest';

import {
  decodeAssemblyActivationControl,
  decodeAssemblyActivationRequest,
  decodeEnvironmentActivationState
} from '../src/protocol/assemblyActivationProtocol.js';

const assembly = {
  assemblyIdentity:
    'skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
};
const configSnapshot = {
  snapshotId: 'skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
};

function transition(type: string) {
  return {
    type,
    environment: 'test',
    activationId: 'activation-1',
    expectedGeneration: 0,
    candidateGeneration: 1,
    assembly,
    configSnapshot,
    replicaId: 'runtime-a'
  };
}

describe('assembly activation serviceDb wire', () => {
  it('requires one exact opaque config snapshot ref everywhere', () => {
    const request = {
      schemaVersion: 'skiff-assembly-activation-request-v2',
      environment: 'test',
      activationId: 'activation-1',
      expectedGeneration: 0,
      assembly,
      configSnapshot
    };
    expect(decodeAssemblyActivationRequest(request)).toEqual(request);
    expect(() =>
      decodeAssemblyActivationRequest({
        ...request,
        configSnapshot: undefined
      })
    ).toThrow();
    expect(() =>
      decodeAssemblyActivationRequest({
        ...request,
        configSnapshot: { snapshotId: 'sha256:public-content-hash' }
      })
    ).toThrow(/32 lowercase hex/);
    expect(() =>
      decodeAssemblyActivationControl({
        ...transition('prepared'),
        configSnapshot: undefined
      })
    ).toThrow();
  });

  it('rejects durable state when either side of the committed/pending pair is absent', () => {
    const base = {
      schemaVersion: 'skiff-environment-activation-state-v2',
      environment: 'test',
      committed: { generation: 0, assembly, configSnapshot },
      pending: null
    };
    expect(decodeEnvironmentActivationState(base)).toEqual(base);
    expect(() =>
      decodeEnvironmentActivationState({
        ...base,
        committed: { generation: 0, assembly }
      })
    ).toThrow();
    expect(() =>
      decodeEnvironmentActivationState({
        ...base,
        pending: {
          activationId: 'activation-1',
          expectedGeneration: 0,
          candidateGeneration: 1,
          assembly,
          participantReplicaIds: ['runtime-a']
        }
      })
    ).toThrow();
  });

  it.each(['prepare', 'commit'])('roundtrips strict serviceDb on %s', (type) => {
    const input = {
      ...transition(type),
      serviceDb: { mongoUrl: 'mongodb://127.0.0.1:45123/test?replicaSet=rs0' }
    };
    expect(decodeAssemblyActivationControl(input)).toEqual(input);
  });

  it.each([
    { mongoUrl: '' },
    { mongoUrl: '   ' },
    { mongoUrl: 42 },
    { mongoUrl: 'mongodb://db', storageNamespace: 'legacy' },
    { mongoUrl: 'mongodb://db', retryWrites: true }
  ])('rejects invalid serviceDb %#', (serviceDb) => {
    expect(() =>
      decodeAssemblyActivationControl({ ...transition('prepare'), serviceDb })
    ).toThrow();
  });

  it.each(['prepared', 'abort'])('rejects serviceDb on runtime response/control %s', (type) => {
    expect(() =>
      decodeAssemblyActivationControl({
        ...transition(type),
        serviceDb: { mongoUrl: 'mongodb://db' }
      })
    ).toThrow();
  });

  it('rejects serviceDb from the public activation request', () => {
    expect(() =>
      decodeAssemblyActivationRequest({
        schemaVersion: 'skiff-assembly-activation-request-v2',
        environment: 'test',
        activationId: 'activation-1',
        expectedGeneration: 0,
        assembly,
        configSnapshot,
        serviceDb: { mongoUrl: 'mongodb://db' }
      })
    ).toThrow();
  });

  it('cannot persist mongoUrl in durable activation state', () => {
    expect(() =>
      decodeEnvironmentActivationState({
        schemaVersion: 'skiff-environment-activation-state-v2',
        environment: 'test',
        committed: {
          generation: 0,
          assembly,
          configSnapshot,
          serviceDb: { mongoUrl: 'mongodb://db' }
        },
        pending: null
      })
    ).toThrow();
  });
});
