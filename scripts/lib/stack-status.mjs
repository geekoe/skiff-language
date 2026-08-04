import { loadStackConfig, parseStackYaml, requireRemoteStackConfig } from './stack-config.mjs';
import { createStackShell } from './stack-shell.mjs';

export async function stackStatus({
  configDir,
  skiffRoot,
  shell = createStackShell({ skiffRoot }),
}) {
  const stack = await loadStackConfig(configDir, { skiffRoot });
  requireRemoteStackConfig(stack, 'stack status');
  const profile = stack.config.profile;
  const { host, remoteSkiff } = stack.config.remote;
  const { controlPort, healthPath } = stack.config.verify;

  const remoteRouterText = await shell.remoteCapture(
    host,
    `cat ${remoteSkiff}/config/router.yml`,
  );
  const remoteRouter = parseStackYaml(remoteRouterText, 'remote router.yml');
  const remoteRouterProfile = remoteRouter.profile;
  if (remoteRouterProfile !== profile) {
    throw new Error(
      `stack profile mismatch: config.yml.profile=${JSON.stringify(profile)} but remote router.yml.profile=${JSON.stringify(remoteRouterProfile)}`,
    );
  }

  const healthText = await shell.remoteCapture(
    host,
    `curl -fsS http://127.0.0.1:${controlPort}${healthPath}`,
  );
  let health;
  try {
    health = JSON.parse(healthText);
  } catch (error) {
    throw new Error(`router health returned invalid JSON: ${error.message}`);
  }
  const activeAssembly = health?.activeAssembly;
  const activeProfile = activeAssembly?.profile;
  if (activeProfile !== profile) {
    throw new Error(
      `stack profile mismatch: config.yml.profile=${JSON.stringify(profile)} but health activeAssembly.profile=${JSON.stringify(activeProfile)}`,
    );
  }
  const generation = activeAssembly?.generation;
  if (!Number.isSafeInteger(generation) || generation < 0) {
    throw new Error(`router health activeAssembly.generation must be a non-negative integer`);
  }
  const replicas = health?.replicas;
  if (!Array.isArray(replicas)) {
    throw new Error('router health replicas must be an array');
  }
  const connectedReplica = replicas.find(
    (replica) => (
      replica?.connected === true
      && replica?.state === 'healthy'
      && replica?.profile === profile
      && replica?.generation === generation
    ),
  );
  if (connectedReplica === undefined) {
    throw new Error(`runtime is not connected and healthy for profile ${profile} at generation ${generation}`);
  }

  return {
    profile,
    generation,
    remoteRouterProfile,
    activeProfile,
    runtimeConnected: true,
    replicaId: connectedReplica.replicaId,
    health,
    remote: {
      host,
      remoteSkiff,
      healthUrl: `http://127.0.0.1:${controlPort}${healthPath}`,
    },
  };
}
