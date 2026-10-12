# Mesh Core

Shared Rust topology, routing and peer transport extracted from GrepMesh.

- Keeps one implementation of host selection and the one-hop loop fence.
- Reuses topology validation, freshness and atomic cache persistence.
- Provides the existing GrepMesh HTTP peer transport as an optional feature.
- Offers a private singleton API and small Go and Node clients for non-Rust applications.

Add the library to a Rust application:

```sh
cargo add mesh-core --git https://github.com/megamen32/mesh-core.git
```

This is an extraction in progress. Fleet-wide adoption and failover are not
accepted yet. Applications retain authentication, tool names and business state.
See [architecture and adoption status](docs/ARCHITECTURE.md).
