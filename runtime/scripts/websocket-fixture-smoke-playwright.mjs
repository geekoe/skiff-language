#!/usr/bin/env node

import { createServer } from 'node:http';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '..', '..');
const profileDir = path.join(repoRoot, '.playwright-profile');
const screenshotDir = path.join(repoRoot, '.browser-screenshot');

const keepArtifacts = process.env.SKIFF_KEEP_BROWSER_ARTIFACTS === '1';
const messageCount = parseMessageCount(process.env.SKIFF_WS_SMOKE_MESSAGES ?? '3');
const browserWebSocketOpenState = 1;
const wsUrl =
  process.env.SKIFF_WS_URL ??
  `ws://127.0.0.1:4000/ws?service=websocket_fixture&deviceId=browser-smoke-${Date.now()}&platform=web&clientVersion=1.0.0&language=en`;

const consoleFindings = [];
const networkFindings = [];
const wsFrames = [];
const requestIds = [];

let server;
let context;
let page;

try {
  const { chromium } = await importPlaywright();
  await resetScreenshotDir();
  await mkdir(profileDir, { recursive: true });

  server = await startTestPageServer();
  const testPageUrl = `http://127.0.0.1:${server.port}/__websocket-fixture-smoke.html`;

  context = await chromium.launchPersistentContext(profileDir, {
    args: ['--disable-features=MacAppCodeSignClone'],
    headless: process.env.SKIFF_HEADLESS !== '0',
    slowMo: Number(process.env.SKIFF_SLOW_MO_MS ?? '0'),
    viewport: { width: 1280, height: 900 },
  });

  page = await context.newPage();
  attachObservers(page);

  await page.goto(testPageUrl, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('[data-testid="smoke-root"]');
  await page.click('#smoke-start');

  const rendered = await page.evaluate(() => ({
    url: location.href,
    started: document.body.dataset.started === 'true',
    rootText: document.querySelector('[data-testid="smoke-root"]')?.textContent ?? '',
  }));
  assert(rendered.url === testPageUrl, `unexpected test page URL: ${rendered.url}`);
  assert(rendered.started, 'test page start button did not update page state');
  assert(
    rendered.rootText.includes('WebSocket Fixture Smoke'),
    'test page root did not render expected DOM'
  );

  const runId = `websocket-fixture-smoke-${Date.now().toString(36)}`;
  await page.evaluate((input) => window.__smoke.reset(input), { runId });
  await page.evaluate((input) => window.__smoke.connect(input), { wsUrl });

  for (let index = 1; index <= messageCount; index += 1) {
    const requestId = `${runId}-${index}`;
    requestIds.push(requestId);
    await page.evaluate((payload) => window.__smoke.send(payload), {
      tag: 'fixture_ping',
      requestId,
      input: { index },
    });
  }

  await page.waitForTimeout(250);
  const readyState = await page.evaluate(() => window.__smoke.readyState());
  assert(
    readyState === browserWebSocketOpenState,
    `websocket closed during smoke send; readyState=${readyState}`
  );
  await page.evaluate(() => window.__smoke.close());

  const storage = await readSmokeStorage(page);
  validateLocalStorage(storage, runId);
  validateObservers();

  console.log(
    JSON.stringify(
      {
        ok: true,
        wsUrl,
        messages: requestIds.length,
        websocketFrames: {
          sent: wsFrames.filter((frame) => frame.direction === 'sent').length,
          received: wsFrames.filter((frame) => frame.direction === 'received').length,
        },
        localStorageSends: storage.sent.length,
      },
      null,
      2
    )
  );
} catch (error) {
  if (keepArtifacts && page) {
    await saveFailureArtifacts(error);
  }
  throw error;
} finally {
  if (page && !page.isClosed()) {
    await page.close().catch(() => {});
  }
  if (context) {
    await context.close().catch(() => {});
  }
  if (server) {
    await server.close();
  }
  if (!keepArtifacts) {
    await rm(screenshotDir, { recursive: true, force: true });
  }
}

function parseMessageCount(value) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 10) {
    throw new Error('SKIFF_WS_SMOKE_MESSAGES must be an integer from 1 to 10');
  }
  return parsed;
}

async function importPlaywright() {
  try {
    return await import('playwright');
  } catch (error) {
    if (error?.code !== 'ERR_MODULE_NOT_FOUND') {
      throw error;
    }
    try {
      const requireFromCwd = createRequire(path.join(process.cwd(), 'package.json'));
      return requireFromCwd('playwright');
    } catch {
      throw new Error(
        [
          'Missing Playwright dependency.',
          'Install the script-local dependencies and retry:',
          '  cd scripts',
          '  pnpm install',
          '  pnpm websocket-fixture:smoke',
        ].join('\n')
      );
    }
  }
}

async function resetScreenshotDir() {
  await rm(screenshotDir, { recursive: true, force: true });
  await mkdir(screenshotDir, { recursive: true });
}

function startTestPageServer() {
  const testHtml = buildTestHtml();
  const localServer = createServer((request, response) => {
    if (request.url === '/favicon.ico') {
      response.writeHead(204);
      response.end();
      return;
    }
    if (request.url === '/__websocket-fixture-smoke.html') {
      response.writeHead(200, {
        'content-type': 'text/html; charset=utf-8',
        'cache-control': 'no-store',
      });
      response.end(testHtml);
      return;
    }
    response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
    response.end('not found');
  });

  return new Promise((resolve, reject) => {
    localServer.once('error', reject);
    localServer.listen(0, '127.0.0.1', () => {
      const address = localServer.address();
      if (!address || typeof address === 'string') {
        reject(new Error('test page server did not bind to a TCP port'));
        return;
      }
      resolve({
        port: address.port,
        close: () =>
          new Promise((closeResolve, closeReject) => {
            localServer.close((error) => (error ? closeReject(error) : closeResolve()));
          }),
      });
    });
  });
}

function buildTestHtml() {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Skiff WebSocket Fixture Smoke</title>
    <style>
      body { font-family: system-ui, sans-serif; margin: 24px; color: #17202a; }
      button { min-height: 40px; padding: 0 14px; }
      #smoke-events { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
    </style>
  </head>
  <body>
    <main id="smoke-root" data-testid="smoke-root">
      <h1>WebSocket Fixture Smoke</h1>
      <button id="smoke-start" type="button">Start</button>
      <ul id="smoke-events"></ul>
    </main>
    <script>
      const storagePrefix = 'skiff:websocket-fixture-smoke:';
      const eventList = document.querySelector('#smoke-events');
      let ws;
      let runId = '';

      document.querySelector('#smoke-start').addEventListener('click', () => {
        document.body.dataset.started = 'true';
        record('button.clicked', {});
      });

      function record(kind, payload) {
        const item = document.createElement('li');
        item.dataset.kind = kind;
        item.textContent = kind + ' ' + JSON.stringify(payload);
        eventList.appendChild(item);
        console.info('[websocket-fixture-smoke]', kind, JSON.stringify(payload));
      }

      function setJson(key, value) {
        localStorage.setItem(storagePrefix + key, JSON.stringify(value));
      }

      function getJson(key, fallback) {
        const raw = localStorage.getItem(storagePrefix + key);
        return raw ? JSON.parse(raw) : fallback;
      }

      window.__smoke = {
        reset(input) {
          runId = input.runId;
          for (const key of Object.keys(localStorage)) {
            if (key.startsWith(storagePrefix)) {
              localStorage.removeItem(key);
            }
          }
          setJson('run', { runId, startedAt: new Date().toISOString() });
          setJson('sent', []);
          record('storage.reset', { runId });
        },
        connect(input) {
          return new Promise((resolve, reject) => {
            ws = new WebSocket(input.wsUrl);
            const timer = setTimeout(() => reject(new Error('timed out waiting for websocket open')), 5000);
            ws.addEventListener('open', () => {
              clearTimeout(timer);
              record('websocket.open', { url: input.wsUrl });
              resolve();
            }, { once: true });
            ws.addEventListener('message', (event) => {
              record('websocket.message', { data: String(event.data) });
            });
            ws.addEventListener('error', () => {
              clearTimeout(timer);
              reject(new Error('websocket error'));
            }, { once: true });
            ws.addEventListener('close', (event) => {
              record('websocket.close', { code: event.code, reason: event.reason });
            });
          });
        },
        send(input) {
          if (!ws || ws.readyState !== WebSocket.OPEN) {
            throw new Error('websocket is not open');
          }
          const sent = getJson('sent', []);
          sent.push({
            runId,
            tag: input.tag,
            requestId: input.requestId,
            at: new Date().toISOString(),
          });
          setJson('sent', sent);
          record('request.send', { tag: input.tag, requestId: input.requestId });
          ws.send(JSON.stringify({ tag: input.tag, requestId: input.requestId, input: input.input }));
        },
        readyState() {
          return ws ? ws.readyState : -1;
        },
        close() {
          if (ws && ws.readyState === WebSocket.OPEN) {
            ws.close();
          }
        }
      };
    </script>
  </body>
</html>`;
}

function attachObservers(observedPage) {
  observedPage.on('console', (message) => {
    const entry = {
      type: message.type(),
      text: message.text(),
    };
    if (entry.type === 'error' || entry.type === 'warning') {
      consoleFindings.push(entry);
    }
  });

  observedPage.on('pageerror', (error) => {
    consoleFindings.push({ type: 'pageerror', text: error.message });
  });

  observedPage.on('requestfailed', (request) => {
    networkFindings.push({
      type: 'requestfailed',
      method: request.method(),
      url: request.url(),
      failure: request.failure()?.errorText ?? 'unknown',
    });
  });

  observedPage.on('response', (response) => {
    if (response.status() >= 400) {
      networkFindings.push({
        type: 'http',
        status: response.status(),
        url: response.url(),
      });
    }
  });

  observedPage.on('websocket', (socket) => {
    socket.on('framesent', (event) => {
      wsFrames.push({ direction: 'sent', url: socket.url(), payload: String(event.payload) });
    });
    socket.on('framereceived', (event) => {
      wsFrames.push({ direction: 'received', url: socket.url(), payload: String(event.payload) });
    });
    socket.on('close', () => {
      wsFrames.push({ direction: 'close', url: socket.url(), payload: '' });
    });
  });
}

async function readSmokeStorage(targetPage) {
  return targetPage.evaluate(() => {
    const output = {};
    for (const key of Object.keys(localStorage)) {
      if (key.startsWith('skiff:websocket-fixture-smoke:')) {
        output[key] = JSON.parse(localStorage.getItem(key));
      }
    }
    return {
      run: output['skiff:websocket-fixture-smoke:run'],
      sent: output['skiff:websocket-fixture-smoke:sent'] ?? [],
    };
  });
}

function validateLocalStorage(storage, runId) {
  assert(storage.run?.runId === runId, `localStorage run id is stale: ${JSON.stringify(storage.run)}`);
  assert(storage.sent.length === requestIds.length, 'localStorage sent length does not match sent requests');

  const seen = new Set();
  for (const entry of storage.sent) {
    assert(entry.runId === runId, `localStorage sent entry contains stale run id: ${JSON.stringify(entry)}`);
    assert(!seen.has(entry.requestId), `localStorage sent entry contains duplicate requestId: ${entry.requestId}`);
    seen.add(entry.requestId);
  }
}

function validateObservers() {
  assert(consoleFindings.length === 0, `console findings: ${JSON.stringify(consoleFindings, null, 2)}`);
  assert(networkFindings.length === 0, `network findings: ${JSON.stringify(networkFindings, null, 2)}`);

  const sentFrames = wsFrames.filter((frame) => frame.direction === 'sent');
  assert(sentFrames.length === requestIds.length, `expected ${requestIds.length} sent websocket frames, got ${sentFrames.length}`);

  const sentRequestIds = new Set();
  for (const frame of sentFrames) {
    const payload = parseFramePayload(frame.payload);
    if (payload?.requestId) {
      sentRequestIds.add(payload.requestId);
    }
  }
  for (const requestId of requestIds) {
    assert(sentRequestIds.has(requestId), `missing websocket sent frame for ${requestId}`);
  }
}

function parseFramePayload(payload) {
  try {
    return JSON.parse(payload);
  } catch {
    return undefined;
  }
}

async function saveFailureArtifacts(error) {
  await mkdir(screenshotDir, { recursive: true });
  await page.screenshot({ path: path.join(screenshotDir, 'websocket-fixture-smoke-failure.png'), fullPage: true });
  await writeFile(
    path.join(screenshotDir, 'websocket-fixture-smoke-report.json'),
    JSON.stringify(
      {
        error: error instanceof Error ? error.stack : String(error),
        consoleFindings,
        networkFindings,
        wsFrames,
        requestIds,
      },
      null,
      2
    )
  );
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}
