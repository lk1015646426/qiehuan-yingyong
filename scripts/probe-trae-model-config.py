"""Read-only official model-configuration probe. No claim, chat or token refresh."""
import base64
import json
from pathlib import Path
import urllib.request
import urllib.error
import uuid
import argparse
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

parser = argparse.ArgumentParser()
parser.add_argument('--function', choices=['chat', 'builder_with_mcp', 'chat_v3', 'solo_agent', 'solo_coder', 'solo_work_lite', 'solo_agent_remote', 'solo_work_remote'], default='solo_agent')
parser.add_argument('--ide-version', default='3.4.13')
parser.add_argument('--version-code', default='20250325')
parser.add_argument('--model', default='glm-5.2')
parser.add_argument('--probe', choices=['config', 'remote-catalog', 'manager-catalog', 'inline-chat', 'native-chat', 'solo-chat'], default='config')
args = parser.parse_args()
root = Path.home() / '.qiehuan_yingyong'
key = base64.b64decode((root / 'secure-account-storage.key').read_text().strip())
for path in sorted((root / 'trae_accounts').glob('*.json')):
    envelope = json.loads(path.read_text(encoding='utf-8'))
    account = json.loads(AESGCM(key).decrypt(base64.b64decode(envelope['nonce']), base64.b64decode(envelope['ciphertext']), None))
    token = account['access_token']
    device = (account.get('trae_auth_raw') or {}).get('deviceInfo') or {}
    headers = {
        'Authorization': 'Cloud-IDE-JWT ' + token, 'X-Cloudide-Token': token,
        'X-Ide-Token': token, 'X-App-Function': args.function, 'X-Ide-Function': args.function,
        'X-Plugin-Channel': 'desktop', 'X-Request-Id': uuid.uuid4().hex,
        'Accept': 'application/json', 'Content-Type': 'application/json',
        'x-app-id': '6eefa01c-1036-4c7e-9ca5-d891f63bfcd8',
        'x-app-version': 'default', 'x-ide-version': args.ide_version,
        'x-ide-version-code': args.version_code, 'x-app-version-code': args.version_code,
        'x-ide-version-type': 'stable', 'request-traffic-type': 'prod',
        'x-device-type': 'Windows', 'x-device-id': account['auth_device_id'],
        'x-machine-id': account.get('machine_id') or device.get('MachineID', ''),
        'x-uid': account.get('user_id') or '', 'x-custom-trace-id': uuid.uuid4().hex,
    }
    headers = {k: v for k, v in headers.items() if v}
    headers.update({'User-Agent': 'Trae/' + args.ide_version, 'x-device-type': 'windows'})
    url = 'https://trae-api-cn.mchost.guru/api/ide/v1/get_detail_param'
    body = json.dumps({'function': args.function, 'config_names': None, 'need_prompt': False, 'current_config_info': None, 'poly_prompt': True, 'mode_type': None, 'agent_type': None}).encode()
    if args.probe == 'manager-catalog':
        url = 'https://trae-api-cn.mchost.guru/api/ide/v1/model_list?type=llm_raw_chat'
        body = None
    if args.probe in ['remote-catalog', 'native-chat']:
        url = 'https://trae-api-cn.mchost.guru/api/remote/v1/models?functions=solo_agent_remote%2Csolo_work_remote%2Csolo_design_remote&show_custom_model=true'
        headers.update({'X-Trae-Client-Type': 'web', 'X-Preferenced-Language': 'zh-CN', 'x-user-region': 'CN', 'Origin': 'https://solo.trae.cn', 'Referer': 'https://solo.trae.cn/'})
        body = None
    elif args.probe == 'inline-chat':
        url = 'https://trae-api-cn.mchost.guru/api/agent/v3/llm_utils_chat'
        headers['Accept'] = 'text/event-stream'
        body = json.dumps({'messages': [{'role': 'user', 'content': [{'type': 'text', 'text': 'Reply only OK.'}]}], 'function': 'inline_chat', 'stream': True, 'max_tokens': 8}).encode()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers=headers, data=body), timeout=25) as response:
            if args.probe == 'inline-chat':
                sample = response.read(4096).decode(errors='replace')
                print(json.dumps({'probe': args.probe, 'http': response.status, 'contentType': response.headers.get('Content-Type'), 'sample': sample if token not in sample else '[redacted]'}, ensure_ascii=False))
                continue
            data = json.loads(response.read())
            if args.probe == 'manager-catalog':
                print(json.dumps({'probe':args.probe,'http':response.status,'code':data.get('code'),'keys':list(data),'rows':[{'keys':list(m),'isPreset':m.get('is_preset'),'provider':m.get('provider'),'managed':str(m.get('name') or m.get('model_name') or '').startswith('trae-pool-')} for m in data.get('model_configs') or []]},ensure_ascii=False))
                continue
            if args.probe == 'remote-catalog':
                groups = (data.get('data') or {}).get('list') or []
                print(json.dumps({'probe': args.probe, 'http': response.status, 'code': data.get('code'), 'groups': [{'function':g.get('function'), 'models':[{k:m.get(k) for k in ['name','model_name','raw_model_name','config_name','config_source','provider','is_default']} for m in g.get('models') or []]} for g in groups]}, ensure_ascii=False))
                continue
            if args.probe == 'native-chat':
                groups = (data.get('data') or {}).get('list') or []
                group = next(g for g in groups if g.get('function') == args.function)
                models = group['models']
                model = next((m for m in models if m.get('is_default')), models[0])
                session = uuid.uuid4().hex
                payload = {'config_name': model.get('config_name') or model['name'], 'model_name': model.get('model_name') or model['name'], 'config_source':model.get('config_source') or 1, 'raw_chat_function':args.function, 'session_id':session, 'conversation_id':session, 'is_custom_model':False, 'stream':True, 'messages':[{'role':'user','content':[{'type':'text','text':'Reply only OK.'}]}]}
                native_headers = dict(headers, Accept='text/event-stream')
                native_url = 'https://trae-api-cn.mchost.guru/api/ide/v2/llm_raw_chat'
                with urllib.request.urlopen(urllib.request.Request(native_url, headers=native_headers, data=json.dumps(payload).encode()), timeout=25) as native_response:
                    sample = native_response.read(4096).decode(errors='replace')
                    print(json.dumps({'probe':args.probe,'model':model['name'],'http':native_response.status,'sample':sample if token not in sample else '[redacted]'},ensure_ascii=False))
                continue
            configs = data.get('config_info_list') or []
            selected_meta = next((c for c in configs if c.get('config_name') == args.model), None)
            if selected_meta and args.probe == 'config':
                print(json.dumps({'selectedModel':args.model,'configSwitch':selected_meta.get('config_switch'),'invisible':selected_meta.get('is_invisible_to_user'),'displayConfig':selected_meta.get('display_config'),'extraConfig':selected_meta.get('extra_config')},ensure_ascii=False))
            if args.probe == 'solo-chat':
                model = next(c for c in configs if c.get('config_name') == args.model)
                assert model.get('config_switch') is not False
                chat_body = {'messages':[{'role':'user','content':[{'type':'text','text':'仅回复OK，不要解释。'}]}], 'function':args.function, 'stream':True, 'config_name':args.model, 'model':args.model, 'max_tokens':256}
                chat_headers = dict(headers, Accept='text/event-stream')
                chat_url = 'https://trae-api-cn.mchost.guru/api/agent/v3/llm_utils_chat'
                with urllib.request.urlopen(urllib.request.Request(chat_url, headers=chat_headers, data=json.dumps(chat_body).encode()), timeout=45) as chat_response:
                    sample=chat_response.read(16384).decode(errors='replace')
                    print(json.dumps({'probe':args.probe,'model':args.model,'http':chat_response.status,'sample':sample if token not in sample else '[redacted]'},ensure_ascii=False))
                continue
            print(json.dumps({'accountId': account['id'], 'http': response.status, 'code': data.get('code'), 'keys': list(data), 'models': [{'configName': c.get('config_name'), 'details': len(c.get('model_detail_list') or []), 'hasEncryptedParameters': any(bool(d.get('encrypted_model_params')) for d in c.get('model_detail_list') or [])} for c in configs]}, ensure_ascii=False))
    except urllib.error.HTTPError as error:
        print(json.dumps({'accountId': account['id'], 'http': error.code}))
    except Exception as error:
        print(json.dumps({'accountId': account['id'], 'errorType': type(error).__name__}))
