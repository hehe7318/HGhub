import {
  addPluginListener as tauriAddPluginListener,
  invoke as tauriInvoke,
} from "@tauri-apps/api/core";

const hasTauriRuntime = () => "__TAURI_INTERNALS__" in window;

export const isWebRuntime = !hasTauriRuntime();

export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (hasTauriRuntime()) return tauriInvoke<T>(command, args);

  if (command === "open_url") {
    const url = typeof args?.url === "string" ? args.url : "";
    if (url) window.open(url, "_blank", "noopener,noreferrer");
    return undefined as T;
  }
  if (command === "set_orientation" || command === "plugin:app|exit") {
    return undefined as T;
  }

  const response = await fetch(`/api/invoke/${encodeURIComponent(command)}`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });
  if (response.status === 401) {
    window.location.assign("/login");
    throw new Error("登录已失效");
  }
  const payload = await response.json().catch(() => null) as
    | { ok: true; data: T }
    | { ok: false; error: string }
    | null;
  if (!response.ok || !payload || !payload.ok) {
    throw new Error(payload && "error" in payload ? payload.error : `HTTP ${response.status}`);
  }
  return payload.data;
}

export async function addPluginListener<T>(
  plugin: string,
  event: string,
  handler: (payload: T) => void,
) {
  if (hasTauriRuntime()) return tauriAddPluginListener(plugin, event, handler);
  return { unregister: async () => undefined };
}
