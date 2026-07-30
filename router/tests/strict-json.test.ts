import { describe, expect, it } from 'vitest';

import { parseStrictActivationJson } from '../src/protocol/strictActivationJson.js';
import { parseStrictJson } from '../src/protocol/strictJson.js';

describe('strict JSON', () => {
  it('accepts the complete canonical JSON number grammar', () => {
    expect(parseStrictJson('[-2,0.1,1e3,-2.5E-2]')).toEqual([
      -2,
      0.1,
      1_000,
      -0.025,
    ]);
  });

  it.each([
    ['leading zero', '01'],
    ['negative leading zero', '-01'],
    ['missing fraction digits', '1.'],
    ['missing exponent digits', '1e'],
    ['missing signed exponent digits', '1e+'],
    ['trailing input', '1 true'],
  ])('rejects %s', (_label, source) => {
    expect(() => parseStrictJson(source)).toThrow();
  });

  it('rejects duplicate keys and invalid surrogate escapes', () => {
    expect(() => parseStrictJson('{"key":1,"key":2}')).toThrow(
      /duplicate JSON object key/,
    );
    expect(() => parseStrictJson('"\\ud800"')).toThrow(/lone high surrogate/);
    expect(() => parseStrictJson('"\\udc00"')).toThrow(/lone low surrogate/);
  });

  it('rejects invalid UTF-8', () => {
    expect(() => parseStrictJson(Uint8Array.from([0x22, 0xff, 0x22]))).toThrow();
  });
});

describe('strict activation JSON', () => {
  it.each([
    ['negative', '-1'],
    ['fraction', '0.1'],
    ['exponent', '1e3'],
    ['unsafe integer', '9007199254740992'],
  ])('continues to reject %s generation syntax', (_label, generation) => {
    expect(() =>
      parseStrictActivationJson(`{"generation":${generation}}`),
    ).toThrow();
  });
});
