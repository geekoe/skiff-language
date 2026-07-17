import { spawn } from 'node:child_process';

/**
 * Runs one canonical closure-validation transaction. Dev sync does not consume
 * artifact contents, so it confirms only transaction cardinality, keys and the
 * assembly marker before publishing the already-staged roots.
 */
export async function validateArtifactClosureBatch(identityCliPath, candidates) {
  if (!Array.isArray(candidates) || candidates.length === 0) {
    throw new Error('artifact identity validation requires at least one service candidate');
  }
  const expected = new Map();
  for (const candidate of candidates) {
    const key = requiredString(candidate?.key, 'artifact identity candidate key');
    if (expected.has(key)) {
      throw new Error(`artifact identity candidates contain duplicate key ${key}`);
    }
    expected.set(key, candidate);
  }

  const stdout = await runIdentityCli(identityCliPath, { services: candidates });
  const response = jsonRecord(stdout, 'artifact identity CLI stdout');
  if (!Array.isArray(response.results) || response.results.length !== candidates.length) {
    throw new Error(`artifact identity CLI stdout.results must contain exactly ${candidates.length} results`);
  }

  const validated = new Map();
  for (const [index, raw] of response.results.entries()) {
    const result = record(raw, `artifact identity CLI results[${index}]`);
    const key = requiredString(result.key, `artifact identity CLI results[${index}].key`);
    const candidate = expected.get(key);
    if (candidate === undefined || validated.has(key)) {
      throw new Error(`artifact identity CLI returned unexpected or duplicate key ${key}`);
    }
    const assemblyIdentity = requiredString(
      result.assemblyIdentity,
      `artifact identity CLI results[${index}].assemblyIdentity`,
    );
    if (assemblyIdentity !== candidate.serviceAssembly.assemblyIdentity) {
      throw new Error(`artifact identity CLI result ${key} assemblyIdentity mismatch`);
    }
    validated.set(key, { key, assemblyIdentity });
  }
  return validated;
}

function runIdentityCli(identityCliPath, payload) {
  return new Promise((resolvePromise, reject) => {
    let child;
    try {
      child = spawn(identityCliPath, ['runtime-program-build-id'], {
        stdio: ['pipe', 'pipe', 'pipe'],
      });
    } catch (error) {
      reject(new Error(`failed to start artifact identity CLI ${identityCliPath}`, { cause: error }));
      return;
    }
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.once('error', (error) => {
      reject(new Error(`artifact identity CLI ${identityCliPath} failed to start`, { cause: error }));
    });
    child.once('close', (code, signal) => {
      if (code !== 0 || signal !== null) {
        reject(new Error(
          `artifact identity CLI failed${code === null ? '' : ` with exit ${code}`}${signal === null ? '' : ` from signal ${signal}`}${stderr.trim() ? `: ${stderr.trim()}` : ''}`,
        ));
        return;
      }
      resolvePromise(stdout);
    });
    child.stdin.end(`${JSON.stringify(payload)}\n`);
  });
}

function jsonRecord(text, label) {
  try {
    return record(JSON.parse(text), label);
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error(`${label} must be valid JSON`, { cause: error });
    }
    throw error;
  }
}

function record(value, label) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value;
}

function requiredString(value, label) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${label} must be a non-empty string`);
  }
  return value;
}
