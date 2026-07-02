export interface CompiledPathPattern {
  pattern: string;
  shape: string;
  match(pathname: string): Record<string, string> | null;
}

const PARAM_RE = /^\{([A-Za-z_][A-Za-z0-9_]*)\}$/;

export function compilePathPattern(pattern: string): CompiledPathPattern {
  if (!pattern.startsWith('/')) {
    throw new Error(`path pattern must start with /: ${pattern}`);
  }

  const segments = pattern.split('/');
  const shape = segments
    .map((segment) => (PARAM_RE.test(segment) ? '{}' : segment))
    .join('/');
  const paramNames = new Set<string>();

  for (const segment of segments) {
    const param = PARAM_RE.exec(segment)?.[1];
    if (!param) {
      continue;
    }
    if (paramNames.has(param)) {
      throw new Error(`duplicate path parameter ${param} in ${pattern}`);
    }
    paramNames.add(param);
  }

  return {
    pattern,
    shape,
    match(pathname: string): Record<string, string> | null {
      const actual = pathname.split('/');
      if (actual.length !== segments.length) {
        return null;
      }

      const params: Record<string, string> = {};
      for (let index = 0; index < segments.length; index += 1) {
        const expectedSegment = segments[index];
        const actualSegment = actual[index];
        if (expectedSegment === undefined || actualSegment === undefined) {
          return null;
        }

        const paramName = PARAM_RE.exec(expectedSegment)?.[1];
        if (paramName) {
          params[paramName] = actualSegment;
          continue;
        }

        if (expectedSegment !== actualSegment) {
          return null;
        }
      }

      return params;
    }
  };
}
