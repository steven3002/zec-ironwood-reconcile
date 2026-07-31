# Evidence Bundle Format

An evidence bundle is the artifact `zec-ironwood-reconcile` produces and verifies. It
contains raw captured chain data, the values the capturing node reported, and a versioned
manifest listing every file with its digest.

This document specifies the format precisely enough for an independent implementation to
read and validate a bundle without reference to this crate's source.

**Current schema version: `1.0.0`.**

---

## 1. Directory layout

```
<bundle>/
├── manifest.json                  versioned index of the bundle
├── manifest.sha256                digest of the manifest's canonical serialization
├── anchor/
│   ├── block.hex                  raw consensus bytes of the anchor block
│   └── value-pools.json           pool balances reported at the anchor height
├── blocks/
│   ├── <height>.hex               raw consensus bytes of the block at <height>
│   ├── <height>.pools.json        pool balances reported after the block at <height>
│   └── ...
├── rpc/
│   ├── blockchain-info-start.json
│   ├── blockchain-info-end.json
│   └── node-info.json
├── reports/                       produced by reconciliation, not by capture
│   ├── report.json
│   ├── report.md
│   └── report.sha256
└── metadata/
    ├── capture-environment.json
    ├── command.txt
    └── tool-version.txt
```

### Why raw bytes

Blocks are stored as **raw consensus bytes, hex encoded**, not as a node's decoded JSON.
Decoded JSON is a presentation choice that varies between node implementations and
versions; consensus bytes do not. Storing the decoded form would make a bundle's digests
dependent on the formatting decisions of whichever node captured it, and would make the
tool's parse step a re-reading of another implementation's interpretation rather than an
independent one.

Every file records its encoding explicitly in the manifest, so a reader never infers an
encoding from a file extension.

### The reported-pools files

`anchor/value-pools.json` and `blocks/<height>.pools.json` hold the node's `getblock`
response reduced to the fields that describe the block:

```json
{
  "hash": "0000...",
  "height": 3428143,
  "valuePools": [
    { "id": "orchard",  "chainValueZat": 366000000000000, "valueDeltaZat": -1000, "monitored": true },
    { "id": "ironwood", "chainValueZat": 1000,            "valueDeltaZat": 1000,  "monitored": true }
  ]
}
```

The response is projected rather than stored verbatim because it also carries fields that
describe *when it was asked for*. `confirmations` is the distance to the chain tip, so the
same block yields a different response every few minutes. Storing it would mean two
operators capturing the same interval produced different bytes for the same block, and
independent reproduction is the property this format exists to support.

The projection is an allow-list — `hash`, `height`, and per pool `id`, `chainValueZat`,
`valueDeltaZat`, `monitored` — so a field added by a future node release cannot silently
make evidence unreproducible. The floating-point `chainValue` and `valueDelta` fields are
dropped as a lossy restatement of the zatoshi figures.

An absent balance stays absent. Omission is meaningful: a node undergoing a database
upgrade serves empty pool values at arbitrary heights while otherwise appearing healthy,
and treating that as zero would yield a confident and meaningless reconciliation.

**`monitored`** states whether the node is tracking a pool at that height. A node reports
`chainValueZat: 0` for a pool it is not tracking, so without this flag a placeholder cannot
be told apart from a measured zero — and agreement with a placeholder is not corroboration.

Only the pools files are projected. A block's consensus bytes are stored exactly as served.

### Provenance is not evidence

The `rpc/` and `metadata/` files record the capturing node and session: tip height, sync
progress, database size, capture time. These legitimately differ between two captures of the
same interval and are not inputs to any calculation. Reproducibility applies to the evidence
— `anchor/` and `blocks/` — and to the report derived from it.

---

## 2. Manifest schema

```json
{
  "schema_version": "1.0.0",
  "bundle_id": "mainnet-3428142-3429143",
  "created_at": "2026-07-29T14:30:00Z",
  "tool": {
    "name": "zec-ironwood-reconcile",
    "version": "1.0.0",
    "git_commit": "abc123"
  },
  "source": {
    "implementation": "zebra",
    "version": "6.2.3",
    "rpc_url_redacted": true
  },
  "network": "mainnet",
  "activation": {
    "upgrade": "NU6.3",
    "expected_height": 3428143
  },
  "interval": {
    "anchor_height": 3428142,
    "start_height": 3428143,
    "end_height": 3429143,
    "block_count": 1001
  },
  "anchor": {
    "block_hash": "0000...",
    "orchard_balance_zatoshis": "366000000000000",
    "ironwood_balance_zatoshis": "0"
  },
  "end": {
    "block_hash": "0000...",
    "reported_orchard_balance_zatoshis": "348400000000000",
    "reported_ironwood_balance_zatoshis": "17600000000000",
    "tracking": {
      "orchard_tracked_by_node": true,
      "ironwood_tracked_by_node": true
    }
  },
  "files": [
    {
      "path": "anchor/block.hex",
      "sha256": "e3b0c442...",
      "size_bytes": 2048,
      "encoding": "raw-block-hex"
    }
  ]
}
```

### Field rules

| Rule | Detail |
| --- | --- |
| Monetary values | Integer zatoshi, encoded as JSON **strings** |
| Heights | Unsigned integers |
| Hashes | Lowercase hexadecimal, 64 characters for SHA-256 |
| Timestamps | RFC 3339, UTC |
| `bundle_id` | `<network>-<anchor_height>-<end_height>` |
| `encoding` | One of `raw-block-hex`, `json`, `text` |
| `files` | Sorted by `path`; no duplicates |
| `rpc_url_redacted` | The RPC endpoint is never recorded, only that it was withheld |
| `end.tracking.*` | `true`, `false`, or `null` when the node published no opinion |

### Why monetary values are strings

RFC 8785 canonicalization processes JSON numbers through IEEE-754 doubles, which cannot
represent integers above 2^53 exactly. Zatoshi values reach that magnitude, so a numeric
encoding would be silently lossy inside a hashed artifact. This is a correctness
requirement, not a formatting preference.

---

## 3. Hashing

**File digests.** SHA-256 over the file's exact bytes, lowercase hex. Files are hashed
incrementally, so bundle size does not bound memory use.

**Manifest digest.** SHA-256 over the manifest's **canonical serialization**, per
[RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) (JSON Canonicalization Scheme) — not
over the bytes of `manifest.json` as written. Reformatting or re-indenting the file
therefore does not invalidate its digest, while any change of meaning does.

Canonicalization sorts object keys, emits no insignificant whitespace, and is independent
of field declaration order and of any map iteration order.

---

## 4. Validation

A reader validates a bundle in two phases. Structural validation runs first and completely,
because **the manifest is untrusted input** — a bundle can be authored by anyone.

### Phase 1 — structural, before any file is opened

- `schema_version` major component matches the reader's supported version, otherwise the
  bundle is rejected rather than interpreted;
- `bundle_id` is consistent with `network` and `interval`;
- `anchor_height` is exactly one below `start_height`;
- `block_count` matches the interval bounds;
- every `path` is a safe relative path (see below);
- every `sha256` is 64 lowercase hexadecimal characters;
- `files` is sorted by path and contains no duplicates.

### Path safety

A manifest path is rejected if it is empty, absolute, or contains a `..` component, a `.`
component, a backslash, or a null byte. Without this, a crafted manifest could cause a
reader to open or overwrite a file outside the bundle directory.

Directory traversal during validation never follows symbolic links, so a link cannot cause
a reader to leave the bundle or loop.

### Phase 2 — content

For every listed file: it exists, its size matches `size_bytes`, and its SHA-256 matches
`sha256`. All damaged files are reported, not merely the first.

### Failures and warnings

These are kept strictly separate. A warning must never be mistakable for a pass, and a
failure is never downgraded into one.

| Condition | Result |
| --- | --- |
| Listed file missing | **Failure** |
| Listed file digest or size mismatch | **Failure** |
| Manifest digest mismatch | **Failure** |
| Unsafe path in manifest | **Failure** |
| Unrecognized schema version | **Failure** |
| File present in bundle but absent from manifest | Warning |
| No `manifest.sha256` present | Warning |

Files under `reports/`, and `manifest.sha256` itself, are produced by reconciliation rather
than capture and are exempt from the unlisted-file warning. `manifest.json` does not list
itself.

---

## 5. Stability

The schema is versioned. A change that alters the meaning or serialization of existing
fields requires a major version increment; a reader must reject a major version it does not
implement rather than attempt a partial interpretation.

Error identifiers embedded in reports are stable strings. They do not change when the
human-readable wording of a message changes, so improving a message never alters a
published report or its hash.
