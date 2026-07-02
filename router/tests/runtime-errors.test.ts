import { describe, expect, it } from 'vitest';

import { RuntimeResponseError } from '../src/router/errors.js';

describe('runtime error HTTP mapping', () => {
  it('maps module decode errors to 400 with details', () => {
    for (const code of [
      'config.DecodeError',
      'std.bytes.DecodeError',
      'std.number.DecodeError',
      'std.json.DecodeError',
      'std.db.DecodeError',
      'std.file.FileError',
      'std.time.DecodeError'
    ]) {
      const error = new RuntimeResponseError({
        code,
        message: `${code} failed`,
        details: { target: code }
      });

      expect(error.statusCode).toBe(400);
      expect(error.toHttpBody()).toEqual({
        message: `${code} failed`,
        detail: { target: code }
      });
    }
  });
});
