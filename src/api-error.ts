/** Native IPC rejects strings; the web adapter rejects Error objects. */
export function apiErrorMessage(error: unknown): string {
  const raw = typeof error === "string"
    ? error
    : error instanceof Error
      ? error.message
      : error && typeof error === "object" && "message" in error
        && typeof error.message === "string"
        ? error.message
        : "";
  if (!raw.trim()) return "请求失败，请检查网络";

  // Show actionable states, not upstream HTML, response bodies or signed URLs.
  if (raw.includes("接口已停用")) {
    return "上游接口已停用，需要更新适配；重试无法恢复";
  }
  const status = /^HTTP\s+(\d{3})\b/.exec(raw)?.[1];
  if (raw.includes("内容已下线") || status === "410") {
    return "该内容已下线，重试无法恢复";
  }
  if (raw === "401" || status === "401" || raw.includes("登录已失效")) {
    return "登录已失效，请重新登录";
  }
  if (status === "429") return "请求过于频繁，请稍后再试（HTTP 429）";
  if (status) {
    return `服务器请求失败（HTTP ${status}），请稍后重试或在设置中切换线路`;
  }
  if (/Failed to fetch|NetworkError|Load failed/i.test(raw)) {
    return "网络连接失败，请稍后重试";
  }
  const safe = raw.replace(/<[^>]*>/g, "")
    .replace(/https?:\/\/[^\s"'<>]+/g, "（地址已隐藏）").trim();
  return safe ? safe.length > 140 ? `${safe.slice(0, 140)}…` : safe
    : "请求失败，请检查网络";
}
