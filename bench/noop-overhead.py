import subprocess,time,os,glob,statistics
H=os.path.expanduser('~')
cwd=glob.glob(H+'/.justrust/bench/work/desktop/jcode@*/jcode')[0]
T=H+'/.justrust/bench/versus/target/desktop/'
stock=os.environ.get('VERSUS_STOCK_CARGO','/usr/bin/cargo')
def t(argv,env):
    e=dict(os.environ,**env); s=time.monotonic(); subprocess.run(argv,cwd=cwd,env=e,capture_output=True); return time.monotonic()-s
res={}
for i in range(6):
    for name,argv,env in [
        ('stock',[stock,'check','-p','jcode-base'],{'CARGO_TARGET_DIR':T+'stock'}),
        ('justrust',['justrust','check','-p','jcode-base'],{'CARGO_TARGET_DIR':T+'justrust','JUSTRUST_REMOTE':'0','JUSTRUST_QUIET':'1'}),
        ('jr-disable',['justrust','check','-p','jcode-base'],{'CARGO_TARGET_DIR':T+'justrust','JUSTRUST_DISABLE':'1'}),
        ('jr-nounify',['justrust','check','-p','jcode-base'],{'CARGO_TARGET_DIR':T+'justrust','JUSTRUST_REMOTE':'0','JUSTRUST_QUIET':'1','JUSTRUST_UNIFY_FEATURES':'0'}),
        ('jr-nofork',['justrust','check','-p','jcode-base'],{'CARGO_TARGET_DIR':T+'justrust','JUSTRUST_REMOTE':'0','JUSTRUST_QUIET':'1','JUSTRUST_CARGO_FORK':'0'}),
    ]:
        res.setdefault(name,[]).append(t(argv,env))
for k,v in res.items(): print(f'{k:12s} median {statistics.median(v[1:]):.3f}  all {[round(x,2) for x in v]}')
