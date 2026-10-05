"""Emulator regression: run after launching a fresh probe on the 360x720 phone.

Uses real mouse tab selection and system Back; checks native TitleBar ownership,
not merely the presence of retained Day nodes in its logical tree.
"""
import json
import os
from pathlib import Path
import subprocess
import time

import argparse
parser=argparse.ArgumentParser()
parser.add_argument('--project',type=Path,required=True)
parser.add_argument('--day',type=Path,required=True)
args=parser.parse_args()
project=args.project.resolve()
day=args.day.resolve()
out=project/'build/navigation-validation'
out.mkdir(parents=True,exist_ok=True)
target=os.environ.get('DAY_OHOS_TARGET','127.0.0.1:55555')
checks=0

def hdc(*args):
    return subprocess.run(['hdc','-t',target,*map(str,args)],check=True,text=True,capture_output=True).stdout

def drive(*steps):
    result=subprocess.run([str(day),'drive','--project',str(project),'-p','harmony-arkui','--steps-json',json.dumps([*steps,{'wait_idle':{}}])],check=True,text=True,capture_output=True)
    value=json.loads(result.stdout)
    assert value['failed']==0,value

def titles(expected):
    global checks
    deadline=time.monotonic()+12
    while True:
        hdc('shell','uitest','dumpLayout','-p','/data/local/tmp/nav-probe-tree.json')
        hdc('file','recv','/data/local/tmp/nav-probe-tree.json',out/'tree.json')
        tree=json.loads((out/'tree.json').read_text())
        actual=[]
        def visit(node):
            a=node.get('attributes',{})
            if a.get('visible')=='false':return
            if a.get('type')=='TitleBar':actual.append(a.get('text'))
            for child in node.get('children',[]):visit(child)
        visit(tree)
        if actual==expected:break
        if time.monotonic()>deadline:raise AssertionError((expected,actual))
    checks+=1
    print('PASS native titles',expected,flush=True)

def tab(x):
    hdc('shell','uinput','-M','-m',x,660,'-d',0,'-i',100,'-u',0)
    drive()

def back():
    hdc('shell','uitest','uiInput','keyEvent','Back')
    drive()

for cycle in range(5):
    tab(55); titles(['Library'])
    drive({'tap':{'id':'push-Library'}})
    drive({'tap':{'id':'deeper-Library'}})
    titles(['Library deeper'])
    tab(178); titles(['Catalogs'])
    drive({'tap':{'id':'push-Catalogs'}}); titles(['Catalogs detail'])
    tab(55); titles(['Library deeper'])
    back(); titles(['Library detail'])
    tab(300); titles([])
    if cycle==0:
        hdc('shell','uitest','screenCap','-p','/data/local/tmp/nav-settings.png')
        hdc('file','recv','/data/local/tmp/nav-settings.png',out/'settings.png')
    tab(178); titles(['Catalogs detail'])
    back(); titles(['Catalogs'])
    tab(55); titles(['Library detail'])
    drive({'tap':{'id':'pop-Library'}}); titles(['Library'])
    print('PASS cycle',cycle+1,flush=True)
# A hidden sibling can receive a push without stealing the visible title or
# keeping dayscript wait_idle pending forever.
drive({'tap':{'id':'hidden-Library'}}); titles(['Library'])
tab(178); titles(['hidden detail'])
back(); titles(['Catalogs'])
tab(55)
drive({'tap':{'id':'push-Library'}},{'tap':{'id':'guard-Library'}})
back(); titles(['Library detail'])
drive({'tap':{'id':'guard-Library'}})
back(); titles(['Library'])
drive({'tap':{'id':'transient-Library'}}); titles(['Library'])
drive({'tap':{'id':'push-Library'}})
tab(178); drive({'tap':{'id':'push-Catalogs'}})
drive({'tap':{'id':'mount-tabs'}}); titles([])
drive({'tap':{'id':'mount-tabs'}}); titles(['Library'])
drive({'tap':{'id':'push-Library'}}); titles(['Library detail'])
back(); titles(['Library'])
(out/'result.json').write_text(json.dumps({'cycles':5,'native_title_checks':checks,'failed':0},indent=2)+'\n')
