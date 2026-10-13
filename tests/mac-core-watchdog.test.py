"""Fast units: sealed inputs and finite policy only, no runtime claim (max5s)."""
import hashlib, importlib.util, os, tempfile, types, unittest
from pathlib import Path
path=Path(__file__).resolve().parents[1]/'deploy/mac-core-watchdog.py'
spec=importlib.util.spec_from_file_location('core_guard',path);guard=importlib.util.module_from_spec(spec);spec.loader.exec_module(guard)

class GuardPolicy(unittest.TestCase):
    def sample(self):
        return {'rss':32*guard.MIB,'tasks':16,'processes':1,'cpu':10,'rolling_cpu':10,
                'available':5*1024**3,'reserve':4*1024**3,'disk':3*1024**3}
    def test_exact_policy_boundary_and_soft_window(self):
        sample=self.sample();self.assertEqual(guard.limits(sample,None,30),[])
        self.assertIn('memory_soft',guard.limits(sample,0,30))
        sample['rss']=64*guard.MIB+1;self.assertIn('memory_hard',guard.limits(sample,None,0))
    def test_cpu_tasks_and_full_host_reserve(self):
        sample=self.sample();sample.update(tasks=17,cpu=101,rolling_cpu=26,available=4*1024**3+63*guard.MIB)
        self.assertEqual(set(guard.limits(sample,None,0)),{'tasks','cpu_peak','cpu_sustained','host_memory'})
    def test_sealed_hash_owner_and_bound(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Path(folder)/'payload';p.write_bytes(b'verified');p.chmod(0o600)
            digest=hashlib.sha256(b'verified').hexdigest()
            self.assertEqual(guard.sealed(p,digest,8),b'verified')
            with self.assertRaises(RuntimeError):guard.sealed(p,'0'*64,8)
            with self.assertRaises(RuntimeError):guard.sealed(p,digest,7)
            p.chmod(0o666)
            with self.assertRaises(RuntimeError):guard.sealed(p,digest,8)
    def test_symlink_is_never_adopted(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Path(folder)/'payload';p.write_bytes(b'x');p.chmod(0o600)
            link=Path(folder)/'alias';link.symlink_to(p)
            with self.assertRaises(OSError):guard.sealed(link,hashlib.sha256(b'x').hexdigest(),1)
    def test_private_directory_mode(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Path(folder);guard.private(p);p.chmod(0o755)
            with self.assertRaises(RuntimeError):guard.private(p)
    def test_state_write_failure_cannot_skip_child_cleanup(self):
        calls=[]
        def write(path,value):
            if path.name=='current.json':raise OSError('unit fixture disk error')
            calls.append(value)
        observer=types.SimpleNamespace(atomic_json=write,stop_owned=lambda *a,**kw:calls.append('stop'))
        guard.final_state(observer,object(),[],Path('/unused'),None,{'reason':'unit'},Path('/unused'))
        self.assertEqual(calls[0],'stop');self.assertTrue(calls[1]['cleanupVerified'])
        self.assertEqual(calls[1]['stateWriteErrorClass'],'OSError')
    def test_unknown_cleanup_never_reports_success(self):
        calls=[]
        def stop(*a,**kw):raise RuntimeError('unit fixture cleanup')
        observer=types.SimpleNamespace(atomic_json=lambda path,value:calls.append(value),stop_owned=stop)
        with self.assertRaisesRegex(RuntimeError,'cleanup_unverified'):
            guard.final_state(observer,object(),[],Path('/unused'),None,{'reason':'unit'},Path('/unused'))
        self.assertFalse(calls[-1]['cleanupVerified']);self.assertEqual(calls[-1]['cleanupErrorClass'],'RuntimeError')

if __name__=='__main__':unittest.main()
