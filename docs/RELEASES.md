# Native singleton assets

The public `Native singleton release` workflow builds Linux x86_64, Intel macOS,
and Apple Silicon macOS with the pinned Rust compiler. Each platform must pass
the real executable/Node-client socket, routing and teardown checks before its
asset is published. Failed or timed-out jobs do not produce a release.

Each archive contains only `mesh-core-service` and `release.json`, including the
source commit, exact Rust source hashes and executable SHA-256. `SHA256SUMS`
covers all three archives. Fleet configuration, credentials and application
state are never packaging inputs. The workflow needs no repository secret;
only its final publish job uses the built-in repository token.

Dispatch the workflow on `main`. Assets use a source-specific tag
`v0.1.0-<12-character-commit>`. Verify the manifest and checksums before the
existing platform installer promotes a binary. An asset release does not
restart any fleet process or prove fleet-wide adoption or failover.
