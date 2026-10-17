/*!
 * Volna JS prelude: the request dispatcher behind `volna_streams::innertube::JsRuntime`.
 * Loaded after the yt-dlp EJS solver (`ejs/`, Unlicense), which defines `jsc`.
 *
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * The BotGuard part follows BgUtils by LuanRT (MIT, see LICENSE-bgutils):
 * https://github.com/LuanRT/BgUtils
 *
 * Contract: `volnaJs(request)` takes the request (object or JSON string) and
 * resolves to a JSON string, `{"ok": ...}` or `{"error": {"code", "message"}}`.
 * It never rejects. It makes no network requests: Rust does all HTTP.
 */
(function () {
  "use strict";

  const VERSION = 1;
  const MAX_PLAYERS = 2;
  const MAX_SESSIONS = 2;
  const VM_TIMEOUT_MS = 15000;

  /** player id -> preprocessed player from the EJS solver. */
  const players = new Map();
  /** BotGuard session id -> { webPoSignalOutput, minter, integrityToken }. */
  const sessions = new Map();

  class VolnaError extends Error {
    constructor(code, message) {
      super(message);
      this.code = code;
    }
  }

  function remember(map, key, value, max) {
    map.delete(key);
    while (map.size >= max) map.delete(map.keys().next().value);
    map.set(key, value);
  }

  function withTimeout(promise, what) {
    let timer;
    return Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new VolnaError("botguard", `${what} timed out`)), VM_TIMEOUT_MS);
      }),
    ]).finally(() => clearTimeout(timer));
  }

  function base64ToU8(text) {
    const std = text.replace(/[-_.]/g, (c) => ({ "-": "+", "_": "/", ".": "=" })[c]);
    const bin = atob(std);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function u8ToBase64Url(bytes) {
    let bin = "";
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_");
  }

  // n / signature challenges through the EJS solver (`jsc`).
  function solve(req) {
    if (typeof jsc !== "function") throw new VolnaError("no_solver", "EJS solver is not loaded");
    const requests = [];
    if (req.n && req.n.length) requests.push({ type: "n", challenges: req.n });
    if (req.sig && req.sig.length) requests.push({ type: "sig", challenges: req.sig });
    const result = { n: {}, sig: {} };
    if (!requests.length) return result;

    let input;
    const preprocessed = players.get(req.player_id);
    if (typeof req.player === "string") {
      input = { type: "player", player: req.player, requests, output_preprocessed: true };
    } else if (preprocessed) {
      input = { type: "preprocessed", preprocessed_player: preprocessed, requests };
    } else {
      throw new VolnaError("need_player", `player ${req.player_id} is not loaded`);
    }

    const out = jsc(input);
    if (out.type === "error") throw new VolnaError("solver", String(out.error));
    if (out.preprocessed_player) remember(players, req.player_id, out.preprocessed_player, MAX_PLAYERS);
    requests.forEach((r, i) => {
      const resp = out.responses[i];
      if (!resp || resp.type === "error") {
        throw new VolnaError("solver", `${r.type}: ${resp ? resp.error : "no response"}`);
      }
      Object.assign(result[r.type], resp.data);
    });
    return result;
  }

  // Loads a BotGuard VM from a challenge and takes the snapshot whose
  // response Rust exchanges for an integrity token.
  async function botguardSnapshot(req) {
    new Function(req.interpreter_js)();
    const vm = globalThis[req.global_name];
    if (!vm || typeof vm.a !== "function") throw new VolnaError("botguard", "BotGuard VM unavailable");

    let setup;
    const functions = new Promise((resolve) => (setup = resolve));
    const vmSetup = (asyncSnapshot, shutdown, passEvent, checkCamera) =>
      setup({ asyncSnapshot, shutdown, passEvent, checkCamera });
    try {
      await vm.a(req.program, vmSetup, true, undefined, () => {}, [[], []]);
    } catch (e) {
      throw new VolnaError("botguard", `could not load program: ${e && e.message ? e.message : e}`);
    }
    const fns = await withTimeout(functions, "VM setup");
    const webPoSignalOutput = [];
    const response = await withTimeout(
      new Promise((resolve) => fns.asyncSnapshot((r) => resolve(r), [undefined, undefined, webPoSignalOutput, undefined])),
      "snapshot",
    );
    if (typeof response !== "string" || !response) throw new VolnaError("botguard", "empty snapshot");
    remember(sessions, req.session, { webPoSignalOutput, minter: null, integrityToken: null }, MAX_SESSIONS);
    return { botguard_response: response };
  }

  // Mints PO tokens with the integrity token Rust got for this session.
  async function botguardMint(req) {
    const s = sessions.get(req.session);
    if (!s) throw new VolnaError("unknown_session", `BotGuard session ${req.session} is gone`);
    if (!s.minter || s.integrityToken !== req.integrity_token) {
      const getMinter = s.webPoSignalOutput[0];
      if (typeof getMinter !== "function") throw new VolnaError("botguard", "PMD:Undefined (no minter factory)");
      const minter = await getMinter(base64ToU8(req.integrity_token));
      if (typeof minter !== "function") throw new VolnaError("botguard", "APF:Failed (no minter)");
      s.minter = minter;
      s.integrityToken = req.integrity_token;
    }
    const tokens = [];
    for (const binding of req.content_bindings) {
      const bytes = await s.minter(new TextEncoder().encode(binding));
      if (!bytes || typeof bytes.length !== "number") throw new VolnaError("botguard", "ODM:Invalid (mint failed)");
      tokens.push(u8ToBase64Url(bytes));
    }
    return { po_tokens: tokens };
  }

  async function dispatch(req) {
    switch (req.op) {
      case "solve":
        return solve(req);
      case "botguard_snapshot":
        return await botguardSnapshot(req);
      case "botguard_mint":
        return await botguardMint(req);
      case "version":
        return { version: VERSION };
      default:
        throw new VolnaError("bad_request", `unknown op ${req.op}`);
    }
  }

  globalThis.volnaJs = async function (request) {
    try {
      const req = typeof request === "string" ? JSON.parse(request) : request;
      return JSON.stringify({ ok: await dispatch(req) });
    } catch (e) {
      const code = e && typeof e.code === "string" ? e.code : "js_error";
      const message = e && e.message ? e.message : String(e);
      return JSON.stringify({ error: { code, message } });
    }
  };
})();
