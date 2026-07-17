import { spawn } from 'node:child_process';
import { isDeepStrictEqual } from 'node:util';

const dynamicBuildIdPattern = /^skiff-service-build-v1:sha256:[0-9a-f]{64}$/;

/**
 * Runs one canonical closure-validation transaction and returns the complete,
 * typed closure together with the source references proven by that transaction.
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
    const dynamicBuildId = requiredString(
      result.dynamicBuildId,
      `artifact identity CLI results[${index}].dynamicBuildId`,
    );
    if (!dynamicBuildIdPattern.test(dynamicBuildId)) {
      throw new Error(
        `artifact identity CLI results[${index}].dynamicBuildId must be skiff-service-build-v1:sha256:<64 lowercase hex>`,
      );
    }
    const assemblyIdentity = requiredString(
      result.assemblyIdentity,
      `artifact identity CLI results[${index}].assemblyIdentity`,
    );
    if (assemblyIdentity !== candidate.serviceAssembly.assemblyIdentity) {
      throw new Error(`artifact identity CLI result ${key} assemblyIdentity mismatch`);
    }
    const serviceAssembly = validatedArtifact(
      candidate.serviceAssembly,
      result.serviceAssembly,
      `artifact identity CLI results[${index}].serviceAssembly`,
      candidate.serviceAssembly.assemblyPath,
    );
    const serviceUnit = validatedArtifact(
      candidate.serviceUnit,
      result.serviceUnit,
      `artifact identity CLI results[${index}].serviceUnit`,
      candidate.serviceUnit.unitPath,
    );
    const packageUnits = validatedPackageArtifacts(
      candidate.packageUnits,
      result.packageUnits,
      `artifact identity CLI results[${index}].packageUnits`,
    );
    validated.set(key, {
      key,
      dynamicBuildId,
      assemblyIdentity,
      serviceAssembly,
      serviceUnit,
      packageUnits,
    });
  }
  return validated;
}

/**
 * Exact matching is the JS trust boundary: callers may only use the returned
 * references for filesystem access after the Rust CLI has validated the source
 * closure and the target wire has matched it field-for-field.
 */
export function assertArtifactReferencesMatchValidated(actual, validated, label) {
  const expected = artifactReferencesFromValidated(validated, label);
  for (const field of ['serviceAssembly', 'serviceUnit', 'packageUnits']) {
    if (!isDeepStrictEqual(actual?.[field], expected[field])) {
      throw new Error(`${label} ${field} does not match validated artifact references`);
    }
  }
  return expected;
}

function artifactReferencesFromValidated(validated, label) {
  const closure = record(validated, `${label} validated closure`);
  const serviceAssembly = record(
    closure.serviceAssembly,
    `${label} validated closure serviceAssembly`,
  );
  const serviceUnit = record(closure.serviceUnit, `${label} validated closure serviceUnit`);
  if (!Array.isArray(closure.packageUnits)) {
    throw new Error(`${label} validated closure packageUnits must be an array`);
  }
  return {
    serviceAssembly: record(
      serviceAssembly.reference,
      `${label} validated closure serviceAssembly.reference`,
    ),
    serviceUnit: record(
      serviceUnit.reference,
      `${label} validated closure serviceUnit.reference`,
    ),
    packageUnits: closure.packageUnits.map((unit, index) => record(
      record(unit, `${label} validated closure packageUnits[${index}]`).reference,
      `${label} validated closure packageUnits[${index}].reference`,
    )),
  };
}

function validatedPackageArtifacts(references, rawContents, label) {
  if (!Array.isArray(rawContents)) {
    throw new Error(`${label} must be an array`);
  }
  if (rawContents.length !== references.length) {
    throw new Error(`${label} must contain exactly ${references.length} entries`);
  }
  const byPath = new Map();
  for (const [index, rawContent] of rawContents.entries()) {
    const content = validatedContent(rawContent, `${label}[${index}]`);
    if (byPath.has(content.path)) {
      throw new Error(`${label} contains duplicate path ${content.path}`);
    }
    byPath.set(content.path, content);
  }
  return references.map((reference, index) => {
    const content = byPath.get(reference.unitPath);
    if (content === undefined) {
      throw new Error(`${label} is missing validated content for ${reference.unitPath}`);
    }
    byPath.delete(reference.unitPath);
    return validatedArtifact(reference, content, `${label}[${index}]`, reference.unitPath);
  });
}

function validatedArtifact(reference, rawContent, label, expectedPath) {
  const content = rawContent?.path === expectedPath && rawContent?.value !== undefined
    ? rawContent
    : validatedContent(rawContent, label);
  if (content.path !== expectedPath) {
    throw new Error(`${label}.path must be ${expectedPath}`);
  }
  return {
    reference: { ...reference },
    content: {
      path: content.path,
      value: record(content.value, `${label}.value`),
    },
  };
}

function validatedContent(value, label) {
  const content = record(value, label);
  return {
    path: requiredString(content.path, `${label}.path`),
    value: record(content.value, `${label}.value`),
  };
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
