// js/player-bridge.js
// The only place that talks to the native player (docs/native-player.md §3).
//
// Every call goes through one FIFO promise chain: the Rust engine applies
// commands in the order it receives them, and Tauri runs async commands
// concurrently, so the UI waits for each reply before sending the next one.
// Replies come as soon as the state is updated, so the chain never waits on
// the network.
//
// `createBridge()` returns the real bridge inside Tauri, or the in-memory
// MockBridge (js/player-mock.js) in a plain browser (`bun run dev`, vitest)
// or when localStorage['volna-mock-player'] === '1'.

/** Every event the player side sends (§3.4). */
export const PLAYER_EVENTS = Object.freeze([
    'player:state',
    'player:position',
    'player:queue',
    'player:error',
    'player:journal',
    'player:queue-low',
    'player:settings',
    'player:sleep',
    'player:wave',
    'app:will-close',
]);

/** Returned by a lazy argument function to drop the call without sending it. */
export const SKIP = Symbol('skip');

/** Splits `"<code>: <message>"` (§3.1) into its parts. */
export function parseError(error) {
    const text = String(error?.message ?? error ?? '');
    const m = /^([a-z_]+):\s*(.*)$/s.exec(text);
    return m ? { code: m[1], message: m[2] } : { code: 'internal', message: text };
}

/**
 * The FIFO part shared by both bridges. Subclasses implement `_invoke`.
 * `call(cmd, args)` accepts either an args object or a function (sync or
 * async) that is evaluated right before the command is sent, so it can read
 * state that earlier calls in the chain produced. Returning SKIP drops it.
 */
export class SerialBridge {
    constructor() {
        this._tail = Promise.resolve();
    }

    call(cmd, args) {
        const run = async () => {
            const resolved = typeof args === 'function' ? await args() : args;
            if (resolved === SKIP) return null;
            return this._invoke(cmd, resolved ?? {});
        };
        const p = this._tail.then(run);
        this._tail = p.then(
            () => undefined,
            () => undefined
        );
        return p;
    }

    /** Outside the chain. Only for reads that must not wait (or deadlock) behind it. */
    direct(cmd, args = {}) {
        return this._invoke(cmd, args);
    }

    /** Resolves once everything queued so far has been answered. */
    idle() {
        return this._tail;
    }
}

export class TauriBridge extends SerialBridge {
    constructor({ invoke, listen, Channel }) {
        super();
        this.kind = 'tauri';
        this._invokeFn = invoke;
        this._listen = listen;
        this._Channel = Channel;
    }

    _invoke(cmd, args) {
        return this._invokeFn(cmd, args);
    }

    /** Registers a listener; resolves to the unlisten function. */
    on(event, handler) {
        return this._listen(event, (e) => handler(e.payload));
    }

    /** A binary channel for `player_visualizer_start` (§3.5). */
    channel(onmessage) {
        return new this._Channel(onmessage);
    }
}

function mockForced() {
    try {
        return globalThis.localStorage?.getItem('volna-mock-player') === '1';
    } catch {
        return false;
    }
}

export async function createBridge(win = globalThis.window) {
    if (!win?.__TAURI_INTERNALS__ || mockForced()) {
        const { MockBridge } = await import('./player-mock.js');
        return new MockBridge();
    }
    const [{ invoke, Channel }, { listen }] = await Promise.all([
        import('@tauri-apps/api/core'),
        import('@tauri-apps/api/event'),
    ]);
    return new TauriBridge({ invoke, listen, Channel });
}
