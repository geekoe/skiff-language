import { describe, expect, it } from 'vitest';

import { compilePathPattern } from '../src/router/pathPattern.js';

describe('compilePathPattern', () => {
  it('matches literal and parameter segments without normalizing trailing slashes', () => {
    const pattern = compilePathPattern('/rooms/{room}/echo');

    expect(pattern.shape).toBe('/rooms/{}/echo');
    expect(pattern.match('/rooms/general/echo')).toEqual({ room: 'general' });
    expect(pattern.match('/rooms/general/echo/')).toBeNull();
    expect(pattern.match('/rooms/general')).toBeNull();
  });

  it('rejects duplicate path parameters', () => {
    expect(() => compilePathPattern('/users/{id}/posts/{id}')).toThrow(
      /duplicate path parameter/
    );
  });
});
