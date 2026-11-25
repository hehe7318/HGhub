import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

const source = readFileSync(new URL("../src/api-error.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2021 },
});
const { apiErrorMessage } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`
);

const offline = 'HTTP 410 Gone: {"data":[],"status":0,"msg":"内容已下线"}';
for (const error of [offline, new Error(offline), { message: offline }]) {
  assert.equal(apiErrorMessage(error), "该内容已下线，重试无法恢复");
}
assert.match(
  apiErrorMessage(new Error('HTTP 503 Service Unavailable: {"msg":"接口已停用"}')),
  /接口已停用.*重试无法恢复/,
);
for (const error of [
  "HTTP 404 Not Found: <html>private upstream error</html>",
  new Error("HTTP 404 Not Found: <html>private upstream error</html>"),
]) {
  assert.match(apiErrorMessage(error), /^服务器请求失败（HTTP 404）/);
  assert(!apiErrorMessage(error).includes("private upstream"));
}
assert.equal(apiErrorMessage("401"), "登录已失效，请重新登录");
assert.match(apiErrorMessage(new Error("HTTP 429 Too Many Requests")), /请求过于频繁/);
assert.match(apiErrorMessage(new Error("Failed to fetch")), /网络连接失败/);
assert.equal(apiErrorMessage(null), "请求失败，请检查网络");
const redacted = apiErrorMessage(new Error("连接失败 https://example.invalid/api?token=fixture-secret"));
assert(!redacted.includes("example.invalid") && !redacted.includes("fixture-secret"));
assert(apiErrorMessage("x".repeat(300)).length <= 141);
console.log("PASS API errors: native strings, web Errors, offline/disabled states, HTTP codes and URL redaction");
