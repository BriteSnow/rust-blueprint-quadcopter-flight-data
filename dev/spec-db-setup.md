# SQLite Database Setup Spec

## Purpose

- Describe the SQLite setup used by `crates/lib-db/` for the 200 Hz sensor capture and its 2 Hz summary.
- Goal: sustained inserts with low CPU use and a compact on-disk footprint.
- Scope: handles, setup, and write path only. Query benchmarks are deferred.

## Handles

- `DbRw` is the one-thread read-write handle: `DbRw::create_new(path)` creates the file, opens it, and sets the pragmas. `close()` ends the connection.
- `DbRo` is the one-thread read-only handle: `DbRo::open(path)` sets `query_only=ON` and a 1000 ms `busy_timeout`, so a reader cannot write and never blocks the writer for long.
- Only `DbRw::create_new` sets `journal_mode` and `synchronous`. Both values are read back there, and startup fails if they do not match.
- Writers are concrete types: `SamplesWriter` for `samples`, `SumWriter` for `summary`. Readers are concrete too: `SumReader` for `summary`.
- Every handle is `!Sync`, owned by one thread.

## Pragmas

- `journal_mode = WAL`: commits append to the `-wal` file instead of rewriting pages in the main file, and readers do not block the writer.
- `synchronous = NORMAL`: fsync happens at checkpoints instead of every commit. The database stays consistent, but the latest commits can be lost on power failure.
- Both values are read back after being set in `DbRw::create_new`. Startup fails if they do not match.

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
- The ten channels are `REAL` (8-byte double) in tick order. The `f32` values are widened to `f64` on insert.

```sql
CREATE TABLE samples (
    ts_epoch_micro INTEGER PRIMARY KEY,
    rotor_fl_rpm REAL,
    rotor_fr_rpm REAL,
    rotor_rl_rpm REAL,
    rotor_rr_rpm REAL,
    accel_x_mps2 REAL,
    accel_y_mps2 REAL,
    accel_z_mps2 REAL,
    gyro_x_rads  REAL,
    gyro_y_rads  REAL,
    gyro_z_rads  REAL
);
```

## Batched Transactions

- Each nonempty consumer batch is one transaction. The batch is flushed every 100 ms by default, or at 8192 ticks.
- One commit per batch amortizes WAL write and locking cost. Per-tick commits are not allowed.
- A failed batch rolls back as a whole, so no partial batch is stored.
- The insert statement is prepared once per connection with `prepare_cached` and binds the timestamp plus the ten channels per tick.

```sql
BEGIN;
INSERT INTO samples (
    ts_epoch_micro, rotor_fl_rpm, rotor_fr_rpm, rotor_rl_rpm, rotor_rr_rpm,
    accel_x_mps2, accel_y_mps2, accel_z_mps2, gyro_x_rads, gyro_y_rads, gyro_z_rads
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11);
-- repeated for each tick in the batch, same prepared statement
COMMIT;
```

## Secondary Indexes

- Opt-in through `--with-index`, which selects the `sqlite-all-index` variant.
- The default `sqlite-pk-index` variant has only the primary key.
- One index per channel, ten in total, named `idx_samples_<column>`.
- Each index adds one B-tree write per insert. The cost is measured by comparing both variants.
- The current code creates the indexes before loading data. Building them after the bulk load is faster but is not implemented.

```sql
CREATE INDEX idx_samples_rotor_fl_rpm ON samples (rotor_fl_rpm);
CREATE INDEX idx_samples_rotor_fr_rpm ON samples (rotor_fr_rpm);
CREATE INDEX idx_samples_rotor_rl_rpm ON samples (rotor_rl_rpm);
CREATE INDEX idx_samples_rotor_rr_rpm ON samples (rotor_rr_rpm);
CREATE INDEX idx_samples_accel_x_mps2 ON samples (accel_x_mps2);
CREATE INDEX idx_samples_accel_y_mps2 ON samples (accel_y_mps2);
CREATE INDEX idx_samples_accel_z_mps2 ON samples (accel_z_mps2);
CREATE INDEX idx_samples_gyro_x_rads ON samples (gyro_x_rads);
CREATE INDEX idx_samples_gyro_y_rads ON samples (gyro_y_rads);
CREATE INDEX idx_samples_gyro_z_rads ON samples (gyro_z_rads);
```

## Connection Lifecycle

- The database file is created with `create_new`, so an existing database is never overwritten.
- One `DbRw` connection is used per database. Readers, such as the console or a test, open a separate `DbRo`.
- Finalization closes the connection, then counts the database file plus any remaining `-wal` and `-shm` files.
- Values are always bound as parameters, never formatted into SQL strings.

## Summary Database

- Written by the `simrec-rec-sum` thread into `capture-sum.sqlite`, beside the main recording in `capture-main.sqlite`. It is written for every backend, including `noop`, and is skipped with `--no-sum`.
- Uses the same `lib-db` handles and pragmas as the main recording.
- Table `meta` holds `schema_version`, `bucket_us`, `channels`, and `start_epoch_us`, written once when the database is created.
- Table `summary` has `ts_epoch_micro INTEGER PRIMARY KEY` (bucket start), `tick_count INTEGER NOT NULL`, then `_min`, `_max`, and `_avg` `REAL` columns per channel in channel order, 32 columns in total.
- Buckets are 500 ms (`SUM_BUCKET_US = 500_000`), so `tick_count = 100` for a full bucket at 200 Hz. `avg = sum / tick_count`, computed in `f64`.
- Each nonempty commit is one transaction with a cached prepared statement binding 32 values. Commits happen every `--sum-flush-s` seconds and at disconnect.
- Primary key only, no value indexes, so a day is 172,800 rows.
- Finalization closes the connection, then counts the database plus any remaining `-wal` and `-shm` files.
- `SumReader::bucket_us()` and `SumReader::rows_after()`/`latest()` give the console and tests a read-only view.

## Known Limits

- Not crash-safe. `synchronous = NORMAL` can lose the latest commits after power failure. See `dev/spec-overall.md`.
