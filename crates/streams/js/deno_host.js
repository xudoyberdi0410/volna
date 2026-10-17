// Deno host for `volna_streams::innertube::DenoRuntime` (desktop dev and tests only).
// SPDX-License-Identifier: Apache-2.0
//
// stdin:  one JSON object per line, {"id": <u64>, "request": <JsRequest>}
// stdout: one JSON object per line, {"id": <u64>, "response": "<volnaJs result string>"}
//
// BotGuard fingerprints the DOM, so a jsdom window stands in for the WebView's
// document (as in bgutil-ytdlp-pot-provider's TypeScript server). Rust prepends
// `globalThis.__VOLNA_PRELUDE__ = "<prelude source>";` to this file (imports
// are hoisted, the assignment still runs before the code below).
import { JSDOM } from "npm:jsdom@26.1.0";

const dom = new JSDOM(
  '<!DOCTYPE html><html lang="en"><head><title></title></head><body></body></html>',
  { url: "https://music.youtube.com/", referrer: "https://music.youtube.com/", pretendToBeVisual: true },
);
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  location: dom.window.location,
  origin: dom.window.origin,
});
if (!Reflect.has(globalThis, "navigator")) {
  Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator });
}

// stdout carries the protocol; anything the scripts print goes to stderr.
console.log = console.error;
console.info = console.error;
console.debug = console.error;

// Classic-script semantics, as `evaluateJavaScript` in a WebView: top-level
// `var`s of the prelude become globals.
(0, eval)(globalThis.__VOLNA_PRELUDE__);
delete globalThis.__VOLNA_PRELUDE__;

const encoder = new TextEncoder();
const writer = Deno.stdout.writable.getWriter();
let writes = Promise.resolve();
function reply(line) {
  writes = writes.then(() => writer.write(encoder.encode(line + "\n")));
  return writes;
}

async function handle(line) {
  let id = null;
  try {
    const msg = JSON.parse(line);
    id = msg.id;
    const response = await globalThis.volnaJs(msg.request);
    await reply(JSON.stringify({ id, response }));
  } catch (e) {
    const error = { code: "host", message: String(e && e.message ? e.message : e) };
    await reply(JSON.stringify({ id, response: JSON.stringify({ error }) }));
  }
}

const decoder = new TextDecoder();
let buffered = "";
for await (const chunk of Deno.stdin.readable) {
  buffered += decoder.decode(chunk, { stream: true });
  let nl;
  while ((nl = buffered.indexOf("\n")) >= 0) {
    const line = buffered.slice(0, nl);
    buffered = buffered.slice(nl + 1);
    if (line.trim()) handle(line);
  }
}
await writes;
