import { createHash } from 'node:crypto';

export function stableStringify(value: unknown): string {
  return JSON.stringify(sortForJson(value));
}

export function sha256Hex(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function sortForJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => sortForJson(item));
  }

  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    const result: Record<string, unknown> = {};
    for (const key of Object.keys(record).sort()) {
      const nested = record[key];
      if (nested !== undefined) {
        result[key] = sortForJson(nested);
      }
    }
    return result;
  }

  return value;
}
