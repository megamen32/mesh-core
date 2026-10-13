# macOS singleton

Use the native `mesh-core-service` asset for the host architecture from an exact
public release. Verify the archive checksum, `release.json` source closure and
binary checksum before installation. Never copy another host's credentials.

`deploy/install-macos.py` installs one owner-only Background LaunchAgent and one
Rust process at `~/.local/share/mesh-core/run/core.sock`. An existing directory,
socket or label refuses installation. Its real acceptance calls `/v1/normalize`
on the installed executable; this alone does not prove fleet adoption/failover.

The small controller reuses the already installed, SHA-sealed Herder fleet
read-only libproc observer and ownership-aware teardown. It does not start a
Herder, Codex or MCP worker. An observer update requires updating the core's
declared hashes before its next activation; it never silently adopts changed
code. The Rust binary source and observer source are recorded separately.

The child has 64 file descriptors and a 1 MiB output-file ceiling. The native
watchguard observes 32/64 MiB RSS (soft sustained 30 seconds), CPU 25% sustained
over 30 seconds / 100% peak after startup grace, at most 16 tasks/processes, host
reserve Mini 2 GiB or M1 4 GiB plus the full 64 MiB allowance, and 2 GiB free disk.
The Rust executable has no child process creation or persistent payload storage;
the sealed release is at most 16 MiB and diagnostics stay bounded. The Python
supervisor and transient read-only observations are additional host overhead.
Darwin monitoring is observational, not a kernel memory reservation or swap-zero
guarantee. A breached/unknown observation stops only its owned child and exits;
KeepAlive is false and there is no hidden retry/restart or fallback.

Installation saves an exclusive intent and result. Failed installation removes
only the exact new label/plist after verified child cleanup. Unknown cleanup
retains evidence and refuses further activation. Successful core service install
does not change product auth, profile, journal, sessions or routing targets.

Product adoption is explicit: Herder uses `AGENT_HERDER_MESH_CORE_SOCKET`,
GPTAdmin uses its existing configured core client, and GrepMesh pins the common
Rust library. Statically linked GrepMesh needs a rebuilt product for new library
code; publishing a common-core commit does not change running binaries.
