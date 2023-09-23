# DDCP

Database-to-Database Copy.

# What is it?

Multi-master database synchronization with [VLCN](https://vlcn.io)'s [CR-SQLite](https://vlcn.io/docs/cr-sqlite/intro) over [Veilid](https://veilid.com).

DDCP currently provides a Git-like CLI for fetching and merging SQLite database changes over Veilid.

# Why would I use it?

Distribute a SQLite database and synchronize it like a Git repository.

# How do I use it?

```bash
ddcp serve
```

This process must remain running in the background, in order for remotes to be able to fetch changes from this database.

On first run, an empty database is created and a Veilid DHT address registered for synchronization.

```
VLD0:LPrSd0aQgiOlTyDcdajBDKo19ge66zViznaSUmt-Bhw
```

`ddcp serve` always prints this address, which can be shared with other peers.

## Remotes

Add a remote peers to synchronize with:

```bash
ddcp remote add alice VLD0:XB2wAFtxvj3u2CxON099uMw0HdRiQltc-SkFaXc49hU
```

List remote databases:

```bash
ddcp remote list
```

```
alice VLD0:XB2wAFtxvj3u2CxON099uMw0HdRiQltc-SkFaXc49hU
```

Remove a remote database:

```bash
ddcp remote remove alice
```

## Fetch remote changes

Fetching changes from remotes downloads the changes and stores them locally, but does not apply them to the local database state.

In order for database changes to propagate, the database must have the crsqlite extension loaded, and the tables must have been updated to a Conflict-free Replicated Relation (CRR). See VLCN documentation:

- [Loading the extension](https://vlcn.io/docs/cr-sqlite/installation#loading-the-extension)
- [crsql_as_crr](https://vlcn.io/docs/cr-sqlite/api-methods/crsql_as_crr)

Note that these changes can be made in a separate process in your language of choice, so long that the extension has been loaded and the tables upgraded for replication.

Fetch changes from all remotes:

```bash
ddcp fetch
```

Fetch from a specific remote:

```bash
ddcp fetch peer3
```

## Merge remote changes

Merge remote changes fetched from a peer:

```bash
ddcp merge peer1
```

## Pull remote changes

Like git, a `pull` is a `fetch` followed by a `merge`:

```bash
ddcp pull
```

## Automatic synchronization

Serve database fetch requests. (Coming soon: pull from remotes automatically)

```
ddcp serve
```

# How do I build it?

DDCP currently builds in a Nix flake devshell. If you Nix,

```bash
nix develop
cargo build
```

Build scripts supporting a Debian-based OCI image build is planned.

## Packaging & release engineering plans

`cargo install ddcp`

`podman run -v ./data:/data somecr.io/ddcp serve`

`nix-shell -p ddcp`

# How do I develop an application with it?

More on this to come, but for now:

Run `ddcp serve` in one process.

Develop and run VLCN / cr-sqlite based applications in another, using the same database file.

# TODO

Automatic fetching and merging in `ddcp serve`.

Overcome `app_call` message size limits. Currently there's no checks on this so sending large changesets (like pictures of cats) will likely fail in uncontrolled ways.

Library so you don't have to operate the separate process yourself.

Improve organization of db & Veilid node files.

More control over how changes are merged. In some use cases, you want to merge all peers' changes in a consistent state. In others, you probably want to keep peers' content separate but linked. Primitives that support these different synchronization patterns.

Veilid blockstore integration when it's ready.

Some apps. Ideas for kinds of data that could be shared:

- Distributed BBS
- Bookmarks & link sharing
- Network scanning results
- Code hosting to get this project off Git\*\*b

Control via localhost or unix socket.

## What's the thinking behind DDCP?

Why do we need APIs, when we can share data directly?

In a local-first world, what even is a "client" or "server"?

There is only synchronization of my state, your state, their state. Make what you will of that state. Merge, copy, reference, or ignore parts or all of it as you like.

Our applications should enable this kind of autonomy and optionality. User agents that work for us, not someone else's rent-seeking business model.

