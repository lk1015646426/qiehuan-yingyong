// WorkBuddy 只读诊断工具（合并版，替代原 tokens/bak/probe/credits 四个脚本）。
//
// 用法:
//   node scripts/diagnose-workbuddy.mjs           本地诊断（无网络）：客户端认证文件 + 全部账号记录比对
//   node scripts/diagnose-workbuddy.mjs probe      网络探测：逐账号验证 access token 服务端有效性
//   node scripts/diagnose-workbuddy.mjs credits    网络查询：当前客户端账号积分原始包明细 + 求和复现
//   node scripts/diagnose-workbuddy.mjs all        以上全部
//
// 只读不写：不修改任何账号文件或认证文件；probe/credits 与 app 日常状态查询使用同一接口，无副作用。
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import crypto from "node:crypto";

const DATA_DIR = path.join(os.homedir(), ".qiehuan_yingyong");
const ACCOUNTS_DIR = path.join(DATA_DIR, "workbuddy_accounts");
const KEY_FILE = path.join(DATA_DIR, "secure-account-storage.key");
const AUTH_FILE = path.join(
  process.env.LOCALAPPDATA || "",
  "CodeBuddyExtension",
  "Data",
  "Public",
  "auth",
  "workbuddy-desktop.info",
);
const RESOURCE_URL = "https://copilot.tencent.com/v2/billing/meter/get-user-resource";

const mode = process.argv[2] ?? "local";
if (!["local", "probe", "credits", "all"].includes(mode)) {
  console.error("用法: node scripts/diagnose-workbuddy.mjs [local|probe|credits|all]");
  process.exit(1);
}

const fp = (value) =>
  crypto.createHash("sha256").update(String(value ?? "")).digest("hex").slice(0, 12);

function decryptAccount(file) {
  const envelope = JSON.parse(fs.readFileSync(file, "utf8"));
  const key = Buffer.from(fs.readFileSync(KEY_FILE, "utf8").trim(), "base64");
  const nonce = Buffer.from(envelope.nonce, "base64");
  const ciphertext = Buffer.from(envelope.ciphertext, "base64");
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, nonce);
  decipher.setAuthTag(ciphertext.subarray(ciphertext.length - 16));
  return JSON.parse(
    Buffer.concat([
      decipher.update(ciphertext.subarray(0, ciphertext.length - 16)),
      decipher.final(),
    ]).toString("utf8"),
  );
}

function tokensOf(snapshot) {
  const auth = snapshot?.auth ?? {};
  return {
    access: auth.accessToken ?? auth.access_token ?? null,
    refresh: auth.refreshToken ?? auth.refresh_token ?? null,
    expiresAt: auth.expiresAt ?? auth.expires_at ?? null,
  };
}

function loadAccounts() {
  return fs
    .readdirSync(ACCOUNTS_DIR)
    .filter((name) => name.startsWith("wb-") && name.endsWith(".json"))
    .map((name) => {
      try {
        const record = decryptAccount(path.join(ACCOUNTS_DIR, name));
        return { record, tokens: tokensOf(record.snapshot), error: null };
      } catch (error) {
        return { record: { id: name, display_name: "?" }, tokens: {}, error: error.message };
      }
    });
}

function loadClientAuth() {
  if (!fs.existsSync(AUTH_FILE)) return null;
  const parsed = JSON.parse(fs.readFileSync(AUTH_FILE, "utf8"));
  return {
    uid: parsed.account?.uid ?? parsed.uid ?? "(未知)",
    ...tokensOf(parsed),
  };
}

// ---------- 本地诊断 ----------
function runLocal(clientAuth, accounts) {
  console.log("=== WorkBuddy 客户端认证文件 ===");
  if (!clientAuth) {
    console.log(`未找到: ${AUTH_FILE}（客户端未登录？）`);
  } else {
    console.log(`账号 uid: ${clientAuth.uid}`);
    console.log(`access_fp=${fp(clientAuth.access)} refresh_fp=${fp(clientAuth.refresh)} expiresAt=${clientAuth.expiresAt}`);
  }
  console.log("\n=== 本地账号记录（解密） ===");
  const now = Date.now();
  for (const { record, tokens, error } of accounts) {
    if (error) {
      console.log(`${record.id}: 解密失败 ${error}`);
      continue;
    }
    const uid = record.snapshot?.account?.uid ?? record.uid ?? "(未知)";
    const daysLeft = tokens.expiresAt
      ? ((tokens.expiresAt / 1000 - now / 1000) / 86400).toFixed(1)
      : "?";
    const marks = [
      clientAuth && uid === clientAuth.uid ? "<== 当前客户端账号" : "",
      clientAuth && tokens.access === clientAuth.access ? "[access与客户端一致]" : "",
      clientAuth && tokens.refresh === clientAuth.refresh ? "[refresh与客户端一致]" : "",
    ].filter(Boolean).join(" ");
    console.log(
      `${record.display_name ?? record.id} (${record.id}) uid=${String(uid).slice(0, 8)}… token剩${daysLeft}天 access_fp=${fp(tokens.access)} refresh_fp=${fp(tokens.refresh)} ${marks}`,
    );
  }
}

// ---------- 服务端有效性探测 ----------
async function runProbe(accounts) {
  console.log("=== access token 服务端有效性（与 app 状态查询同接口） ===");
  for (const { record, tokens, error } of accounts) {
    if (error || !tokens.access) {
      console.log(`${(record.display_name ?? record.id).padEnd(10)} 无法探测（${error ?? "无 access token"}）`);
      continue;
    }
    try {
      const response = await fetch(RESOURCE_URL, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${tokens.access}`,
          "Content-Type": "application/json",
          Accept: "application/json",
          Origin: "https://www.codebuddy.cn",
          Referer: "https://www.codebuddy.cn/",
          "User-Agent": "Mozilla/5.0 WorkBuddy Desktop Switcher",
        },
        body: "{}",
      });
      const verdict = response.status === 200 ? "有效" : response.status === 401 ? "已失效（需重新登录导入）" : `HTTP ${response.status}`;
      console.log(`${(record.display_name ?? record.id).padEnd(10)} ${verdict}`);
    } catch (error_) {
      console.log(`${record.display_name ?? record.id}: 请求失败 ${error_.message}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}

// ---------- 积分明细 ----------
async function runCredits(clientAuth) {
  console.log("=== 当前客户端账号积分包明细 ===");
  if (!clientAuth?.access) {
    console.log("客户端认证文件缺失或无 token，跳过");
    return;
  }
  const response = await fetch(RESOURCE_URL, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${clientAuth.access}`,
      "Content-Type": "application/json",
      Accept: "application/json",
      Origin: "https://www.codebuddy.cn",
      Referer: "https://www.codebuddy.cn/",
      "User-Agent": "Mozilla/5.0 WorkBuddy Desktop Switcher",
    },
    body: "{}",
  });
  console.log(`HTTP ${response.status}`);
  const payload = await response.json();
  const packages = payload?.data?.Response?.Data?.Accounts ?? [];
  const byGroup = {};
  let appSum = 0;
  for (const pkg of packages) {
    const remain = Number(pkg.CapacityRemainPrecise ?? pkg.CapacityRemain ?? 0);
    const key = `type=${pkg.CapacityType} status=${pkg.Status}`;
    byGroup[key] = (byGroup[key] ?? 0) + remain;
    if (Number(pkg.Status) === 0) appSum += remain;
    console.log(
      `  ${String(pkg.PackageName ?? "?").padEnd(24)} type=${pkg.CapacityType} status=${pkg.Status} 剩余=${pkg.CapacityRemainPrecise ?? pkg.CapacityRemain}`,
    );
  }
  console.log("按类型/状态汇总:");
  for (const [key, value] of Object.entries(byGroup)) console.log(`  ${key}: ${value}`);
  console.log(`app 求和口径（Status==0）: ${Math.round(appSum * 100) / 100}`);
  const totalDosage = payload?.data?.Response?.Data?.TotalDosage;
  if (totalDosage != null) console.log(`服务端 TotalDosage: ${totalDosage}`);
}

const clientAuth = loadClientAuth();
const accounts = loadAccounts();

try {
  if (mode === "local" || mode === "all") runLocal(clientAuth, accounts);
  if (mode === "probe" || mode === "all") await runProbe(accounts);
  if (mode === "credits" || mode === "all") await runCredits(clientAuth);
} catch (error) {
  console.error(`诊断失败: ${error.message}`);
  process.exit(1);
}
