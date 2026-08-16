/**
 * Environment-adaptive transport.
 *
 * The app runs in two modes sharing one frontend bundle:
 *  1. Tauri desktop — `window.__TAURI_INTERNALS__` exists; use
 *     `@tauri-apps/api/core` invoke (original behavior).
 *  2. Pure Web — plain browser hitting the axum backend; call
 *     `POST /api/{command}` with a JSON body and subscribe to events over
 *     `WebSocket /ws`.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Call a backend command. Web mode maps Tauri's camelCase argument names to
 * the snake_case keys the Rust dispatch expects.
 */
export async function callCommand<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (isTauri()) {
    return invoke<T>(command, args);
  }
  // Web mode: camelCase → snake_case for the HTTP bridge.
  const body: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(args)) {
    body[snakeCase(key)] = value;
  }
  const resp = await fetch(`/api/${command}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = (await resp.json()) as { ok: boolean; data?: T; error?: string };
  if (!json.ok) {
    throw new Error(json.error ?? `command ${command} failed`);
  }
  return json.data as T;
}

/** camelCase → snake_case (e.g. `viaProxy` → `via_proxy`). */
function snakeCase(s: string): string {
  return s.replace(/[A-Z]/g, (m) => `_${m.toLowerCase()}`);
}

type EventCallback<T> = (payload: T) => void;

/**
 * Subscribe to backend events. Tauri mode uses the event system; Web mode
 * connects to the WebSocket bridge and dispatches by event name.
 */
export function onEvent<T>(event: string, callback: EventCallback<T>): () => void {
  if (isTauri()) {
    const unlistenPromise = listen<T>(event, (e) => callback(e.payload));
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }
  return subscribeWs(event, (payload) => callback(payload as T));
}

// ---------- WebSocket (Web mode only) ----------

const wsSubscribers = new Map<string, Set<EventCallback<unknown>>>();
let ws: WebSocket | null = null;
let wsReconnectTimer: ReturnType<typeof setTimeout> | null = null;
let wsConnected = false;

function wsUrl(): string {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${window.location.host}/ws`;
}

function ensureWs(): void {
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
    return;
  }
  ws = new WebSocket(wsUrl());
  ws.onopen = () => {
    wsConnected = true;
  };
  ws.onmessage = (ev) => {
    try {
      const frame = JSON.parse(ev.data as string) as { event: string; payload: unknown };
      const set = wsSubscribers.get(frame.event);
      if (set) {
        for (const cb of set) {
          try {
            cb(frame.payload);
          } catch {
            // subscriber error must not break the socket loop
          }
        }
      }
    } catch {
      // malformed frame — ignore
    }
  };
  ws.onclose = () => {
    wsConnected = false;
    if (wsSubscribers.size > 0 && wsReconnectTimer === null) {
      wsReconnectTimer = setTimeout(() => {
        wsReconnectTimer = null;
        ensureWs();
      }, 2000);
    }
  };
  ws.onerror = () => {
    ws?.close();
  };
}

function subscribeWs(event: string, callback: EventCallback<unknown>): () => void {
  ensureWs();
  let set = wsSubscribers.get(event);
  if (!set) {
    set = new Set();
    wsSubscribers.set(event, set);
  }
  set.add(callback);
  return () => {
    set!.delete(callback);
    if (set!.size === 0) {
      wsSubscribers.delete(event);
    }
  };
}

/** For diagnostics: whether the WebSocket is currently connected. */
export function wsIsConnected(): boolean {
  return wsConnected;
}
