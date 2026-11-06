# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Serve local review files; save choices without changing the landing-page source."""
import argparse
import copy
import datetime
import json
import mimetypes
import os
from pathlib import Path
import secrets
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit, unquote
ROOT=Path(__file__).resolve().parent
LOCAL=ROOT/'.local'
DATA={}
SEGMENTS={}
TOKEN=secrets.token_urlsafe(32)
LOCK=threading.Lock()
STATE=LOCAL/'review-state.json'

def load_state():
    if STATE.exists():
        data=json.loads(STATE.read_text())
        if data['revision']!=DATA['revision']:
            raise ValueError('proposal_changed')
        return data
    return {'schema':1,'revision':DATA['revision'],'source_commit':DATA['source_commit'],
            'source_sha256':DATA.get('source_sha256'),'version':0,'records':{}}

def save_choice(body):
    if not isinstance(body,dict) or body.get('revision')!=DATA['revision']:
        raise ValueError('revision')
    ident=body.get('segment')
    if not isinstance(ident,str) or ident not in SEGMENTS:
        raise ValueError('segment')
    segment=SEGMENTS[ident]
    record=body.get('record')
    if not isinstance(record,dict) or record.get('option') not in ['original']+[o['id'] for o in segment['options']]:
        raise ValueError('option')
    values=record.get('values')
    if not isinstance(values,dict) or set(values)!={f['id'] for f in segment['fields']}:
        raise ValueError('fields')
    for f in segment['fields']:
        v=values[f['id']]
        if not isinstance(v,str) or len(v)>10000 or (not v.strip() and f['kind']!='eyebrow'):
            raise ValueError('value')
    base={f['id']:f['text'] for f in segment['fields']} if record['option']=='original' else next(o['values'] for o in segment['options'] if o['id']==record['option'])
    with LOCK:
        state=load_state()
        if body.get('base_version')!=state['version']:
            raise RuntimeError('conflict')
        updated=copy.deepcopy(state)
        stamp=datetime.datetime.now(datetime.timezone.utc).isoformat()
        updated['records'][ident]={'option':record['option'],'edited':values!=base,'values':values,'saved_at':stamp,'status':'chosen'}
        updated['version']+=1
        updated['updated_at']=stamp
        tmp=STATE.with_suffix('.tmp')
        with tmp.open('w') as stream:
            json.dump(updated,stream,ensure_ascii=False,indent=2)
            stream.write('\n');stream.flush();os.fsync(stream.fileno())
        os.replace(tmp,STATE)
        return updated

class Handler(BaseHTTPRequestHandler):
    def allowed(self):
        return self.headers.get('Host') in {f'127.0.0.1:{self.server.server_port}',f'localhost:{self.server.server_port}'}
    def send(self,status,payload,mime='application/json; charset=utf-8'):
        body=json.dumps(payload,ensure_ascii=False).encode() if isinstance(payload,dict) else payload
        self.send_response(status)
        self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)))
        self.send_header('Cache-Control','no-store');self.send_header('X-Content-Type-Options','nosniff')
        self.send_header('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
        self.end_headers();self.wfile.write(body)
    def do_GET(self):
        if not self.allowed():return self.send(403,{'error':'host'})
        route=unquote(urlsplit(self.path).path)
        if route=='/api/state':
            with LOCK:state=load_state()
            return self.send(200,{'state':state,'token':TOKEN})
        if route=='/health':return self.send(200,{'ok':True,'revision':DATA['revision']})
        if route=='/':route='/index.html'
        if route not in {'/index.html','/review.js','/review.css','/res.mjs','/proposals.json'} and not route.startswith('/assets/'):
            return self.send(404,{'error':'not_found'})
        # UI code is retained source; snapshots and decision data live only in .local.
        base=ROOT if route in {'/review.js','/review.css'} else LOCAL
        file=(base/route.lstrip('/')).resolve()
        if not file.is_relative_to(base.resolve()) or not file.is_file():return self.send(404,{'error':'not_found'})
        return self.send(200,file.read_bytes(),mimetypes.guess_type(file)[0] or 'application/octet-stream')
    def do_POST(self):
        origins={f'http://localhost:{self.server.server_port}',f'http://127.0.0.1:{self.server.server_port}'}
        if not self.allowed() or self.headers.get('Origin') not in origins or not secrets.compare_digest(self.headers.get('X-Review-Token',''),TOKEN):
            return self.send(403,{'error':'origin'})
        if self.path!='/api/choice':return self.send(404,{'error':'not_found'})
        try:
            size=int(self.headers.get('Content-Length','0'))
            if not 0<size<=200000 or self.headers.get('Content-Type')!='application/json':raise ValueError('body')
            return self.send(200,{'state':save_choice(json.loads(self.rfile.read(size)))})
        except RuntimeError:return self.send(409,{'error':'conflict'})
        except (ValueError,TypeError,KeyError):return self.send(400,{'error':'invalid'})
        except OSError:return self.send(500,{'error':'save_failed'})

def main():
    global STATE, DATA, SEGMENTS
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port',type=int,default=4335)
    parser.add_argument('--state',type=Path,default=STATE)
    args=parser.parse_args()

    # Both entry points require the same opt-in. Calling this file directly must not
    # bypass the default-off behavior of start.mjs. Enable via the documented launcher:
    # DAY_COPY_REVIEW=1 npm --prefix website run copy:review
    # Ctrl-C stops the server; the variable applies to that invocation only.
    if os.environ.get('DAY_COPY_REVIEW') != '1':
        print('Copy review is disabled. Set DAY_COPY_REVIEW=1 to enable it locally.')
        return
    STATE=args.state
    DATA=json.loads((LOCAL/'proposals.json').read_text())
    SEGMENTS={s['id']:s for s in DATA['segments']}
    load_state()  # Refuse a stale decision file before accepting requests.
    with ThreadingHTTPServer(('127.0.0.1',args.port),Handler) as server:
        print(f'Copy review: http://127.0.0.1:{server.server_port}/; choices: {STATE}',flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass

if __name__=='__main__':main()
