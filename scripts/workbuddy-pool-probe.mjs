// Read-only feasibility probe. Never writes credentials, sessions or client settings.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';

const root = path.join(os.homedir(), '.qiehuan_yingyong');
const index = JSON.parse(fs.readFileSync(path.join(root, 'workbuddy_accounts.json'), 'utf8'));
const db = new DatabaseSync(path.join(os.homedir(), '.workbuddy', 'workbuddy.db'), { readOnly: true });
const columns = db.prepare('PRAGMA table_info(sessions)').all().map(row => row.name);
const summary = {
  accountCount: index.accounts.length,
  sessionsIsolatedByUser: columns.includes('user_id'),
  sessionCount: db.prepare('SELECT count(*) AS n FROM sessions').get().n,
  modelCount: JSON.parse(fs.readFileSync(path.join(os.homedir(), '.workbuddy', 'models.json'), 'utf8')).length,
};
db.close();
if (process.argv.includes('--live') || process.argv.includes('--catalog')) {
  const key = Buffer.from(fs.readFileSync(path.join(root, 'secure-account-storage.key'), 'utf8').trim(), 'base64');
  const account = index.accounts.find(a => !a.token_expires_at || a.token_expires_at > Date.now() / 1000);
  if (!account) throw new Error('No unexpired indexed account; login required.');
  const envelope = JSON.parse(fs.readFileSync(path.join(root, 'workbuddy_accounts', `${account.id}.json`), 'utf8'));
  const bytes = Buffer.from(envelope.ciphertext, 'base64');
  const decipher = crypto.createDecipheriv('aes-256-gcm', key, Buffer.from(envelope.nonce, 'base64'));
  decipher.setAuthTag(bytes.subarray(-16));
  const record = JSON.parse(Buffer.concat([decipher.update(bytes.subarray(0, -16)), decipher.final()]).toString('utf8'));
  const token = record.snapshot?.auth?.accessToken;
  if (typeof token !== 'string' || token.startsWith('$wbEncrypted')) throw new Error('Account needs native credential decoding.');
  const headers = {
    Authorization: `Bearer ${token}`, 'Content-Type': 'application/json',
    Accept: 'application/json, text/event-stream', 'X-User-Id': account.uid,
    'X-No-Enterprise-Id': '1', 'X-No-Department-Info': '1',
    'X-Product': 'SaaS', 'X-CodeBuddy-Request': '1', 'X-Agent-Purpose': 'conversation',
    Origin: 'https://copilot.tencent.com', Referer: 'https://copilot.tencent.com/',
  };
  if (process.argv.includes('--catalog')) {
    const response = await fetch('https://copilot.tencent.com/console/enterprises/personal/models', { headers, signal: AbortSignal.timeout(30000) });
    const payload = await response.json();
    summary.catalog = { status: response.status, code: payload.code, keys: Object.keys(payload.data || {}) };
    const target = process.argv.find(arg => arg.startsWith('--catalog-out='))?.slice('--catalog-out='.length);
    if (target && response.ok && payload.code === 0) fs.writeFileSync(target, JSON.stringify(payload.data, null, 2));
    if (!response.ok || payload.code !== 0) process.exitCode = 1;
  }
  if (process.argv.includes('--live')) {
  const toolsProbe = process.argv.includes('--tools');
  const probeModel = process.argv.find(arg => arg.startsWith('--model='))?.slice('--model='.length) || 'hy3';
  const bodyToSend = toolsProbe ? {
    model: probeModel, stream: true, max_tokens: 64,
    messages: [{ role: 'user', content: 'Call pool_probe with value ready. Do not answer in text.' }],
    tools: [{ type: 'function', function: { name: 'pool_probe', description: 'Report a readiness value', parameters: { type: 'object', properties: { value: { type: 'string' } }, required: ['value'] } } }],
    tool_choice: 'auto',
  } : { model: probeModel, messages: [{ role: 'user', content: 'Reply with OK.' }], max_tokens: 8, stream: true };
  const response = await fetch('https://copilot.tencent.com/v2/chat/completions', {
    method: 'POST', headers,
    body: JSON.stringify(bodyToSend),
    signal: AbortSignal.timeout(45000),
  });
  const body = await response.text();
  summary.live = { model: probeModel, status: response.status, sse: body.includes('data:'), completed: body.includes('[DONE]'), bytes: Buffer.byteLength(body) };
  if (toolsProbe) summary.live.toolCall = body.includes('"tool_calls"') && body.includes('pool_probe');
  if (!response.ok || !summary.live.sse || !summary.live.completed) process.exitCode = 1;
  if (toolsProbe && !summary.live.toolCall) process.exitCode = 1;
  }
}
console.log(JSON.stringify(summary, null, 2));
