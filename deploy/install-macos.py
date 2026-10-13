#!/usr/bin/env python3
"""Install a new per-owner, background-only core. Existing instances refuse.

Uses the existing sealed Mac fleet observer. No product/native restart.
"""
import argparse, hashlib, http.client, json, os, platform, plistlib, re, socket, stat, subprocess, time
from pathlib import Path

def sealed(path,expected,maximum):
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    try:
        first=os.fstat(fd)
        if not stat.S_ISREG(first.st_mode) or first.st_uid!=os.getuid() or first.st_mode&0o022:raise RuntimeError('candidate_owner')
        with os.fdopen(fd,'rb',closefd=False) as f:data=f.read(maximum+1)
        last=os.fstat(fd)
        if len(data)>maximum or (first.st_dev,first.st_ino,first.st_size,first.st_mtime_ns,first.st_ctime_ns)!=(last.st_dev,last.st_ino,last.st_size,last.st_mtime_ns,last.st_ctime_ns):raise RuntimeError('candidate_generation')
    finally:os.close(fd)
    if hashlib.sha256(data).hexdigest()!=expected:raise RuntimeError('candidate_hash')
    return data

def write_new(path,data,mode):
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,mode)
    with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())

class PrivateHTTP(http.client.HTTPConnection):
    def __init__(self,path):super().__init__('mesh-core',timeout=3);self.path=path
    def connect(self):
        self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.settimeout(3);self.sock.connect(str(self.path))

def main():
    ap=argparse.ArgumentParser()
    for key in ('binary','binary-sha','source','watchdog','watchdog-sha','observer-sha','probe-sha','python','host','receipt'):ap.add_argument('--'+key,required=True)
    ap.add_argument('--reserve-gib',type=int,choices=(2,4),required=True);a=ap.parse_args()
    if platform.system()!='Darwin' or os.getuid()==0:raise RuntimeError('native_owner_required')
    if not re.fullmatch('[0-9a-f]{40}',a.source):raise RuntimeError('source_pin')
    host=subprocess.check_output(['/usr/sbin/scutil','--get','LocalHostName'],timeout=3).decode().strip()
    if host!=a.host:raise RuntimeError('host_identity')
    data=sealed(a.binary,a.binary_sha,16*1024**2)
    cpu=0x0100000c if platform.machine()=='arm64' else 0x01000007
    if len(data)<12 or data[:4]!=b'\xcf\xfa\xed\xfe' or int.from_bytes(data[4:8],'little')!=cpu:raise RuntimeError('native_architecture')
    wrapper=sealed(a.watchdog,a.watchdog_sha,128*1024)
    fleet=Path.home()/'.local/share/agent-herder/fleet'
    sealed(fleet/'mac-node-watchdog.py',a.observer_sha,128*1024)
    sealed(fleet/'mac-codex-owner-probe',a.probe_sha,1024**2)
    python=Path(a.python)
    if not python.is_absolute() or not python.is_file() or not os.access(python,os.X_OK):raise RuntimeError('python_native')
    base=Path.home()/'.local/share/mesh-core';plist=Path.home()/'Library/LaunchAgents/com.bezrabotnyi.mesh-core.plist'
    label='com.bezrabotnyi.mesh-core';domain='user/'+str(os.getuid());target=domain+'/'+label
    if os.path.lexists(base) or os.path.lexists(plist):raise RuntimeError('existing_core_owner')
    for check in (target,'gui/'+str(os.getuid())+'/'+label):
        r=subprocess.run(['/bin/launchctl','print',check],capture_output=True,timeout=5)
        if r.returncode==0:raise RuntimeError('existing_core_controller')
    receipt=Path(a.receipt)
    write_new(receipt.with_suffix('.intent.json'),json.dumps({'status':'intent','host':host,'source':a.source,'otherProductRestarts':0}).encode(),0o600)
    started=False;plist_created=False;child_pid=None;wrapper_pid=None
    try:
        base.mkdir(mode=0o700);(base/'run').mkdir(mode=0o700);(base/'releases').mkdir(mode=0o700)
        release=base/'releases'/a.source;release.mkdir(mode=0o700)
        write_new(release/'mesh-core-service',data,0o700)
        write_new(base/'mac-core-watchdog.py',wrapper,0o700)
        config={'source':a.source,'binarySha256':a.binary_sha,'observerSha256':a.observer_sha,'probeSha256':a.probe_sha,'host':host,'reserveGiB':a.reserve_gib}
        write_new(base/'config.json',json.dumps(config,indent=2).encode(),0o600)
        values={'Label':label,'ProgramArguments':[str(python),'-I','-S','-B',str(base/'mac-core-watchdog.py')],
                'WorkingDirectory':str(base),'RunAtLoad':True,'KeepAlive':False,'ProcessType':'Background','LimitLoadToSessionType':['Background'],
                'Umask':0o077,'StandardOutPath':'/dev/null','StandardErrorPath':'/dev/null',
                'SoftResourceLimits':{'NumberOfFiles':64},'HardResourceLimits':{'NumberOfFiles':64}}
        plist_data=plistlib.dumps(values);write_new(plist,plist_data,0o600)
        plist_info=plist.lstat();plist_generation=(plist_info.st_dev,plist_info.st_ino,plist_info.st_uid)
        plist_created=True
        subprocess.run(['/usr/bin/plutil','-lint',str(plist)],check=True,capture_output=True,timeout=5)
        started=True;subprocess.run(['/bin/launchctl','bootstrap',domain,str(plist)],check=True,capture_output=True,timeout=10)
        deadline=time.monotonic()+15;current=None
        while time.monotonic()<deadline:
            try:
                current=json.loads((base/'current.json').read_bytes())
                if current.get('ready'):break
            except (OSError,ValueError):pass
            time.sleep(.25)
        if not current or not current.get('ready'):raise RuntimeError('core_not_ready')
        child_pid=current['pid'];properties=subprocess.check_output(['/bin/launchctl','print',target],timeout=5).decode()
        match=re.search(r'^\s*pid = ([0-9]+)$',properties,re.M)
        if not match:raise RuntimeError('controller_not_ready')
        wrapper_pid=int(match[1])
        probe=fleet/'mac-codex-owner-probe'
        subprocess.run([str(probe),'--text',str(child_pid),str(release/'mesh-core-service')],check=True,capture_output=True,timeout=3)
        con=PrivateHTTP(base/'run/core.sock')
        try:
            con.request('POST','/v1/normalize',json.dumps({'local_host_id':host,'known_hosts':['server-44'],'hosts':'server-44','request_id':'owner-core-install','origin_host':host,'hop_count':0}),{'Content-Type':'application/json'})
            response=con.getresponse();body=response.read(1048577)
            if response.status!=200 or len(body)>1048576:raise RuntimeError('core_protocol')
            result=json.loads(body)
            if result.get('hosts')!=['server-44'] or result.get('request_id')!='owner-core-install':raise RuntimeError('core_route')
        finally:con.close()
        proof={'status':'installed','host':host,'source':a.source,'binarySha256':a.binary_sha,'watchdogSha256':a.watchdog_sha,
               'pid':child_pid,'watchdogPid':wrapper_pid,'socket':str(base/'run/core.sock'),'realNormalize':True,
               'enforcement':'darwin_observation_not_kernel_or_swap_reservation','otherProductRestarts':0,'fleetAdoption':False}
        write_new(receipt,json.dumps(proof,indent=2).encode(),0o600);print(json.dumps(proof))
    except BaseException as error:
        disposition={'status':'failed','errorClass':type(error).__name__,'rollback':'not_started','otherProductRestarts':0}
        try:
            if plist_created and not started:
                current_plist=plist.lstat()
                if (current_plist.st_dev,current_plist.st_ino,current_plist.st_uid)!=plist_generation or plist.read_bytes()!=plist_data:raise RuntimeError('controller_generation')
                plist.unlink()
                disposition['rollback']='owned_unloaded_plist_removed'
            if started:
                # Only the exact newly created label/plist. Preserve uncertain runtime evidence.
                current_plist=plist.lstat()
                if (current_plist.st_dev,current_plist.st_ino,current_plist.st_uid)!=plist_generation or plist.read_bytes()!=plist_data:raise RuntimeError('controller_generation')
                subprocess.run(['/bin/launchctl','bootout',target],check=True,capture_output=True,timeout=15)
                deadline=time.monotonic()+12
                while time.monotonic()<deadline:
                    try:last=json.loads((base/'last-run.json').read_bytes())
                    except (OSError,ValueError):last={}
                    if last.get('cleanupVerified'):break
                    time.sleep(.25)
                if not last.get('cleanupVerified'):raise RuntimeError('cleanup_unverified')
                current_plist=plist.lstat()
                if (current_plist.st_dev,current_plist.st_ino,current_plist.st_uid)!=plist_generation or plist.read_bytes()!=plist_data:raise RuntimeError('controller_generation')
                plist.unlink() # Only this new label's exact file, after confirmed cleanup.
                disposition['rollback']='owned_controller_and_plist_removed_child_cleanup_verified'
        except BaseException as cleanup:disposition['rollback']='unverified';disposition['cleanupErrorClass']=type(cleanup).__name__
        write_new(receipt,json.dumps(disposition,indent=2).encode(),0o600);print(json.dumps(disposition));raise

if __name__=='__main__':main()
