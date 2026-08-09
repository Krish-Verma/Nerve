// Viewport-faithful browser QA over the Chrome DevTools Protocol.
//
// Why not `--window-size`: Chrome clamps a headless window to a platform minimum (measured at
// 500 CSS px on this machine), so `--window-size=380,700` renders the page at 500px and merely
// crops the screenshot to 380. That is the same failure the Chrome extension's `resize_window`
// has, and row 7a-ii already recorded a QA claim built on an unverified resize. This driver
// therefore uses `Emulation.setDeviceMetricsOverride` — what Playwright uses underneath — and
// *asserts* the resulting `window.innerWidth` rather than assuming it.
//
// No new dependency: Node 22+ ships a global WebSocket, and Chrome is already installed.

import { writeFileSync } from "node:fs";

const CDP = "http://127.0.0.1:9222";

async function targets() {
  const res = await fetch(`${CDP}/json/list`);
  return res.json();
}

class Session {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.pending = new Map();
    this.events = [];
    ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id !== undefined) {
        const p = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
        else p.resolve(msg.result);
      } else {
        this.events.push(msg);
      }
    });
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
}

async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });
  return new Session(ws);
}

const [, , url, widthArg, outPath, label] = process.argv;
const width = Number(widthArg);
const height = Number(process.env.QA_HEIGHT || 900);

const list = await targets();
const page = list.find((t) => t.type === "page");
if (!page) throw new Error("no page target; is Chrome running with --remote-debugging-port=9222?");

const s = await connect(page.webSocketDebuggerUrl);
await s.send("Page.enable");
await s.send("Runtime.enable");
await s.send("Log.enable");
await s.send("Network.enable");

// The whole point: a real layout viewport, not a cropped screenshot.
await s.send("Emulation.setDeviceMetricsOverride", {
  width,
  height,
  deviceScaleFactor: 1,
  // `mobile: false` on purpose. With mobile emulation on, Chrome gives a page that carries no
  // `<meta name="viewport">` the legacy 980px layout viewport — correct emulation, but it measures
  // the meta tag rather than the layout. A narrow *desktop* window is the case row 7a-ii left open,
  // and it is the one where `innerWidth` is exactly what was asked for.
  mobile: false,
});

await s.send("Page.navigate", { url });
await new Promise((r) => setTimeout(r, 2500));

const probe = await s.send("Runtime.evaluate", {
  expression: `JSON.stringify({
    innerWidth: window.innerWidth,
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
    bodyScrollWidth: document.body ? document.body.scrollWidth : -1,
    title: document.title,
    text: document.body ? document.body.innerText.slice(0, 30000) : ""
  })`,
  returnByValue: true,
});
const info = JSON.parse(probe.result.value);

// Horizontal overflow: the page body must never scroll sideways (brief §6).
info.horizontalOverflow = info.scrollWidth > info.clientWidth;
info.viewportHonoured = info.innerWidth === width;

const console_ = s.events
  .filter((e) => e.method === "Runtime.consoleAPICalled" || e.method === "Log.entryAdded")
  .map((e) =>
    e.method === "Log.entryAdded"
      ? { source: e.params.entry.source, level: e.params.entry.level, text: e.params.entry.text }
      : {
          source: "console",
          level: e.params.type,
          text: (e.params.args || []).map((a) => a.value ?? a.description ?? "").join(" "),
        }
  );
info.requests = s.events
  .filter((e) => e.method === "Network.responseReceived")
  .map((e) => ({ status: e.params.response.status, url: e.params.response.url.slice(0, 120) }))
  .filter((r) => r.status >= 400);

const exceptions = s.events
  .filter((e) => e.method === "Runtime.exceptionThrown")
  .map((e) => e.params.exceptionDetails.text);

if (outPath) {
  const shot = await s.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
  writeFileSync(outPath, Buffer.from(shot.data, "base64"));
}

console.log(
  JSON.stringify(
    { label: label || url, width, ...info, console: console_, exceptions },
    null,
    2
  )
);
s.ws.close();
