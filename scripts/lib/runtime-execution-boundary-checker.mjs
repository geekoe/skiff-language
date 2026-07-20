import {
  inspectRuntimeExecutionBoundaryOwners,
  runtimeExecutionBoundaryViolation,
  validateRuntimeExecutionBoundaryRegistry,
} from './runtime-execution-boundary-registry.mjs';
import { checkRuntimeExecutionBoundaryRules } from './runtime-execution-boundary-rules.mjs';
import { loadRuntimeExecutionBoundarySources } from './runtime-execution-boundary-source.mjs';
import { PROPOSED_RUNTIME_EXECUTION_BOUNDARY_REGISTRY } from './runtime-execution-boundary-subjects.mjs';

export async function collectRuntimeExecutionBoundaryViolations(
  repoRoot,
  registry = PROPOSED_RUNTIME_EXECUTION_BOUNDARY_REGISTRY,
) {
  const violations = validateRuntimeExecutionBoundaryRegistry(registry);
  if (violations.some(({ id }) => id === 'invalid-execution-boundary-registry')) {
    return sortAndDedupe(violations);
  }

  const { missingRoots, sources } = await loadRuntimeExecutionBoundarySources(
    repoRoot,
    registry.sourceRoots,
  );
  for (const sourceRoot of missingRoots) {
    violations.push(runtimeExecutionBoundaryViolation({
      id: 'execution-source-root-missing',
      subject: sourceRoot.id,
      relPath: sourceRoot.root,
      detail: `registered ${sourceRoot.language} production root is absent`,
    }));
  }

  for (const subject of registry.subjects) {
    for (const relPath of subject.requiredFiles ?? []) {
      if (!sources.has(relPath)) {
        violations.push(runtimeExecutionBoundaryViolation({
          id: 'required-subject-file-missing',
          subject: subject.id,
          relPath,
          detail: `required production subject file ${relPath} is absent or test-only`,
        }));
      }
    }
  }

  const ownerMatches = inspectRuntimeExecutionBoundaryOwners(registry, sources, violations);
  checkRuntimeExecutionBoundaryRules(registry, sources, ownerMatches, violations);
  return sortAndDedupe(violations);
}

export function formatRuntimeExecutionBoundaryViolation(entry) {
  const location = entry.relPath
    ? `${entry.relPath}${entry.line ? `:${entry.line}` : ''}`
    : '<execution-boundary-registry>';
  const subject = entry.subject ? ` subject=${entry.subject}` : '';
  const owner = entry.ownerRole ? ` owner=${entry.ownerRole}` : '';
  const matched = entry.matched ? ` matched=${JSON.stringify(entry.matched)}` : '';
  return `${location} ${entry.id}${subject}${owner}${matched}: ${entry.detail}`;
}

function sortAndDedupe(violations) {
  const unique = new Map();
  for (const entry of violations) {
    const key = [
      entry.id,
      entry.subject ?? '',
      entry.ownerRole ?? '',
      entry.relPath ?? '',
      entry.line ?? 0,
      entry.matched ?? '',
      entry.detail,
    ].join('\0');
    unique.set(key, entry);
  }
  return [...unique.values()].sort((left, right) => {
    const pathOrder = (left.relPath ?? '').localeCompare(right.relPath ?? '');
    if (pathOrder !== 0) {
      return pathOrder;
    }
    const lineOrder = (left.line ?? 0) - (right.line ?? 0);
    if (lineOrder !== 0) {
      return lineOrder;
    }
    const idOrder = left.id.localeCompare(right.id);
    return idOrder !== 0
      ? idOrder
      : (left.ownerRole ?? '').localeCompare(right.ownerRole ?? '');
  });
}
