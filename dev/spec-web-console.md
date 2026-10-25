# Web Console Protocol v1 Spec

## Intent

**Status: proposed, not implemented.** This document defines the target web console, not existing functionality. The current summary database and read-only access in `lib-db` are existing foundations; the console server, endpoints, binary protocol, and UI described here are not implemented yet.

- Serve a browser console from a new Axum-based `console-server`, reading `capture-sum.sqlite` through `lib_db::DbRo` and `SumReader`.

- Provide live summary data through one WebSocket at `/ws/live`, historical ranges through `GET /api/summary`, and schema metadata through `GET /api/meta`.

- Display the ten summary channels with min, max, and average values, using the existing summary bucket timestamps and channel order.

- Keep server state limited to each open socket's cursor and bounded transport state. No persistent session or resume token is required.

- Keep the UI's resume cursor as the `ts_epoch_micro` of its newest row.

- Define "now" as the newest committed row in the database, never the wall clock. Max-speed replays of old datasets must work without special handling.

- Use a live window of `LIVE_WINDOW_US = 120_000_000` microseconds, two minutes or 240 rows at the current 2 Hz summary rate.

This scope covers the console's read-only transport and rendering behavior. It does not change capture generation, recording, or the static capture report dashboard.

## Code Design

### Existing foundations and required additions

- The summary schema and metadata are described in `dev/spec-overall.md`, section `Summary Database`, and `dev/spec-db-setup.md`, section `Summary Database`.

- Existing access lives in `crates/lib-db/src/conn.rs` and `crates/lib-db/src/schema/sum.rs`. `SumReader` currently exposes `bucket_us()`, `rows_after(ts, limit)`, and `latest(n)`.

- Add the metadata and bounded time-range reader operations required by this protocol within `lib-db`. The console must not access `rusqlite` directly.

- Keep each database handle owned by one thread. Execute synchronous SQLite work outside Axum's asynchronous request execution threads, returning owned metadata or rows to the transport layer.

- Separate database access, binary frame encoding, HTTP and WebSocket transport, and browser state and rendering responsibilities.

### Time and metadata

All summary row timestamps are `ts_epoch_micro`, the start of a bucket in epoch microseconds. `start_epoch_us` retains the existing metadata field name for the dataset start.

`GET /api/meta` returns `application/json`:

```json
{
  "protocol_version": 1,
  "bucket_us": 500000,
  "live_window_us": 120000000,
  "start_epoch_us": 1792022400000000,
  "latest_ts_epoch_micro": 1792066717000000,
  "channels": [
    "rotor_fl_rpm",
    "rotor_fr_rpm",
    "rotor_rl_rpm",
    "rotor_rr_rpm",
    "accel_x_mps2",
    "accel_y_mps2",
    "accel_z_mps2",
    "gyro_x_rads",
    "gyro_y_rads",
    "gyro_z_rads"
  ],
  "row_bytes": 136
}
```

- Read `bucket_us`, `start_epoch_us`, and `channels` from the database `meta` table.

- Derive `latest_ts_epoch_micro` from the newest summary row. Return `null` when the database is empty.

- Supply `protocol_version`, `live_window_us`, and `row_bytes` from the console protocol definition, not from the database metadata.

- Return HTTP `503` with `{"error":"no_capture"}` while `capture-sum.sqlite` does not exist.

The UI fetches metadata once at startup and uses `bucket_us` rather than hardcoding the bucket width.

### Binary frame format

Use this format for every WebSocket data message and every successful `/api/summary` response. All values are little-endian.

#### Header, 8 bytes

| Offset | Type | Field | Contract |
| ------ | ---- | ----- | -------- |
| 0 | u8 | `kind` | `1` snapshot, `2` live, `3` history |
| 1 | u8 | `version` | `1` |
| 2 | u8 | `flags` | Bit 0 is `truncated`, indicating more rows exist past this frame |
| 3 | u8 | reserved | `0` |
| 4 | u32 | `row_count` | Number of rows following the header |

#### Row, 136 bytes

| Offset | Type | Field |
| ------ | ---- | ----- |
| 0 | i64 | `ts_epoch_micro` |
| 8 | u32 | `tick_count` |
| 12 | u32 | reserved, `0` |
| 16 | f32 x 10 | `min`, in channel order |
| 56 | f32 x 10 | `max`, in channel order |
| 96 | f32 x 10 | `avg`, in channel order |

- Rows are strictly ascending by `ts_epoch_micro`, with no duplicate timestamps.

- Frame length must equal `8 + row_count * 136`. The UI rejects any other length.

- Every row starts on an 8-byte boundary relative to the frame buffer.

- The browser can decode through `DataView`. On little-endian hosts, it can also use `Float32Array` views into the received buffer without copying float values.

- Timestamp gaps represent missing buckets. Charts draw a break rather than connecting across the gap.

### WebSocket live subscription

The UI opens `/ws/live` and sends one text JSON message immediately:

```json
{
  "type": "subscribe",
  "after_us": 1792066597000000
}
```

- When `after_us` is omitted or `null`, send the last `live_window_us` of rows.

- When `after_us` is supplied, send rows strictly after that timestamp, but never older than `latest_ts_epoch_micro - live_window_us`.

- Historical rows outside that window are retrieved through `/api/summary`, not through the socket.

- Accept only one subscription per socket. A second subscription closes the socket with code `1008`.

- If no subscription arrives within five seconds, close the socket with code `1008`.

#### Server data flow

1. Send exactly one snapshot frame, `kind = 1`, containing the initial backfill. An existing empty database may produce a zero-row snapshot.
2. Poll for rows strictly after the socket cursor every `bucket_us / 2`, currently 250 ms.
3. Send a live frame, `kind = 2`, only when new rows are available.
4. After each successful send, advance the cursor to the newest timestamp sent.

If the database does not exist yet, keep the socket open and defer the snapshot until rows appear.

Database failures produce a text JSON error followed by a close with code `1011`:

```json
{
  "type": "error",
  "code": "db_error",
  "message": "..."
}
```

#### Keepalive and backpressure

- Send a WebSocket ping every 15 seconds.

- Close the socket if no pong comes back within 30 seconds.

- Limit each socket's send queue to 64 frames.

- When the queue is full, disconnect the slow client with code `1013` rather than accumulating an unbounded backlog.

#### Browser reconnect behavior

- Reconnect after a close or error with an initial delay of 250 ms, doubling to a maximum of five seconds.

- Subscribe with `after_us` equal to the newest timestamp held by the UI.

- Omit `after_us` when the UI holds no rows.

- Resume through the socket only within its two-minute backfill bound. Retrieve any required older gap through the history endpoint.

### HTTP history endpoint

Request:

```http
GET /api/summary?from_us=<i64>&to_us=<i64>&max_rows=<u32>
```

| Parameter | Required | Contract |
| --------- | -------- | -------- |
| `from_us` | Yes | Inclusive range start |
| `to_us` | Yes | Exclusive range end, greater than `from_us` |
| `max_rows` | No | Default `20000`, capped at `20000` |

- Return `application/octet-stream` containing one `kind = 3` frame with rows in `[from_us, to_us)`.

- When more than `max_rows` rows match, return the first `max_rows` rows and set `truncated`.

- Continue a truncated query from the last returned timestamp plus one microsecond, retaining the original exclusive end.

- Enable gzip response compression.

- Cancel requests that are no longer needed for the visible range through `AbortController`.

Errors return `application/json`:

```json
{
  "error": "<code>",
  "message": "..."
}
```

| HTTP status | Code | Meaning |
| ----------- | ---- | ------- |
| `400` | `bad_range` | Missing or invalid range |
| `503` | `no_capture` | Database does not exist yet |
| `500` | `db_error` | Other database failures |

### Browser state and rendering

- Maintain a live ring with 240 slots for the current 2 Hz summary rate. Fill it from the snapshot, then append live rows, dropping the oldest row when full.

- Keep historical windows separate from the live ring, keyed by range, with count-based eviction. A maximum of 20 windows is the initial suggested bound.

- Data handlers update state and set a dirty flag. A `requestAnimationFrame` loop redraws only when dirty.

- The **1 min** and **2 min** range buttons use the live ring.

- The **10 min** and **Flight** range buttons fetch historical data from `/api/summary`.

- Above approximately 2,000 visible rows, merge rows per pixel column using the minimum of minima, maximum of maxima, and average of averages. This bounds drawing work by chart width.

- Use modern browser ES modules, standard `fetch`, and Canvas 2D drawing. Style elements by class name and prefer CSS Grid for layout.

- Selected tabs and segmented controls use `box-shadow: inset 0 -2px 0 var(--accent)` with `border-radius: 0`, so the bottom indicator stays flat.

### Versioning

- Require `protocol_version = 1` in metadata and `version = 1` in every binary header.

- On a version mismatch, show an error and stop reconnecting.

## Design Considerations

### Database-relative now

Using the newest committed row instead of wall-clock time keeps live windows meaningful for old datasets and accelerated replays. A paused capture therefore leaves the view anchored to its latest data rather than moving into an empty wall-clock interval.

### Bounded live backfill

The live socket never backfills more than two minutes. Older data belongs to the history endpoint, keeping socket startup and recovery separate from arbitrary historical queries.

This is a deliberate recovery limit. Reconnecting with the last timestamp does not guarantee recovery of every missed row after a long disconnection. The UI must use history if it needs continuity beyond the live window.

### Padded, aligned rows

The 136-byte row includes a reserved four-byte field after `tick_count`. Combined with the eight-byte header, this keeps row starts 8-byte aligned and supports direct float views in the browser.

The wire format uses `f32` even though stored summary values are `f64`. This reduces transfer size but introduces rounding. `DataView` remains the portable decoder because the wire format is explicitly little-endian.

### Disconnect rather than buffer

A slow tab is disconnected instead of being buffered indefinitely. The bounded queue protects server memory, while the timestamp cursor lets a reconnect request only rows newer than those already held.

Recovery is bounded by the two-minute socket window. Exact recovery from the previous position after a longer interruption requires history requests, not just a new subscription.

A queue limit measured only in frames is not a complete byte bound unless frame sizes are also bounded. Maximum live frame size and catch-up chunking remain implementation decisions to settle before implementing the transport.

### Read-only ownership and short queries

Reuse `lib-db` so database access remains centralized and read-only. Respect the existing single-thread ownership of handles and keep read transactions short to avoid holding back WAL checkpoints.

### Rendering and aggregation limits

Separating state updates from drawing avoids a redraw for every incoming message. Pixel-column merging bounds rendering work for long ranges without loading the 200 Hz main recording.

An average of bucket averages is a visualization approximation when `tick_count` differs across rows. It is not a tick-weighted aggregate, especially at capture boundaries or after dropped batches.

A 240-slot ring holds at most two minutes at 2 Hz, but timestamp gaps can make those rows span longer than two minutes. Range selection must use timestamps rather than assuming a full, gap-free ring.
