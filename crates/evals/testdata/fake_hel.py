#!/usr/bin/python3
"""Offline executable for testing the eval driver, not a model or hel implementation."""
import json
import os
from pathlib import Path
import sys
import time

args = {}
iterator = iter(sys.argv[1:])
for arg in iterator:
    args[arg] = True if arg in ('--no-env', '--no-context-file', '--no-compaction') else next(iterator)
context = json.loads(Path(args['--context']).read_text())
record_path = Path(args['--record'])
raw = record_path.parent / 'raw'
config_path = Path('fake-config.json')
config = json.loads(config_path.read_text()) if config_path.exists() else {}
count_file = Path('process-count')
number = int(count_file.read_text()) + 1 if count_file.exists() else 1
count_file.write_text(str(number))
(raw / 'invocation.json').write_text(json.dumps({'pid': os.getpid(), 'args': args, 'process': number}))
time.sleep(config.get('delay', 0))
turns = json.loads(Path(args['--turns-file']).read_text()) if '--turns-file' in args else [args['--instruction']]
messages = []
store = Path('.hel/sessions/mock-session')
if '--resume' in args:
    assert args['--resume'] == 'mock-session'
    messages = json.loads((store / 'messages.json').read_text())
entries = []
answer = None
for turn in turns:
    messages.append({'role': 'user', 'content': turn})
    words = ' '.join(m['content'] for m in messages).split()
    token = next((word.rstrip('.') for word in words if word.startswith('HEL-')), 'UNKNOWN')
    answer = 'Acknowledged.' if 'Remember this' in turn else token
    request = {'messages': list(messages), 'tools': [], 'tool_choice': 'none'}
    response = {'usage': {'prompt_tokens': 100 * number, 'completion_tokens': 10, 'prompt_cache_hit_tokens': 20}, 'choices': [{'message': {'role': 'assistant', 'content': answer}}]}
    entries.append({'request': request, 'response': response})
    messages.append(response['choices'][0]['message'])
if not config.get('no_session'):
    store.mkdir(parents=True, exist_ok=True)
    (store / 'messages.json').write_text(json.dumps(messages))
record = json.loads((Path(__file__).parent / 'template.json').read_text())
record['run'].update(context['run'])
record['harness'] = context['harness']
record['model'].update({'provider': context['model']['provider'], 'requested': context['model']['requested'], 'actual': context['model']['requested'], 'params': context['model']['params']})
record['events'] = []
record['outcome'] = {'final_output': answer, 'termination': 'completed', 'error': None}
record['validity'] = {'valid': True, 'reasons': []}
for key, value in {'input_tokens': 100 * number * len(turns), 'output_tokens': 10 * len(turns), 'model_calls': len(turns), 'wall_time_ms': 1, 'cached_input_tokens': 20 * len(turns), 'peak_context_tokens': 100 * number, 'last_context_tokens': 100 * number}.items():
    record['usage'][key] = {'value': value, 'status': 'measured'}
if config.get('fail_first') and number == 1:
    record['outcome'] = {'final_output': None, 'termination': 'error', 'error': 'fake first failure'}
if config.get('missing_usage') and number == 2:
    record['usage']['input_tokens'] = {'value': None, 'status': 'unavailable'}
record_path.write_text(json.dumps(record))
(raw / 'requests.jsonl').write_text(''.join(json.dumps(entry) + '\n' for entry in entries))
(raw / 'permissions.jsonl').write_text('')

if config.get('exit_failure'):
    sys.exit(2)
