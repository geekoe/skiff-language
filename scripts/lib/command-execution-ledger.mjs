export const COMMAND_OWNER_CLASSES = Object.freeze({
  ATTACHED_PRIMITIVE: 'attached-primitive',
  OWNED_PROCESS_GROUP: 'owned-process-group',
  BROWSER: 'browser',
  MANAGED_COMPONENT: 'managed-component',
  SUPERVISOR: 'supervisor',
  BINARY_ADAPTER: 'binary-adapter',
  TIMEOUT_OWNER: 'timeout-owner',
  DOMAIN_ADAPTER: 'domain-adapter',
});

export const COMMAND_EXECUTION_LEDGER = deepFreeze([
  owner('scripts/lib/command-execution.mjs', 'spawn', 'spawnCommandChild',
    'attached-capture-spawn', 'spawnAttachedChild', 'attached-primitive',
    'canonical attached and capture spawn boundary'),
  owner('scripts/lib/owned-command.mjs', 'spawn', 'spawnOwnedChild',
    'owned-process-group', 'runOwnedCommand', 'owned-process-group',
    'detached process-group Abort/TERM/KILL owner'),
  owner('scripts/lib/owned-command.mjs', 'spawn', 'spawnOwnedCapturedChild',
    'owned-captured-process-group', 'captureOwnedCommand', 'owned-process-group',
    'captured owned command drains stdio and owns detached process-group Abort/TERM/KILL'),
  owner('scripts/skiff-instance.mjs', 'spawn', 'spawnManagedChild',
    'managed-component', 'startManagedProcess', 'managed-component',
    'managed component retains PID, PGID, and log descriptors'),
  owner('scripts/lib/isolated-test-runtime-instance.mjs', 'spawn', 'spawnSupervisorChild',
    'isolated-supervisor', 'isolatedInstanceOperations', 'supervisor',
    'isolated runtime lifecycle retains the supervisor child handle'),
  owner('scripts/lib/isolated-test-runtime.mjs', 'spawn', 'spawnAdditionalRuntimeChild',
    'isolated-additional-runtime', 'startIsolatedTestRuntime', 'managed-component',
    'isolated runtime cleanup retains the child handle, sends TERM, escalates to KILL after timeout, and awaits exit'),
  owner('scripts/lib/platform-source-probe-support.mjs', 'spawn', 'spawnPlatformSourceProbeChild',
    'platform-source-probe-group', 'captureOwnedCommand', 'owned-process-group',
    'abort and post-close retirement apply TERM/KILL to the detached process group and verify observed ports are closed'),
  owner('scripts/lib/source-key.mjs', 'spawn', 'spawnGitBufferChild',
    'git-buffer', 'gitBuffer', 'binary-adapter',
    'source-key hashing requires binary git stdout'),
  owner('scripts/lib/source-key.mjs', 'spawn', 'spawnGitExitChild',
    'git-exit-code', 'gitExitCode', 'binary-adapter',
    'source-key comparison preserves git exit-code semantics'),
  owner('scripts/lib/crate-public-api-rustdoc.mjs', 'spawn', 'spawnRustdocChild',
    'rustdoc-timeout', 'runCommand', 'timeout-owner',
    'rustdoc nightly probe owns timeout and kill behavior'),
  owner('scripts/lib/loop-risk-stress-node.mjs', 'execFile', 'execLoopRiskCpuSample',
    'loop-risk-cpu-sample', 'readProcessCpu', 'domain-adapter',
    'loop-risk stress samples runtime CPU with ps'),
  owner('scripts/lib/loop-risk-stress-node.mjs', 'execFile', 'execLoopRiskPgrep',
    'loop-risk-pgrep', 'findRuntimePids', 'domain-adapter',
    'explicit diagnostic pgrep preserves domain outcome semantics'),
]);

function owner(
  path,
  importedSymbol,
  localAlias,
  ownerId,
  ownerFunction,
  ownerClass,
  reason,
) {
  return {
    path,
    importedSymbol,
    localAlias,
    ownerId,
    ownerFunction,
    callCount: 1,
    ownerClass,
    reason,
  };
}

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) {
      deepFreeze(child);
    }
    Object.freeze(value);
  }
  return value;
}
