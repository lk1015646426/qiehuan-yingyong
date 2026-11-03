"""Read-only TRAE credential/protocol probe. Never prints or persists credentials."""
import base64
import json
import os
from pathlib import Path
import time
import urllib.request
import urllib.error
import uuid
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

root = Path.home() / '.qiehuan_yingyong'
key = base64.b64decode((root / 'secure-account-storage.key').read_text().strip())
for path in sorted((root / 'trae_accounts').glob('*.json')):
    raw = json.loads(path.read_text(encoding='utf-8'))
    if raw.get('algorithm') == 'AES-256-GCM':
        raw = json.loads(AESGCM(key).decrypt(base64.b64decode(raw['nonce']), base64.b64decode(raw['ciphertext']), None))
    token = raw.get('access_token', '')
    try:
        payload = token.split('.')[1]
        jwt = json.loads(base64.urlsafe_b64decode(payload + '=' * (-len(payload) % 4)))
    except Exception:
        jwt = {}
    auth = raw.get('trae_auth_raw') or {}
    print(json.dumps({'id': raw.get('id'), 'label': raw.get('tags'), 'expiresInHours': round((jwt.get('exp', 0)-time.time())/3600, 1), 'hasRefresh': bool(raw.get('refresh_token')), 'authKeys': list(auth), 'jwtKeys': list(jwt), 'deviceNumeric': str(raw.get('auth_device_id', '')).isdigit()}, ensure_ascii=False))
    if os.environ.get('TRAE_READONLY_NETWORK') != '1':
        continue
    headers = {'Authorization': f'Cloud-IDE-JWT {token}', 'X-Cloudide-Token': token, 'Content-Type': 'application/json', 'x-device-id': raw.get('auth_device_id', ''), 'x-app-id': '6eefa01c-1036-4c7e-9ca5-d891f63bfcd8'}
    endpoints = [('https://api.trae.cn', '/trae/api/v2/ug/checkin_credits/status', {})]
    if os.environ.get('TRAE_PROBE_MODELS') == '1':
        endpoints += [('https://trae-api-cn.mchost.guru', '/api/ide/v1/model_list?type=llm_raw_chat', None)]
    for base, endpoint, body in endpoints:
        req = urllib.request.Request(base + endpoint, data=json.dumps(body).encode() if body is not None else None, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=20) as res:
                data = json.loads(res.read())
                print(json.dumps({'endpoint': endpoint, 'http': res.status, 'code': data.get('code'), 'keys': list(data), 'checkedIn': data.get('checked_in'), 'enabled': data.get('enable'), 'shape': str(data)[:600] if 'model_list' in endpoint else None}, ensure_ascii=False))
                if os.environ.get('TRAE_PROBE_CHAT') == '1' and 'model_configs' in data:
                    models = [m for m in data['model_configs'] if m.get('is_preset') and m.get('status')]
                    print('models', [(m.get('name'), m.get('config_name')) for m in models])
                    model = models[0]['name']
                    body = {'model_name': model, 'is_preset': True, 'messages': [{'role': 'user', 'content': 'Reply only OK.'}], 'user_input': 'Reply only OK.', 'session_id': str(uuid.uuid4()), 'max_tokens': 4, 'stream': True}
                    chat_req = urllib.request.Request(base + '/api/ide/v1/llm_raw_chat', data=json.dumps(body).encode(), headers=headers)
                    try:
                        with urllib.request.urlopen(chat_req, timeout=30) as chat_res:
                            sample = chat_res.read(4096).decode(errors='replace')
                            print('chat_probe', chat_res.status, chat_res.headers.get('Content-Type'), sample[:1800] if token not in sample else '[redacted]')
                    except urllib.error.HTTPError as chat_exc:
                        print('chat_probe', chat_exc.code, chat_exc.read()[:400].decode(errors='replace'))
        except urllib.error.HTTPError as exc:
            try:
                error = json.loads(exc.read())
                message = error.get('message', error.get('msg', error.get('error', '')))
                if isinstance(message, dict):
                    message = message.get('message', '')
                if not isinstance(message, str) or token in message:
                    message = '[redacted]'
            except Exception:
                message = ''
            print(json.dumps({'endpoint': endpoint, 'http': exc.code, 'message': message[:240]}, ensure_ascii=False))
        except Exception as exc:
            print(json.dumps({'endpoint': endpoint, 'errorType': type(exc).__name__}))
