# SQLite Database Setup Spec

## Purpose

- Describe the SQLite setup used by `crates/simrec/src/rec-sqlite.rs` for the 200 Hz sensor capture.
- Goal: sustained inserts with low CPU use and a compact on-disk footprint.
- Scope: setup and write path only. Query benchmarks are deferred.

## Pragmas

- `journal_mode = WAL`: commits append to the `-wal` file instead of rewriting pages in the main file, and readers do not block the writer.
- `synchronous = NORMAL`: fsync happens at checkpoints instead of every commit. The database stays consistent, but the latest commits can be lost on power failure.
- Both values are read back after being set. Startup fails if they do not match.

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
```

### Synchronous Options

- `OFF` (0): no fsync at all. Fastest, but an OS crash or power loss can corrupt the database.
- `NORMAL` (1), used here: in WAL mode, fsync happens only at checkpoints. The database stays consistent, but the latest commits can be lost on power failure.
- `FULL` (2): fsync on every commit. No committed batch is lost, at the cost of one sync per batch.
- `EXTRA` (3): like `FULL`, plus a directory sync when journal files are created or deleted. Most durable and slowest, rarely needed with WAL.

With one commit per batch every 100 ms, `FULL` means up to about 10 fsyncs per second. The real cost depends on the storage device and should be measured.

### Checkpoints

- A checkpoint copies WAL pages back into the main database file. With `synchronous = NORMAL`, this is also where the fsync happens.
- By default, SQLite checkpoints automatically when the WAL reaches 1000 pages, about 4 MB with the default 4096-byte page size.
- The automatic checkpoint is size-based only. SQLite has no time-based automatic checkpoint, so a time interval needs explicit calls.
- The current code does not set this, so the default applies. The commit that crosses the threshold absorbs the checkpoint latency.
- To control it, disable the automatic checkpoint and run checkpoints from the recorder on a schedule, or keep the automatic one and tune its page count.
- `PRAGMA wal_checkpoint(PASSIVE)` is not the default. It is a manual call the recorder must issue, and it is not implemented yet.

```sql
PRAGMA wal_autocheckpoint = 1000; -- pages, 0 disables
PRAGMA wal_checkpoint(PASSIVE);
PRAGMA journal_size_limit = 67108864; -- bytes, caps WAL size after reset
```

## Schema

- `ts_epoch_micro INTEGER PRIMARY KEY` is the rowid alias, so the table is a B-tree clustered by time.
- Ticks arrive in increasing timestamp order, so each insert appends to the last leaf page.
- Channels are `REAL` (8-byte double). The `f32` values are widened to `f64` on insert.

```sql
CREATE TABLE samples (
    ts_epoch_micro INTEGER PRIMARY KEY,
    rotor_fl REAL,
    rotor_fr REAL,
    rotor_rl REAL,
    rotor_rr REAL,
    accel_z REAL
);
```

## Batched Transactions

- Each nonempty consumer batch is one transaction. The batch is flushed every 100 ms by default, or at 8192 ticks.
- One commit per batch amortizes WAL write and locking cost. Per-tick commits are not allowed.
- A failed batch rolls back as a whole, so no partial batch is stored.
- The insert statement is prepared once per connection with `prepare_cached` and bound per tick.

```sql
BEGIN;
INSERT INTO samples (ts_epoch_micro, rotor_fl, rotor_fr, rotor_rl, rotor_rr, accel_z)
VALUES (?1, ?2, ?3, ?4, ?5, ?6);
-- repeated for each tick in the batch, same prepared statement
COMMIT;
```

## Secondary Indexes

- Opt-in through `--with-index`, which selects the `sqlite-all-index` variant.
- The default `sqlite-pk-index` variant has only the primary key.
- Each index adds one B-tree write per insert. The cost is measured by comparing both variants.
- The current code creates the indexes before loading data. Building them after the bulk load is faster but is not implemented.

```sql
CREATE INDEX idx_samples_rotor_fl ON samples (rotor_fl);
CREATE INDEX idx_samples_rotor_fr ON samples (rotor_fr);
CREATE INDEX idx_samples_rotor_rl ON samples (rotor_rl);
CREATE INDEX idx_samples_rotor_rr ON samples (rotor_rr);
CREATE INDEX idx_samples_accel_z ON samples (accel_z);
```

## Connection Lifecycle

- The database file is created with `create_new`, so an existing database is never overwritten.
- One writer connection is used per database.
- Finalization closes the connection, then counts the database file plus any remaining `-wal` and `-shm` files.
- Values are always bound as parameters, never formatted into SQL strings.

## Known Limits

- Not crash-safe. `synchronous = NORMAL` can lose the latest commits after power failure. See `dev/spec-overall.md`.
