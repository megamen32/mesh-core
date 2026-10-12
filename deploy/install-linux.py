#!/usr/bin/env python3
"""Install only the new, source-pinned private core service; no product restart."""
import argparse, hashlib, json, os, pathlib, pwd, grp, socket, subprocess, time, stat
import http.client

def digest(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--binary',required=True);ap.add_argument('--sha256',required=True);ap.add_argument('--source',required=True);ap.add_argument('--user',required=True);ap.add_argument('--host',required=True);ap.add_argument('--receipt',required=True);a=ap.parse_args()
    if os.geteuid()!=0:raise RuntimeError('system service installation requires owning administrator')
    if socket.gethostname()!=a.host:raise RuntimeError('native host identity mismatch')
    owner=pwd.getpwnam(a.user);group=grp.getgrgid(owner.pw_gid).gr_name
    fd=os.open(a.binary,os.O_RDONLY|os.O_NOFOLLOW)
    try:
        before=os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_size>16*1024**2:raise RuntimeError('bounded regular candidate required')
        with os.fdopen(fd,'rb',closefd=False) as input:data=input.read(16*1024**2+1)
        after=os.fstat(fd)
        if (before.st_dev,before.st_ino,before.st_mtime_ns,before.st_size)!=(after.st_dev,after.st_ino,after.st_mtime_ns,after.st_size):raise RuntimeError('candidate generation drift')
    finally:os.close(fd)
    if not data.startswith(b'\x7fELF') or hashlib.sha256(data).hexdigest()!=a.sha256:raise RuntimeError('candidate digest mismatch')
    if len(a.source)!=40 or any(c not in '0123456789abcdef' for c in a.source):raise RuntimeError('source pin invalid')
    available=next(int(l.split()[1])*1024 for l in pathlib.Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:'))
    if available<20*1024**3+64*1024**2:raise RuntimeError('host reserve unavailable')
    unit=pathlib.Path('/etc/systemd/system/mesh-core.service');runtime=pathlib.Path('/run/mesh-core-'+a.user)
    if os.path.lexists(unit) or os.path.lexists(runtime):raise RuntimeError('existing core owner must be reconciled before installation')
    receipt_path=pathlib.Path(a.receipt)
    intent={'status':'intent','host':a.host,'source':a.source,'binary_sha256':a.sha256,'other_product_restarts':0,'fleet_adoption':False}
    def save(record):
        with receipt_path.open('w') as output:output.write(json.dumps(record,indent=2)+'\n');output.flush();os.fsync(output.fileno())
        receipt_path.chmod(0o600)
    save(intent)
    created=False;started=False;template=None
    try:
        target=pathlib.Path('/opt/mesh-core/releases')/a.source/'mesh-core-service';target.parent.mkdir(parents=True,mode=0o755,exist_ok=True)
        if target.parent.resolve()!=target.parent:raise RuntimeError('candidate directory is not canonical')
        with target.open('xb') as output:output.write(data);output.flush();os.fsync(output.fileno())
        target.chmod(0o755)
        template=pathlib.Path(__file__).with_name('mesh-core.service.in').read_text()
        for key,value in {'USER':a.user,'GROUP':group,'UID':str(owner.pw_uid),'BINARY':str(target)}.items():template=template.replace('@'+key+'@',value)
        if '@' in template:raise RuntimeError('unresolved unit template')
        temporary=unit.with_name('mesh-core-owner55-new.service')
        with temporary.open('x') as output:output.write(template)
        temporary.chmod(0o644)
        subprocess.run(['systemd-analyze','verify',str(temporary)],check=True,capture_output=True,timeout=10)
        temporary.rename(unit);created=True
        subprocess.run(['systemctl','daemon-reload'],check=True,timeout=10)
        started=True
        subprocess.run(['systemctl','enable','--now','mesh-core.service'],check=True,capture_output=True,timeout=15)
        props=subprocess.check_output(['systemctl','show','mesh-core.service','-p','MainPID','-p','InvocationID','-p','ActiveState','-p','MemoryMax','-p','MemorySwapMax','-p','TasksMax'],text=True,timeout=5)
        values=dict(line.split('=',1) for line in props.splitlines() if '=' in line)
        pid=int(values.get('MainPID','0'));assert pid>0 and values['ActiveState']=='active'
        assert digest('/proc/'+str(pid)+'/exe')==a.sha256
        sock=runtime/'core.sock'
        for _ in range(40):
            if sock.exists():break
            time.sleep(.05)
        class PrivateHTTP(http.client.HTTPConnection):
            def connect(self):self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.settimeout(3);self.sock.connect(str(sock))
        conn=PrivateHTTP('mesh-core',timeout=3)
        data=json.dumps({'local_host_id':a.host,'known_hosts':['server-44'],'hosts':'server-44','request_id':'core-install-owner-read','origin_host':a.host,'hop_count':0}).encode()
        try:
            conn.request('POST','/v1/normalize',data,{'Content-Type':'application/json'});response=conn.getresponse();body=json.loads(response.read(1048576));assert response.status==200 and body['hosts']==['server-44'] and body['request_id']=='core-install-owner-read'
        finally:conn.close()
        receipt={'status':'installed','host':a.host,'source':a.source,'binary_sha256':a.sha256,'pid':pid,'invocation':values['InvocationID'],'private_socket':str(sock),'root_product_ipc_pass':True,'memory_max':values['MemoryMax'],'swap_max':values['MemorySwapMax'],'tasks_max':values['TasksMax'],'other_product_restarts':0,'fleet_adoption':False}
        save(receipt);print(json.dumps(receipt))
    except BaseException as error:
        failure={**intent,'status':'failed','failure_class':type(error).__name__,'rollback':'not_started'}
        try:
            if created and template is not None and unit.read_text()==template:
                pid=int(subprocess.check_output(['systemctl','show','mesh-core.service','-p','MainPID','--value'],text=True,timeout=5).strip() or '0')
                if pid==0 or digest('/proc/'+str(pid)+'/exe')==a.sha256:
                    if started:subprocess.run(['systemctl','stop','mesh-core.service'],check=True,timeout=10)
                    subprocess.run(['systemctl','disable','mesh-core.service'],check=True,capture_output=True,timeout=5)
                    pid_after=subprocess.check_output(['systemctl','show','mesh-core.service','-p','MainPID','--value'],text=True,timeout=5).strip()
                    failure['rollback']='owned_new_service_stopped_disabled' if pid_after=='0' else 'unverified'
                else:failure['rollback']='refused_foreign_generation'
        except BaseException as rollback_error:
            failure["rollback"]="cleanup_unverified"
            failure["rollback_error_class"]=type(rollback_error).__name__
        save(failure)
        raise

if __name__=='__main__':main()
