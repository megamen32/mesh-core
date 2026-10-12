package meshcore

import (
 "context"
 "errors"
 "os"
 "os/exec"
 "path/filepath"
 "testing"
 "time"
 "syscall"
)

// Real optional executable integration, never a fake fleet acceptance claim.
func TestRealCoreSingleton(t *testing.T) {
 binary:=os.Getenv("MESH_CORE_TEST_BINARY")
 if binary=="" { t.Skip("real Rust binary not supplied") }
 directory,err:=os.MkdirTemp(os.Getenv("TMPDIR"),"g");if err!=nil{t.Fatal(err)}
 defer os.RemoveAll(directory)
 socket:=filepath.Join(directory,"s")
 if len(socket)>=104{t.Fatal("private socket path too long")}
 command:=exec.Command(binary,"--socket",socket)
 if err=command.Start();err!=nil{t.Fatal(err)}
 done:=make(chan error,1)
 go func(){done<-command.Wait()}()
 stopped:=false
 var stopError error
 stop:=func()error {
  if stopped{return stopError}
  stopped=true
  select {case stopError=<-done:return stopError;default:}
  command.Process.Signal(syscall.SIGTERM)
  select {
  case stopError=<-done:return stopError
  case <-time.After(2*time.Second):
   command.Process.Kill()
   select {case <-done:stopError=errors.New("core required forced cleanup");case <-time.After(2*time.Second):stopError=errors.New("core cleanup unverified")}
   return stopError
  }
 }
 defer func(){if err:=stop();err!=nil{t.Error(err)}}()
 for i:=0;i<40;i++ {if _,err=os.Lstat(socket);err==nil{break};time.Sleep(25*time.Millisecond)}
 client,err:=New(socket);if err!=nil{t.Fatal(err)}
 requestID:="go-exact-id"
 route,err:=client.Normalize(context.Background(),RouteInput{LocalHostID:"a",Hosts:"local",RequestID:&requestID})
 if err!=nil{t.Fatal(err)}
 if route.RequestID!=requestID||len(route.Hosts)!=1||route.Hosts[0]!="a"{t.Fatal("nil-peer local route contract lost")}
 if _,err=client.Normalize(context.Background(),RouteInput{LocalHostID:"other",Hosts:"a"});err==nil{t.Fatal("topology leaked across calls")}
 if err=stop();err!=nil{t.Fatal(err)}
 if _,err=os.Lstat(socket);!os.IsNotExist(err){t.Fatal("owned socket not removed on stop")}
}
