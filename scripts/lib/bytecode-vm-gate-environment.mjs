const ALLOWED_GIT_ENVIRONMENT = new Set([
  // Candidate probes are plumbing commands with captured, non-TTY streams, so
  // the pager cannot redirect repository discovery, objects, index, config, or
  // worktree state. It remains in the complete receipted child environment.
  'GIT_PAGER',
]);

export function assertBytecodeVmGateEnvironment(environment) {
  if (environment === null || typeof environment !== 'object' || Array.isArray(environment)) {
    throw new Error('Bytecode VM Gate requires an explicit environment object');
  }
  const rejected = Object.entries(environment)
    .filter(([name, value]) => typeof value === 'string'
      && name.startsWith('GIT_')
      && !ALLOWED_GIT_ENVIRONMENT.has(name))
    .map(([name]) => name)
    .sort();
  if (rejected.length > 0) {
    throw new Error(
      `Bytecode VM Gate refuses Git repository-control environment variable(s): ${rejected.join(', ')}; unset them before invocation`,
    );
  }
  return environment;
}
