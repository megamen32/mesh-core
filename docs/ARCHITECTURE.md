# Shared implementation and boundaries

The original modules come from GrepMesh; EXTRACTION.json records the source
commit and MIT provenance. Public module names remain compatible through reexports.

Rust applications link the library. Other applications use one same-user Rust
process over a private Unix socket, not a process or stdio MCP for each client.
`mesh-core-service --socket /absolute/private/directory/core.sock` requires an
existing canonical directory with mode0700. It creates mode0600 socket, checks
peer UID, refuses an existing path and removes only its own inode on SIGTERM/SIGINT.
The owner UID and an existing privileged local root product process may use the
private socket; other UIDs are rejected. Root already has filesystem authority
over this directory. This adds no public endpoint, token, profile or tool grant.
The service has eight active connections, three-second total request deadline,
8KiB headers,128KiB input,1MiB output and one Tokio worker. Deployment must enforce
finite RAM/CPU/tasks/storage externally; a healthy socket is not fleet acceptance.

`POST /v1/normalize` calls the extracted host-selection function with local_host_id,
known_hosts, hosts, request_id, origin_host and hop_count. `POST /v1/topology` validates
an existing snapshot and computes freshness at the supplied now_ms. No registry,
credential or session database is created. The authenticated application supplies
only authorized nodes; user request bodies must never supply that authority.
These endpoints are stateless: no profile, token, header, mutable session or request
ID is retained or shared. Go and Node clients serialize each request independently.
They fail closed on an unavailable core and never retry a mutation.

The optional `transport` feature preserves the original GrepMesh direct HTTP and
approved loopback CONNECT transport. Its caller supplies the already-scoped Client,
borrowed bearer and limits. It does not create a global credential-bearing Client.
Fallback remains limited to a direct connect failure before dispatch; timeout or
HTTP failure must not trigger a second business action. Legacy proxy framing and
MCP JSON-only decoding remain explicit compatibility limitations of this extraction.
They are not proof of general HTTP/SSE client compatibility or profile pooling.

## Acceptance status

- Local extracted topology/cache/normalizer:14 tests passed in a finite source run.
- Shared transport and singleton:18 Rust tests passed; a real Node client verified the compiled service, IDs, unknown-host rejection and stop/restart on the same socket. A real Go client also passed against the compiled singleton, including nil-peer local routing and owned cleanup. Deployment into GPTAdmin/Herder is not yet accepted.
- GrepMesh reexports: source wired, release build and real consumer pending.
- GPTAdmin and Agent Herder live adoption: pending authorized integration.
- Full mutual mesh/failover and original Codex/ZCode relay acceptance: pending.

Test doubles establish local logic only. Release requires the actual installed
source and harmless real GrepMesh, GPTAdmin and Agent Herder calls, including
unavailable peer, profile/locality fences and bounded teardown. Existing accepted
or unknown actions must not be replayed to obtain this evidence.
