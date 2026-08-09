// Keyboard-reachability QA over CDP. Tabs through the page and records where focus actually lands.
//
// Why real Tab presses rather than reading the DOM: a tabbable list computed from selectors is a
// model of focus order, and the thing being checked is focus order. `Input.dispatchKeyEvent` moves
// the browser's own focus, so what this records is what a keyboard user gets.

const CDP = "http://127.0.0.1:9222";
const [, , url, widthArg, steps] = process.argv;
const width = Number(widthArg || 1600);
const N = Number(steps || 40);

const list = await (await fetch(`${CDP}/json/list`)).json();
const page = list.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pend = new Map();
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id !== undefined) {
    const p = pend.get(m.id);
    pend.delete(m.id);
    m.error ? p.reject(new Error(JSON.stringify(m.error))) : p.resolve(m.result);
  }
});
const send = (method, params = {}) => {
  const i = ++id;
  ws.send(JSON.stringify({ id: i, method, params }));
  return new Promise((res, rej) => pend.set(i, { resolve: res, reject: rej }));
};

await send("Page.enable");
await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", {
  width, height: 1000, deviceScaleFactor: 1, mobile: false,
});
await send("Page.navigate", { url });
await new Promise((r) => setTimeout(r, 2500));

const evalJs = async (expr) =>
  JSON.parse((await send("Runtime.evaluate", { expression: expr, returnByValue: true })).result.value);

// Anti-patterns that are decidable from the DOM, checked once.
const dom = await evalJs(`JSON.stringify({
  positiveTabindex: [...document.querySelectorAll('[tabindex]')]
      .filter(el => Number(el.getAttribute('tabindex')) > 0)
      .map(el => el.tagName.toLowerCase() + '.' + (el.className||'')).slice(0,10),
  interactive: document.querySelectorAll('a[href],button,input,select,textarea,[tabindex]:not([tabindex="-1"])').length,
  imgNoAlt: [...document.querySelectorAll('img')].filter(i => !i.hasAttribute('alt')).length,
  buttonsNoName: [...document.querySelectorAll('button')]
      .filter(b => !(b.innerText||'').trim() && !b.getAttribute('aria-label') && !b.getAttribute('title')).length
})`);

const seen = [];
for (let i = 0; i < N; i++) {
  for (const type of ["keyDown", "keyUp"]) {
    await send("Input.dispatchKeyEvent", {
      type, key: "Tab", code: "Tab", windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9,
    });
  }
  const at = await evalJs(`(() => {
    const a = document.activeElement;
    if (!a || a === document.body) return JSON.stringify({tag:'(body)'});
    const cs = getComputedStyle(a);
    const r = a.getBoundingClientRect();
    return JSON.stringify({
      tag: a.tagName.toLowerCase(),
      cls: (a.className||'').toString().slice(0,30),
      text: (a.innerText||a.getAttribute('placeholder')||a.getAttribute('aria-label')||'').slice(0,34).replace(/\\n/g,' '),
      outline: cs.outlineStyle + ' ' + cs.outlineWidth,
      boxShadow: cs.boxShadow === 'none' ? 'none' : 'set',
      offscreen: r.width === 0 && r.height === 0
    });
  })()`);
  seen.push(at);
}

const focusables = seen.filter((s) => s.tag !== "(body)");
const noVisibleRing = focusables.filter(
  (s) => (s.outline === "none 0px" || s.outline.startsWith("none")) && s.boxShadow === "none"
);
const offscreen = focusables.filter((s) => s.offscreen);

console.log(JSON.stringify({
  width,
  interactiveInDom: dom.interactive,
  positiveTabindex: dom.positiveTabindex,
  imagesWithoutAlt: dom.imgNoAlt,
  buttonsWithoutAccessibleName: dom.buttonsNoName,
  tabStopsReached: focusables.length,
  distinctStops: [...new Set(focusables.map((s) => s.tag + "|" + s.text))].length,
  focusStopsWithNoVisibleRing: noVisibleRing.length,
  noRingDetail: [...new Set(noVisibleRing.map(s => `${s.tag}.${s.cls}|${s.text}|outline=${s.outline}`))],
  focusStopsOffscreen: offscreen.length,
  order: focusables.slice(0, 14).map((s) => `${s.tag}:${s.text || s.cls}`),
}, null, 2));
ws.close();
