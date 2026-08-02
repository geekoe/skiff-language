// Differential extension: real client WebSocket traffic into the real
// Runtime (`differential_ext_ws_*` scenarios, plan §9).
//
// Uses the same `ws` module resolution as the runtime relay
// (router/package.json) and drives generation cycles, close-oldest business
// replacement and the frozen JSON-RPC id lexeme corpus. connectionId values
// are timestamped router mints (`wsconn-<nanos>-<n>`) and therefore only
// appear in the recordOnly evidence, never in compared values.

import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

import { loadRelayWebSocket } from './relay.mjs';

export const EXT_WS_SERVICE_ID = 'test.skiff/router-rust-differential-ext-ws';
export const EXT_WS_VERSION = '0.1.0';
export const EXT_WS_PATH = '/chat?x=1';

const JSONRPC_IDS_CORPUS_REPO_PATH =
  'runtime/transport/testdata/client-ws/jsonrpc-ids.json';
const CONNECT_TIMEOUT_MS = 30_000;
const MESSAGE_TIMEOUT_MS = 20_000;
const CLOSE_TIMEOUT_MS = 20_000;

const scriptDir = dirname(fileURLToPath(import.meta.url));

function statusRequest(id) {
  return JSON.stringify({
    jsonrpc: '2.0',
    id,
    method: 'status.get',
    params: [],
  });
}

function statusResponseSummary(response) {
  return {
    accepted: response?.result?.accepted ?? null,
    echo: response?.result?.echo ?? null,
    businessIdentity: response?.result?.businessIdentity ?? null,
    errorCode: response?.error?.code ?? null,
  };
}

async function openWsClient(side) {
  const WS = await loadRelayWebSocket();
  return await new Promise((resolvePromise, reject) => {
    const socket = new WS(`ws://127.0.0.1:${side.httpPort}${EXT_WS_PATH}`, {
      headers: {
        'x-skiff-service': EXT_WS_SERVICE_ID,
        'x-skiff-version': EXT_WS_VERSION,
      },
      handshakeTimeout: CONNECT_TIMEOUT_MS,
    });
    const timer = setTimeout(() => {
      socket.terminate();
      reject(new Error(`client WS connect timed out after ${CONNECT_TIMEOUT_MS}ms`));
    }, CONNECT_TIMEOUT_MS);
    socket.once('open', () => {
      clearTimeout(timer);
      resolvePromise(socket);
    });
    socket.once('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    socket.once('unexpected-response', (_request, response) => {
      clearTimeout(timer);
      socket.terminate();
      reject(new Error(`client WS connect returned HTTP ${response.statusCode}`));
    });
  });
}

function wsSend(socket, text) {
  return new Promise((resolvePromise, reject) => {
    // The ws send callback uses a Node-style `(err)` where `null` means
    // success; treat both `null` and `undefined` as success.
    socket.send(text, (error) => (error == null ? resolvePromise() : reject(error)));
  });
}

function wsNextMessage(socket, { timeoutMs = MESSAGE_TIMEOUT_MS } = {}) {
  return new Promise((resolvePromise, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error(`client WS message timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    const onMessage = (data) => {
      cleanup();
      resolvePromise(String(data));
    };
    const onClose = (code, reason) => {
      cleanup();
      reject(new Error(`client WS closed before message: ${code} ${String(reason)}`));
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    function cleanup() {
      clearTimeout(timer);
      socket.off('message', onMessage);
      socket.off('close', onClose);
      socket.off('error', onError);
    }
    socket.on('message', onMessage);
    socket.on('close', onClose);
    socket.on('error', onError);
  });
}

function wsAwaitClose(socket, { timeoutMs = CLOSE_TIMEOUT_MS } = {}) {
  return new Promise((resolvePromise, reject) => {
    if (socket.readyState === 3 /* CLOSED */) {
      resolvePromise({ code: 1005, reason: '' });
      return;
    }
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error(`client WS close timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    const onClose = (code, reason) => {
      cleanup();
      resolvePromise({ code, reason: String(reason) });
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    function cleanup() {
      clearTimeout(timer);
      socket.off('close', onClose);
      socket.off('error', onError);
    }
    socket.on('close', onClose);
    socket.on('error', onError);
    socket.close();
  });
}

function wsWaitForClose(socket, { timeoutMs = CLOSE_TIMEOUT_MS } = {}) {
  return new Promise((resolvePromise, reject) => {
    if (socket.readyState === 3 /* CLOSED */) {
      resolvePromise({ code: 1005, reason: '' });
      return;
    }
    const timer = setTimeout(() => {
      cleanup();
      reject(new Error(`client WS server close timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    const onClose = (code, reason) => {
      cleanup();
      resolvePromise({ code, reason: String(reason) });
    };
    const onError = (error) => {
      cleanup();
      reject(error);
    };
    function cleanup() {
      clearTimeout(timer);
      socket.off('close', onClose);
      socket.off('error', onError);
    }
    socket.on('close', onClose);
    socket.on('error', onError);
  });
}

async function wsClose(socket) {
  if (socket.readyState === 3) {
    return { code: 1005, reason: '' };
  }
  return await wsAwaitClose(socket);
}

async function statusRoundtrip(side, id) {
  const socket = await openWsClient(side);
  try {
    await wsSend(socket, statusRequest(id));
    const text = await wsNextMessage(socket);
    return {
      socket,
      responseText: text,
      summary: statusResponseSummary(JSON.parse(text)),
    };
  } catch (error) {
    socket.terminate();
    throw error;
  }
}

async function captureGeneration(side) {
  const cycles = [];
  for (let index = 0; index < 2; index += 1) {
    const { socket, responseText, summary } = await statusRoundtrip(side, `gen-${index}`);
    const rawResponse = JSON.parse(responseText);
    await wsClose(socket);
    cycles.push({
      connectStatus: 101,
      response: summary,
      rawResponse,
    });
  }
  return {
    clientWs: {
      generation: {
        connectStatuses: cycles.map((cycle) => cycle.connectStatus),
        responses: cycles.map((cycle) => cycle.response),
        rawResponses: cycles.map((cycle) => cycle.rawResponse),
      },
    },
  };
}

async function captureReplacement(side) {
  const first = await openWsClient(side);
  try {
    await wsSend(first, statusRequest('first'));
    const firstText = await wsNextMessage(first);
    const firstSummary = statusResponseSummary(JSON.parse(firstText));
    // Attach the close waiter before opening the second socket: close-oldest
    // may fire immediately after the second connection is admitted.
    const firstClosePromise = wsWaitForClose(first);
    const second = await openWsClient(side);
    const firstClose = await firstClosePromise;
    await wsSend(second, statusRequest('second'));
    const secondText = await wsNextMessage(second);
    const secondSummary = statusResponseSummary(JSON.parse(secondText));
    await wsClose(second);
    return {
      clientWs: {
        replacement: {
          firstConnectStatus: 101,
          secondConnectStatus: 101,
          closeCode: firstClose.code,
          closeReason: firstClose.reason,
          responses: [firstSummary, secondSummary],
        },
      },
    };
  } catch (error) {
    first.terminate();
    throw error;
  }
}

async function captureIdLexical(side) {
  const corpusPath = resolve(
    join(scriptDir, '..', '..', '..'),
    JSONRPC_IDS_CORPUS_REPO_PATH,
  );
  const corpus = JSON.parse(await readFile(corpusPath, 'utf8'));
  const cases = [];
  for (const corpusCase of corpus.cases) {
    if (corpusCase.kind !== 'request' && corpusCase.kind !== 'platformError') {
      continue;
    }
    const socket = await openWsClient(side);
    try {
      await wsSend(socket, corpusCase.frame);
      const text = await wsNextMessage(socket);
      const parsed = JSON.parse(text);
      cases.push({
        name: corpusCase.name,
        id: parsed?.id ?? null,
        errorCode: parsed?.error?.code ?? null,
      });
    } catch (error) {
      socket.terminate();
      throw error;
    }
    await wsClose(socket);
    await delay(50);
  }
  return { clientWs: { idLexical: { cases } } };
}

export async function captureDifferentialExtWs({ side, scenario }) {
  switch (scenario.wsMode) {
    case 'generation':
      return await captureGeneration(side);
    case 'replacement':
      return await captureReplacement(side);
    case 'idLexical':
      return await captureIdLexical(side);
    default:
      throw new Error(`differential_ext_ws scenario requires wsMode, got ${JSON.stringify(scenario.wsMode)}`);
  }
}
