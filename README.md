# DDCP

Database-to-Database Copy (DDCP) over [Veilid](https://veilid.com).

No servers, no cloud. No gods, no masters.

# What is DDCP?

DDCP (Database-to-Database Copy) is two things:

- A Git-like CLI for [VLCN](https://vlcn.io/)'s CR-SQLite.
- A Rust-native networking layer, which coordinates VLCN database changes among remote peers.

Both operate over [Veilid](https://veilid.com) for simple, private, and secure p2p networking with strong cryptographic identities.

# Why would I use DDCP?

Rapid development of local-first distributed applications. Cut out the boilerplate and yak-shaving of APIs and focus on the data.

Convert traditional RDBMS apps into local-first without having to re-architect around low-level CRDTs.

# How do I use it?

DDCP is still in the early stages of development but is already pretty interesting. Here's what you can do with it:

## Initialize a SQLite database

```bash
ddcp init
```

This registers a Veilid DHT public key for your database.

```bash
2023-09-27T16:50:03.646448Z  INFO ddcp: Registered database at DHT key="VLD0:73iT7NuqplS2nab1AH4P7lJYiQROR_k4NyUhag_K4DY"
```

## Publish it

```bash
ddcp serve
```

```bash
2023-09-27T16:50:36.319921Z  INFO ddcp: Serving database at DHT key="VLD0:73iT7NuqplS2nab1AH4P7lJYiQROR_k4NyUhag_K4DY"
2023-09-27T16:50:36.321171Z  INFO ddcp: Database changed, updated status db_version=2 key="VLD0:73iT7NuqplS2nab1AH4P7lJYiQROR_k4NyUhag_K4DY"
```

This process must remain running in the background in order for remotes to be able to fetch changes from this database.

This process also prints the Veilid public key to give to your peers.

## Populate it

Create some VLCN [CRRs (Conflict-free Replicated Relations)](https://vlcn.io/docs/appendix/crr) in your database. This can be done while DDCP is running and publishing your database. Changes will be automatically picked up by remote subscribers.

```bash
ddcp shell < fixtures/cr_tables.sql
ddcp shell < fixtures/ins_test_schema_alice.sql
```

## Replicate it

Add remote peer databases to synchronize with:

```bash
ddcp remote add alice 73iT7NuqplS2nab1AH4P7lJYiQROR_k4NyUhag_K4DY
```

The `VLD0:` prefix is optional.

Changes from remotes are automatically applied to the local database while publishing changes.

```bash
ddcp serve
```

```bash
2023-09-27T17:01:30.615367Z  INFO ddcp: Pulled changes from remote database remote_name="alice" db_version=5
```

# VLCN Caveats

CRRs have certain restrictions in order to work as expected, or even at all:

- Changes in CRRs MUST happen after loading the crsqlite.so extension. Running `ddcp shell` does this automatically for you. Something to keep in mind when using cr-sqlite from other applications though.
- Peers have to share a common schema in order to replicate changes. VLCN supports schema migrations.
- Replication happens on primary keys. Avoid auto-incrementing primary keys, these will not merge well. Prefer strong content identifiers or large random UUID-like keys.

# Development

## OCI image

Probably the easiest way to evaluate DDCP. OCI image build is based on Debian Bookworm.

```bash
docker build -t ddcp .
```

Usage:

```bash
docker run -v $(pwd)/data:/data ddcp:latest init
docker run -v $(pwd)/data:/data ddcp:latest remote add alice 73iT7NuqplS2nab1AH4P7lJYiQROR_k4NyUhag_K4DY
cat fixtures/cr_test_schema.sql | docker run -v $(pwd)/data:/data -i ddcp:latest shell
docker run -v $(pwd)/data:/data ddcp:latest pull
```

## Nix

DDCP currently builds in a Nix flake devshell. If you Nix,

```bash
nix develop
cargo build
```

## Other platforms

This might probably work, with compilers and development libraries installed:

```bash
# Install serde tooling; used in cargo build (for now)
./scripts/install_capnproto.sh
./scripts/install_protoc.sh

cargo build --release
```

You can also try `cargo install ddcp` but the build will still require the serde tooling to be installed.

# Roadmap

DDCP is still a tech preview. Wire protocol, schemas, and APIs are all unstable and subject to breaking changes.

Several areas where DDCP still needs improvements:

### Private sharing

DHT addresses are currently publicly accessible; if you know the address, you can pull from the database. This is OK for public distribution of data sets (blogs, "everybody edits", stuff like that), but not for private apps.

Support sharing for authenticated DHTs.

### Changeset filtering and transformation

More control over how changes are merged. In some use cases, you want to merge all peers' changes in a consistent state. In others, you probably want to keep peers' content separate but linked. Primitives that support these different synchronization patterns. Filters & transformations on `crsql_changes`. Authn and authz (who can change or pull what).

### Large changesets

Overcome `app_call` message size limits. Currently there's no checks on this so sending large changesets (like pictures of cats) will likely fail in uncontrolled ways. Veilid messages are typically limited to 32k.

32k is probably enough for a row without large column data. Large column data should probably reference block storage, like PostgreSQL's Toast.

### Missing Veilid features

Replace polling with a watch on DHT changes (currently [not implemented](https://gitlab.com/veilid/veilid/-/blob/bd4b4233bfed5bdca4da3cacda3ad960e28daab5/veilid-core/src/storage_manager/mod.rs#L485) AFAICT).

Veilid blockstore integration when it's ready.

### Some apps

Ideas for kinds of data that could be shared:

- Distributed BBS
- Bookmarks & link sharing
- Network scanning results
- Remote sensor aggregation
- Code hosting to get this project off Git\*\*b

# About

Name takes inspiration from [UUCP](https://en.wikipedia.org/wiki/UUCP) and [NNCP](https://www.complete.org/nncp/).

Some of the Veilid core integration was derived from examples in [vldpipe](https://gitlab.com/vatueil/vldpipe).
