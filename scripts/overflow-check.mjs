// Does a page scroll sideways on a phone? Emulates a 390 px mobile viewport in
// headless Chrome over the DevTools protocol and lists every element wider than it.
//
//   node scripts/overflow-check.mjs <url> [width] [screenshot.png]
//
// Screenshots taken with plain `chrome --headless --screenshot --window-size` are not
// evidence here: on a scaled Windows display the CSS viewport is not the window size,
// and the nav looks cut off on pages that are fine. This asks the page directly. Note
// that the site's stylesheet is linked absolutely, so a local build only picks up
// local CSS when built with `run_zola build --base-url http://127.0.0.1:<port>`.
// Code lines inside <pre> always show up as wide; they scroll in their own box.
//
// CHROME overrides the browser path.
const [,, url, widthArg] = process.argv;
const width = Number(widthArg || 390);
const port = 9333;
const { spawn } = await import('node:child_process');
const chrome = spawn(process.env.CHROME || 'C:/Program Files/Google/Chrome/Application/chrome.exe',
  ['--headless=new', '--disable-gpu', `--remote-debugging-port=${port}`, '--user-data-dir=' + process.env.TEMP + '/cdp-profile', 'about:blank'],
  { stdio: 'ignore' });
const wait = (ms) => new Promise(r => setTimeout(r, ms));
let target;
for (let i = 0; i < 40; i++) {
  try { const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); target = list.find(t => t.type === 'page'); if (target) break; } catch {}
  await wait(250);
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
let id = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise(res => { const i = ++id; pending.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
await send('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 2, mobile: true });
await send('Page.enable');
await send('Page.navigate', { url });
await wait(2500);
const expr = `(() => {
  const vw = document.documentElement.clientWidth;
  const out = { vw, scrollWidth: document.documentElement.scrollWidth, bodyScroll: document.body.scrollWidth, wide: [] };
  for (const el of document.querySelectorAll('body *')) {
    const r = el.getBoundingClientRect();
    if (r.right > vw + 1 || r.left < -1) {
      const cs = getComputedStyle(el);
      out.wide.push({ tag: el.tagName.toLowerCase(), cls: el.className && el.className.baseVal === undefined ? String(el.className).slice(0, 60) : '', left: Math.round(r.left), right: Math.round(r.right), w: Math.round(r.width), pos: cs.position, disp: cs.display });
    }
    if (out.wide.length > 25) break;
  }
  return JSON.stringify(out);
})()`;
const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true });
console.log(r.result.result.value);
const pre = await send('Runtime.evaluate', { expression: "(()=>{const p=document.querySelector('pre'); return p? getComputedStyle(p).overflowX : 'no pre'})()", returnByValue: true });
console.error('pre overflow-x:', pre.result.result.value);
if (process.argv[4]) { const shot = await send('Page.captureScreenshot', { format: 'png' }); (await import('node:fs')).writeFileSync(process.argv[4], Buffer.from(shot.result.data, 'base64')); }
ws.close(); chrome.kill();
