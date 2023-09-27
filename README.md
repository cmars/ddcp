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
2023-09-26T20:47:29.264567Z  INFO ddcp: VLD0:5bIixJHajo5GmeKnGIM-S8QlKbabIukCFWa-ihV8xqk
```

## Publish it

```bash
ddcp serve
```

This process must remain running in the background in order for remotes to be able to fetch changes from this database.

This process also prints the Veilid public key to give to your peers.

## Populate it

Create some VLCN [CRRs (Conflict-free Replicated Relations)](https://vlcn.io/docs/appendix/crr) in your database. This can be done while DDCP is publishing your database. Changes will be automatically picked up by remote subscribers.

```bash
ddcp shell < cr_tables.sql
```

## Replicate it

Add remote peer databases to synchronize with:

```bash
ddcp remote add alice VLD0:gO-fZJd0Zp5KxC-48J_BYE4vOmCyXJkLlH97uZbgBJg
```

The `VLD0:` prefix is optional.

Give other peers your DHT key, so they can replicate your database.

Note that all peers need to start with a common database schema in order to replicate it.

# VLCN Caveats

CRRs have certain restrictions in order to work as expected, or even at all:

- Changes in CRRs MUST happen after loading the crsqlite.so extension. Running `ddcp shell` does this automatically for you. Something to keep in mind when using cr-sqlite from other applications though.
- Peers have to share a common schema in order to replicate changes. VLCN supports schema migrations.
- Replication happens on primary keys. Avoid auto-incrementing primary keys, these will not merge well. Prefer strong content identifiers or large random UUID-like keys.

# Development

## OCI image

Probably the easiest way right now to evaluate DDCP. OCI image build is based on Debian Bookworm.

```bash
docker build -t ddcp .
```

Usage:

```bash
docker run -it -v $(pwd)/data:/data ddcp:latest serve
```

## Nix

DDCP currently builds in a Nix flake devshell. If you Nix,

```bash
nix develop
cargo build
```

## Other platforms

DDCP development requires Rust nightly because CR-SQLite requires nightly. DDCP [builds CR-SQLite](https://github.com/vlcn-io/cr-sqlite#building) from source in order to simplify distribution.

This might probably work:

```bash
git clone --recurse-submodules https://gitlab.com/cmars232/ddcp 
cd ddcp/external/veilid
# Set up Veilid development environment per instructions there...
cd ../..
cargo build
```

# Roadmap

DDCP is not yet stable and is still being actively worked on. Wire protocol, schemas, and APIs are all unstable and subject to breaking changes.

Several areas where DDCP still needs improvements:

### Large changesets

Overcome `app_call` message size limits. Currently there's no checks on this so sending large changesets (like pictures of cats) will likely fail in uncontrolled ways. Veilid messages are typically limited to 32k.

32k is probably enough for a row without large column data. Large column data should probably reference block storage, like PostgreSQL's Toast.

### Changeset filtering and transformation

More control over how changes are merged. In some use cases, you want to merge all peers' changes in a consistent state. In others, you probably want to keep peers' content separate but linked. Primitives that support these different synchronization patterns. Filters & transformations on `crsql_changes`. Authn and authz (who can change or pull what).

### Missing Veilid features

Replace polling with a watch on DHT changes (currently [not implemented](https://gitlab.com/veilid/veilid/-/blob/bd4b4233bfed5bdca4da3cacda3ad960e28daab5/veilid-core/src/storage_manager/mod.rs#L485) AFAICT).
Veilid blockstore integration when it's ready.

### Some apps

Ideas for kinds of data that could be shared:

- Distributed BBS
- Bookmarks & link sharing
- Network scanning results
- Code hosting to get this project off Git\*\*b
