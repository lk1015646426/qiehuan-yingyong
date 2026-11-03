"""Explicit synthetic acceptance against the locally running TRAE pool.

No project files or personal messages are sent. The local API key stays in memory.
The official account must already have been imported by the user.
"""
import json
from pathlib import Path
import urllib.request
import urllib.error
import uuid
import argparse

parser = argparse.ArgumentParser()
parser.add_argument('--output', required=True)
args = parser.parse_args()
root = Path.home() / '.qiehuan_yingyong'
key = (root / 'trae-model-pool.key').read_text().strip()
endpoint = 'http://127.0.0.1:18790/v1'
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def send(path, body=None, authenticated=True):
    headers = {'Content-Type': 'application/json'}
    if authenticated:
        headers['Authorization'] = 'Bearer ' + key
    request = urllib.request.Request(endpoint + path, headers=headers, data=json.dumps(body).encode() if body is not None else None)
    return opener.open(request, timeout=180)

def chat(body):
    try:
        with send('/chat/completions', body) as response:
            result = json.loads(response.read())
    except urllib.error.HTTPError as error:
        detail = error.read().decode('utf-8', errors='replace')
        if key in detail:
            detail = '[redacted]'
        raise RuntimeError(f'Pool HTTP {error.code}: {detail[:400]}') from None
    if result.get('error'):
        raise RuntimeError('Local pool returned an error')
    return result

results = {}
try:
    with send('/models', authenticated=False):
        raise AssertionError('Unauthenticated request was accepted')
except urllib.error.HTTPError as error:
    assert error.code == 401
    results['unauthenticated_rejected'] = True
with send('/models') as response:
    models = json.loads(response.read())['data']
assert models
model = models[0]['id']
results['model'] = model
nonce = 'pool-' + uuid.uuid4().hex[:10]
history = [{'role': 'user', 'content': f'Remember this exact code: {nonce}. Reply only ACK.'}]
first = chat({'model': model, 'messages': history, 'max_tokens': 512})
assert first['choices'][0]['finish_reason'] == 'stop'
history.append(first['choices'][0]['message'])
history.append({'role': 'user', 'content': 'What exact code did I ask you to remember? Reply only that code.'})
second = chat({'model': model, 'messages': history, 'max_tokens': 512})
assert nonce in second['choices'][0]['message']['content']
results['two_turn_context'] = True
print('two_turn_context passed', flush=True)
results['nonstream_usage_reported'] = 'usage' in second

tools = [{'type': 'function', 'function': {'name': 'echo', 'description': 'Returns the supplied value without side effects.', 'parameters': {'type': 'object', 'properties': {'value': {'type': 'string'}}, 'required': ['value']}}}]
tool_history = [{'role': 'user', 'content': f'Call echo with value {nonce}. Use the echo tool.'}]
tool_response = chat({'model': model, 'messages': tool_history, 'tools': tools, 'tool_choice': {'type': 'function', 'function': {'name': 'echo'}}, 'max_tokens': 768})
assistant = tool_response['choices'][0]['message']
calls = assistant.get('tool_calls') or []
assert calls and calls[0]['function']['name'] == 'echo'
arguments = json.loads(calls[0]['function']['arguments'])
assert arguments['value'] == nonce
results['native_tool_call'] = True
print('native_tool_call passed', flush=True)
tool_history.append(assistant)
for call in calls:
    tool_history.append({'role': 'tool', 'tool_call_id': call['id'], 'content': 'echo-result-' + nonce})
tool_history.append({'role': 'user', 'content': 'Reply with the exact result returned by the echo tool.'})
trace = Path(args.output).with_suffix('.tool-history.json')
trace.parent.mkdir(parents=True, exist_ok=True)
trace.write_text(json.dumps({'model': model, 'messages': [{k:v for k,v in m.items() if k != 'reasoning_content'} for m in tool_history], 'tools': tools, 'tool_choice': 'none', 'max_tokens': 768}, indent=2), encoding='utf-8')
after_tool = chat({'model': model, 'messages': tool_history, 'tools': tools, 'tool_choice': 'none', 'max_tokens': 768})
assert 'echo-result-' + nonce in after_tool['choices'][0]['message']['content']
results['native_tool_result_roundtrip'] = True

text = ''
finished = False
usage = False
with send('/chat/completions', {'model': model, 'stream': True, 'stream_options': {'include_usage': True}, 'messages': [{'role': 'user', 'content': 'Reply only STREAMOK.'}], 'max_tokens': 512}) as response:
    for raw in response:
        line = raw.decode('utf-8').strip()
        if not line.startswith('data:'):
            continue
        data = line[5:].strip()
        if data == '[DONE]':
            finished = True
            break
        chunk = json.loads(data)
        assert 'error' not in chunk
        usage = usage or 'usage' in chunk
        for choice in chunk.get('choices') or []:
            text += (choice.get('delta') or {}).get('content') or ''
assert finished and 'STREAMOK' in text
results['streamed_response'] = True
results['streamed_usage_reported'] = usage
output = Path(args.output)
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps(results, ensure_ascii=False))
