// Package meshcore calls the same-user private Rust singleton.
// Authorization and discovery remain the authenticated product edge's job.
package meshcore

import (
 "bytes"
 "context"
 "encoding/json"
 "errors"
 "io"
 "net"
 "net/http"
 "path/filepath"
 "time"
)

type Client struct { http *http.Client }
type RouteInput struct {
 LocalHostID string `json:"local_host_id"`
 KnownHosts []string `json:"known_hosts"`
 Hosts interface{} `json:"hosts,omitempty"`
 RequestID *string `json:"request_id,omitempty"`
 OriginHost *string `json:"origin_host,omitempty"`
 HopCount *uint8 `json:"hop_count,omitempty"`
}
type Route struct {
 RequestID string `json:"request_id"`
 OriginHost string `json:"origin_host"`
 HopCount uint8 `json:"hop_count"`
 Hosts []string `json:"hosts"`
}
func New(socket string) (*Client, error) {
 if !filepath.IsAbs(socket) { return nil, errors.New("absolute core socket required") }
 transport := &http.Transport{DisableKeepAlives:true, MaxConnsPerHost:8,
  DialContext:func(ctx context.Context, _, _ string)(net.Conn,error){ return (&net.Dialer{Timeout:3*time.Second}).DialContext(ctx,"unix",socket) }}
 return &Client{http:&http.Client{Transport:transport,Timeout:3*time.Second,
  CheckRedirect:func(_ *http.Request,_ []*http.Request)error{return http.ErrUseLastResponse}}},nil
}
func(c *Client) Normalize(ctx context.Context, input RouteInput)(Route,error) {
 var result Route
 if input.KnownHosts==nil { input.KnownHosts=[]string{} }
 // Serialize before I/O; credentials and mutable caller state never enter the core.
 body,err:=json.Marshal(input); if err!=nil{return result,errors.New("invalid core input")}
 if len(body)>128*1024{return result,errors.New("bounded core request exceeded")}
 request,err:=http.NewRequestWithContext(ctx,http.MethodPost,"http://mesh-core/v1/normalize",bytes.NewReader(body));if err!=nil{return result,err}
 request.Header.Set("Content-Type","application/json")
 response,err:=c.http.Do(request);if err!=nil{return result,errors.New("core unavailable")};defer response.Body.Close()
 data,err:=io.ReadAll(io.LimitReader(response.Body,1024*1024+1));if err!=nil{return result,errors.New("core response incomplete")}
 if len(data)>1024*1024{return result,errors.New("bounded core response exceeded")}
 if response.StatusCode!=http.StatusOK{return result,errors.New("core request rejected")}
 if json.Unmarshal(data,&result)!=nil{return result,errors.New("invalid core response")};return result,nil
}
