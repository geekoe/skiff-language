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

function transition(type: string) {
  return {
    type,
    environment: 'test',
    activationId: 'activation-1',
    expectedGeneration: 0,
    candidateGeneration: 1,
    assembly,
    replicaId: 'runtime-a'
  };
}

describe('assembly activation serviceDb wire', () => {
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
        schemaVersion: 'skiff-assembly-activation-request-v1',
        environment: 'test',
        activationId: 'activation-1',
        expectedGeneration: 0,
        assembly,
        serviceDb: { mongoUrl: 'mongodb://db' }
      })
    ).toThrow();
  });

  it('cannot persist mongoUrl in durable activation state', () => {
    expect(() =>
      decodeEnvironmentActivationState({
        schemaVersion: 'skiff-environment-activation-state-v1',
        environment: 'test',
        committed: {
          generation: 0,
          assembly,
          serviceDb: { mongoUrl: 'mongodb://db' }
        },
        pending: null
      })
    ).toThrow();
  });
});
