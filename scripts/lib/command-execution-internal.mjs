export function safeSpawnFailure(command, error) {
  const code = safeErrorCode(error);
  return Object.freeze({
    name: 'SpawnFailure',
    code,
    command,
    message: `failed to spawn ${command}: ${code ?? 'UNKNOWN'}`,
  });
}

export function commandExecutionError(command, outcome, streams) {
  const spawnFailure = outcome.error;
  const code = spawnFailure?.code ?? outcome.code ?? null;
  const signal = outcome.signal ?? null;
  const message = spawnFailure === null
    ? `${command} exited with ${signal ?? code}`
    : spawnFailure.message;
  const error = new Error(message);
  error.name = 'CommandExecutionError';
  Object.defineProperties(error, {
    command: { value: command, enumerable: true },
    code: { value: code, enumerable: true },
    signal: { value: signal, enumerable: true },
  });
  if (streams !== undefined) {
    Object.defineProperties(error, {
      stdout: { value: streams.stdout, enumerable: false },
      stderr: { value: streams.stderr, enumerable: false },
    });
  }
  return error;
}

export function safeErrorClone(error, fallbackMessage) {
  const message = typeof error?.message === 'string' && error.message.length > 0
    ? error.message
    : fallbackMessage;
  const clone = new Error(message);
  const code = safeErrorCode(error);
  if (code !== null) {
    Object.defineProperty(clone, 'code', { value: code, enumerable: true });
  }
  return clone;
}

function safeErrorCode(error) {
  return typeof error?.code === 'string' || typeof error?.code === 'number'
    ? error.code
    : null;
}
