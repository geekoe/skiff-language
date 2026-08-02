// Extension registry for the differential harness (plan §9, batch 10).
//
// Scenario inventory entries may declare an `extension` (`http` / `ws` /
// `actor`); the harness invokes the registered capture after each side's
// Runtime handshake and merges the returned partial observation into the
// side observation. New extensions follow the `differential_ext_*` prefix
// (E-actor-parity uses `actor_parity_*` and registers here the same way).

import { captureDifferentialExtActor } from './differential_ext_actor.mjs';
import { captureDifferentialExtHttp } from './differential_ext_http.mjs';
import { captureDifferentialExtWs } from './differential_ext_ws.mjs';

export const DIFFERENTIAL_EXT_CAPTURES = Object.freeze({
  http: captureDifferentialExtHttp,
  ws: captureDifferentialExtWs,
  actor: captureDifferentialExtActor,
});

export async function runDifferentialExt({ side, scenario, resources }) {
  if (scenario?.extension === undefined) {
    return undefined;
  }
  const capture = DIFFERENTIAL_EXT_CAPTURES[scenario.extension];
  if (capture === undefined) {
    throw new Error(
      `scenario ${scenario.id} declares unknown extension ${JSON.stringify(scenario.extension)}; `
      + `available: ${Object.keys(DIFFERENTIAL_EXT_CAPTURES).join(', ')}`,
    );
  }
  return await capture({ side, scenario, resources });
}
