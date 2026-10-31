#!/usr/bin/env python3
"""Relocation harness: import stdlib first, then deny outside reads and all effects."""
import argparse
import ctypes
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import socket
import subprocess
import sys

root=Path(sys.argv[1]).resolve()
spec=importlib.util.spec_from_file_location('archived_validator',root/'validate.py')
module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
denied=[]
def hook(event,args):
    if event=='open':
        name,mode,flags=args
        if isinstance(name,int): raise PermissionError('file descriptor reads denied')
        target=Path(os.path.abspath(name))
        if not target.is_relative_to(root) or flags & (os.O_WRONLY|os.O_RDWR|os.O_CREAT|os.O_TRUNC|os.O_APPEND):
            denied.append(event); raise PermissionError('outside read or write denied')
    if event in ('os.listdir','os.scandir'):
        if not Path(os.path.abspath(args[0])).is_relative_to(root):
            denied.append(event); raise PermissionError('outside listing denied')
    if event.startswith(('subprocess.','os.exec','os.spawn','socket.','ctypes.')) or event in ('os.system','os.remove','os.rename','os.mkdir','os.rmdir','os.link','os.symlink','os.truncate'):
        denied.append(event); raise PermissionError('side effect denied')
sys.addaudithook(hook)
result=module.validate(root,manifest=True)
# Exercise real denied attempts. None reaches an external resource or native code.
probes=(lambda:open(root.parent/'absent','rb'),lambda:os.listdir(root.parent),lambda:open(root/'forbidden-write','w'),lambda:subprocess.Popen([sys.executable,'-c','pass']),lambda:socket.socket(),lambda:ctypes.CDLL(None))
for probe in probes:
    try: probe()
    except PermissionError: pass
    else: raise AssertionError('sandbox did not deny probe')
assert len(denied)==6 and os.environ.get('HOME')=='' and os.environ.get('PATH')==''
print(json.dumps({'isolated_replay':result,'denied_probes':denied},sort_keys=True))
