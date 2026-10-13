#!/usr/bin/env python3
"""One native Rust child under the existing sealed fleet's Darwin observer.

Darwin observations are not kernel reservations or a swap-zero guarantee.
No product auth, sessions, model, native Codex controller or network listener.
"""
import fcntl, hashlib, json, os, re, resource, signal, stat, subprocess, sys, time, types
from pathlib import Path

BASE = Path.home()/'.local/share/mesh-core'
MIB = 1024**2

def sealed(path, expected, maximum):
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    try:
        before=os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_uid!=os.getuid() or before.st_mode&0o022:
            raise RuntimeError('input_owner')
        with os.fdopen(fd,'rb',closefd=False) as stream:data=stream.read(maximum+1)
        after=os.fstat(fd)
        if len(data)>maximum or (before.st_dev,before.st_ino,before.st_mtime_ns,before.st_ctime_ns,before.st_size)!=(after.st_dev,after.st_ino,after.st_mtime_ns,after.st_ctime_ns,after.st_size):
            raise RuntimeError('input_generation')
    finally:os.close(fd)
    if expected is not None and hashlib.sha256(data).hexdigest()!=expected:raise RuntimeError('input_hash')
    return data

def private(path):
    info=path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid!=os.getuid() or stat.S_IMODE(info.st_mode)!=0o700 or path.resolve()!=path:
        raise RuntimeError('private_directory')

def limits(sample, soft_since, now):
    failed=[]
    for name,bad in [('memory_hard',sample['rss']>64*MIB),
                     ('memory_soft',soft_since is not None and now-soft_since>=30),
                     ('tasks',sample['tasks']>16),('processes',sample['processes']>16),
                     ('cpu_peak',sample['cpu']>100),('cpu_sustained',sample['rolling_cpu']>25),
                     ('host_memory',sample['available']<sample['reserve']+64*MIB),
                     ('host_disk',sample['disk']<2*1024**3)]:
        if bad:failed.append(name)
    return failed

def child_limits():
    resource.setrlimit(resource.RLIMIT_NOFILE,(64,64))
    resource.setrlimit(resource.RLIMIT_CORE,(0,0))
    resource.setrlimit(resource.RLIMIT_FSIZE,(MIB,MIB))

def final_state(observer, child, observed, sock, socket_identity, record, base):
    write_error=None;cleanup_error=None;cleanup=False
    try:observer.atomic_json(base/'current.json',{'ready':False,'reason':record['reason']})
    except BaseException as error:write_error=type(error).__name__
    try:
        if child is not None:observer.stop_owned(child,observed,timeout=10)
        cleanup=True
        if socket_identity and os.path.lexists(sock):
            meta=sock.lstat()
            if stat.S_ISSOCK(meta.st_mode) and (meta.st_dev,meta.st_ino,meta.st_uid)==socket_identity:sock.unlink()
            else:cleanup=False
    except BaseException as error:cleanup_error=type(error).__name__;cleanup=False
    try:
        observer.atomic_json(base/'last-run.json',{**record,'cleanupVerified':cleanup,
                             'cleanupErrorClass':cleanup_error,'stateWriteErrorClass':write_error})
    finally:
        if not cleanup:raise RuntimeError('cleanup_unverified')

def main():
    if sys.platform!='darwin':raise RuntimeError('native_darwin_required')
    private(BASE);private(BASE/'run')
    config_path=BASE/'config.json'
    info=config_path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=os.getuid() or info.st_mode&0o077 or info.st_size>16384:
        raise RuntimeError('config_owner')
    config=json.loads(sealed(config_path,None,16384))
    if not re.fullmatch('[0-9a-f]{40}',config['source']):raise RuntimeError('source_identity')
    if config['host']!=subprocess.check_output(['/usr/sbin/scutil','--get','LocalHostName'],timeout=3).decode().strip():
        raise RuntimeError('host_identity')
    if config['reserveGiB'] not in (2,4):raise RuntimeError('host_policy')
    helper=Path.home()/'.local/share/agent-herder/fleet/mac-node-watchdog.py'
    probe=helper.with_name('mac-codex-owner-probe')
    data=sealed(helper,config['observerSha256'],128*1024)
    sealed(probe,config['probeSha256'],MIB)
    observer=types.ModuleType('mesh_darwin_observer');observer.__file__=str(helper)
    sys.modules[observer.__name__]=observer
    exec(compile(data,str(helper),'exec'),observer.__dict__)
    binary=BASE/'releases'/config['source']/'mesh-core-service'
    sealed(binary,config['binarySha256'],16*MIB)
    sock=BASE/'run/core.sock'
    if os.path.lexists(sock):raise RuntimeError('existing_socket')
    lock_fd=os.open(BASE/'owner.lock',os.O_CREAT|os.O_NOFOLLOW|os.O_RDWR,0o600)
    lock=os.fstat(lock_fd)
    if not stat.S_ISREG(lock.st_mode) or lock.st_uid!=os.getuid() or lock.st_mode&0o077:raise RuntimeError('lock_owner')
    fcntl.flock(lock_fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
    stopping=False
    def stop(*_):
        nonlocal stopping
        stopping=True
    signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
    if observer.host_available_kib()*1024<config['reserveGiB']*1024**3+64*MIB:
        raise RuntimeError('host_reserve')
    child=None;observed=[];reason='startup';started=time.monotonic();soft=None;cpu_samples=[];previous={};previous_at=started
    socket_identity=None;max_rss=0;count=0;cleanup=False
    try:
        child=subprocess.Popen([str(binary),'--socket',str(sock)],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,
                               start_new_session=True,preexec_fn=child_limits,env={'PATH':'/usr/bin:/bin','HOME':str(Path.home()),'TMPDIR':str(BASE/'run')})
        deadline=time.monotonic()+5
        while not os.path.lexists(sock):
            if child.poll() is not None or time.monotonic()>=deadline:raise RuntimeError('startup_deadline')
            time.sleep(.05)
        meta=sock.lstat()
        if not stat.S_ISSOCK(meta.st_mode) or meta.st_uid!=os.getuid() or stat.S_IMODE(meta.st_mode)!=0o600:raise RuntimeError('socket_owner')
        socket_identity=(meta.st_dev,meta.st_ino,meta.st_uid)
        observed=observer.read_processes(child.pid);pinned={r['pid']:r['generation'] for r in observed}
        if child.pid not in pinned:raise RuntimeError('child_generation')
        while child.poll() is None and not stopping:
            now=time.monotonic()
            sealed(probe,config['probeSha256'],MIB)
            rows=observer.read_processes(child.pid)
            if not rows or any(r['pid'] not in pinned or r['generation']!=pinned[r['pid']] for r in rows):raise RuntimeError('group_generation')
            observed=rows
            current={r['pid']:r['cpu_seconds'] for r in rows}
            cpu=observer.cpu_percent(previous,current,now-previous_at) if previous else 0
            if previous:cpu_samples.append((previous_at,now,cpu))
            cpu_samples=[r for r in cpu_samples if r[1]>now-30]
            rolling=sum(max(0,min(end,now)-max(begin,now-30))*value for begin,end,value in cpu_samples)/30
            previous=current;previous_at=now
            rss=sum(r['rss_kib'] for r in rows)*1024;max_rss=max(max_rss,rss)
            soft=(now if soft is None else soft) if rss>32*MIB else None
            disk=os.statvfs(BASE)
            sample={'rss':rss,'tasks':observer.read_threads(rows),'processes':len(rows),'cpu':cpu if now-started>10 else 0,
                    'rolling_cpu':rolling if now-started>30 else 0,'available':observer.host_available_kib()*1024,
                    'reserve':config['reserveGiB']*1024**3,'disk':disk.f_bavail*disk.f_frsize}
            failed=limits(sample,soft,now)
            if failed:raise RuntimeError(','.join(failed))
            count+=1
            observer.atomic_json(BASE/'current.json',{'ready':True,'pid':child.pid,'source':config['source'],'socket':str(sock),'rssBytes':rss,'tasks':sample['tasks'],'samples':count})
            for _ in range(10):
                if stopping:break
                time.sleep(.5)
        reason='operator_stop' if stopping else 'child_exit'
    except BaseException as error:
        reason=type(error).__name__
        if isinstance(error,RuntimeError):reason=str(error)[:120]
        raise
    finally:
        final_state(observer,child,observed,sock,socket_identity,
                    {'reason':reason,'source':config['source'],'peakRssBytes':max_rss,'samples':count,'elapsedSeconds':round(time.monotonic()-started,3)},BASE)

if __name__=='__main__':main()
