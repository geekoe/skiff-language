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
    'instance-managed-component', 'startProcess', 'managed-component',
    'instance.yml-driven local process supervisor retains pid files and log descriptors'),
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
  owner('scripts/check-router-chat-live.mjs', 'spawn', 'spawn',
    'router-chat-live-spawn', 'spawnManaged', 'owned-process-group',
    'router-live:chat spawns the real Router and Runtime processes and retains their handles for TERM/cleanup'),
  owner('scripts/lib/mongod-live-harness.mjs', 'spawn', 'spawn',
    'mongod-live-spawn', 'spawnMongodProcess', 'managed-component',
    'temporary mongod replica-set process retained for cleanup and exit observation'),
  owner('scripts/lib/clean-host-bundle.mjs', 'execFile', 'execFile',
    'clean-host-exec-file', 'execFileAsync', 'domain-adapter',
    'clean-host PATH probe runs sh and preserves its output/exit semantics'),
  owner('scripts/lib/http_live_process.mjs', 'spawn', 'spawn',
    'http-live-process-spawn', 'spawnLoggedProcess', 'managed-component',
    'router-live:http Router/Runtime processes retain handles for TERM and log close'),
  owner('scripts/lib/http_live_suite.mjs', 'spawn', 'spawn',
    'http-live-suite-helper', 'caseBackpressure', 'managed-component',
    'slow-client helper process retained for TERM on failure'),
  {
    path: 'scripts/check-rust-file-lines.mjs',
    importedSymbol: 'execFileSync',
    localAlias: 'execFileSync',
    ownerId: 'rust-file-line-gate',
    ownerFunction: 'runFileLineGate',
    callCount: 2,
    ownerClass: COMMAND_OWNER_CLASSES.DOMAIN_ADAPTER,
    reason: 'rust file line gate runs rg and wc synchronously and preserves their output/exit semantics',
  },
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
