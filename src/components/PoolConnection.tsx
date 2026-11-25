import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export function PoolConnection({ app, running }: { app: string; running?: boolean }) {
  const pool = app === 'TRAE' ? 'trae' : app.startsWith('豆包') ? 'doubao' : 'workbuddy';
  const base = `http://127.0.0.1:${pool === 'trae' ? 18790 : pool === 'doubao' ? 18791 : 18789}/v1`;
  const [key, setKey] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState('');

  useEffect(() => {
    let active = true;
    setBusy(true); setKey(null); setFeedback('');
    void invoke<string>('get_pool_connection_key', { pool }).then(value => { if (active) setKey(value); }).catch(error => { if (active) setFeedback(String(error)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [pool, running]);
  async function copy(value: string, label: string) {
    try { await navigator.clipboard.writeText(value); setFeedback(`${label}已复制`); }
    catch { setFeedback('复制失败，请手动选择并复制'); }
  }
  return <div className="ap-connection">
    <h3>本地模型接口</h3>
    <div className="ap-connection-row"><span>Base URL</span><code>{base}</code><button type="button" className="ap-button" onClick={() => void copy(base, '地址')}>复制</button></div>
    <div className="ap-connection-row"><span>请求地址</span><code>{`${base}/chat/completions`}</code><button type="button" className="ap-button" onClick={() => void copy(`${base}/chat/completions`, '请求地址')}>复制</button></div>
    <div className="ap-connection-row"><span>API Key</span><code className="ap-key">{key ?? (busy ? '读取中…' : '尚不可用')}</code><button type="button" className="ap-button" disabled={key === null} onClick={() => key !== null && void copy(key, 'API Key')}>复制</button></div>
    <p>地址为 OpenAI 兼容 Base URL，服务仅监听本机；使用前请启动账号池。</p>
    {feedback && <p role="status">{feedback}</p>}
  </div>;
}
