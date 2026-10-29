"""Test-only native ACP consumer. Native defaults/providers are never fabricated by this client."""
import json
import os
import select
import signal
import subprocess
import sys
import time
import tempfile

def prompt(product, command, text, selector=None, env=None, timeout=190):
    stderr = tempfile.TemporaryFile()
    # Set the child's private creation mask before exec without preexec_fn (unsafe
    # in threaded fixtures) or Popen's umask argument, absent on Python 3.8.
    private_command = [sys.executable, '-c',
        'import os,sys;os.umask(0o077);os.execvp(sys.argv[1],sys.argv[1:])', *command]
    process = subprocess.Popen(private_command,cwd=product.project,env=env or product.env,
        stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr,start_new_session=True)
    pending = bytearray()
    deadline, updates, counter = time.monotonic()+timeout, [], 0
    def request(method,params):
        nonlocal counter
        counter += 1
        process.stdin.write((json.dumps({'jsonrpc':'2.0','id':counter,'method':method,'params':params})+'\n').encode())
        process.stdin.flush()
        while True:
            remaining = deadline-time.monotonic()
            while b'\n' not in pending:
                assert remaining>0 and select.select([process.stdout],[],[],remaining)[0], 'native ACP request timeout'
                chunk = os.read(process.stdout.fileno(),65536)
                assert chunk, 'native ACP closed before reply'
                pending.extend(chunk)
                assert len(pending)<=4*1024*1024, 'native ACP message limit'
                remaining = deadline-time.monotonic()
            line, _, tail = pending.partition(b'\n')
            pending[:] = tail
            product.outputs.append(line)
            message = json.loads(line)
            if message.get('method') == 'session/update': updates.append(message['params']['update'])
            if message.get('id') == counter:
                assert 'error' not in message, 'native ACP rejected selected operation'
                return message['result']
    try:
        initialized = request('initialize',{'protocolVersion':1,'clientCapabilities':{},'clientInfo':{'name':'hiroute-acceptance','version':'1'}})
        assert initialized['protocolVersion']==1
        session = request('session/new',{'cwd':str(product.project),'mcpServers':[]})
        if selector is not None:
            provider,model = selector.split('/',1)
            selected = json.dumps([provider,model],separators=(',',':'))
            option = next(o for o in session['configOptions'] if o.get('category')=='model' or o.get('id')=='model')
            request('session/set_config_option',{'sessionId':session['sessionId'],'configId':option['id'],'value':selected})
        if text is None:
            # A real native startup/session roundtrip without model inference.
            return b'', session['sessionId']
        result = request('session/prompt',{'sessionId':session['sessionId'],'prompt':[{'type':'text','text':text}]})
        assert result['stopReason']=='end_turn', 'native ACP did not complete normally'
        output = ''.join(u.get('content',{}).get('text','') for u in updates if u.get('sessionUpdate')=='agent_message_chunk')
        return output.encode(),session['sessionId']
    finally:
        if process.poll() is None:
            os.killpg(process.pid,signal.SIGTERM)
            try: process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid,signal.SIGKILL)
                process.wait(timeout=3)
        stderr.seek(0)
        product.outputs.append(stderr.read(2*1024*1024))
        stderr.close()
