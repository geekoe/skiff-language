const websocketAuthoringBlocker = [
  'canonical WebSocket smoke is blocked by production authoring:',
  'package boundary projection cannot prove std.websocket adapter types/native capabilities,',
  'while canonical WebSocket receive discards ordinary return payloads.',
  'Compiler/deployment must publish typed WebSocket adapter proof before this smoke can run.',
].join(' ');

/// The hermetic state-machine self-test remains available. A real smoke fails
/// before allocating an isolated stack until the compiler-owned adapter proof
/// exists; test infrastructure must not manufacture or re-sign that artifact.
export async function runPackageServiceEcosystemSmoke() {
  throw new Error(websocketAuthoringBlocker);
}

export const packageServiceEcosystemSmokeBlocker = websocketAuthoringBlocker;
