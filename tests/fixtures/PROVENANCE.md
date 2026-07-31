# Fixture Provenance

Every test vector in this directory records where its bytes came from. A fixture without
recorded provenance is not evidence and must not be cited in support of any claim about the
tool's behaviour against real chain data.

Fixtures are classified as either **captured** or **synthetic**.

| Class | Meaning | What it can demonstrate |
| --- | --- | --- |
| **Captured** | Raw consensus bytes obtained from a Zcash node, recorded with network, height, block hash and the node version that served them | That the tool reconstructs real value-pool changes |
| **Synthetic** | Bytes constructed by this test suite | That the tool handles a structural case, including malformed input |

A synthetic fixture can demonstrate that malformed input is rejected. It **cannot**
demonstrate that extraction from a real Ironwood bundle is correct, because it was built
from the same understanding of the format that the code under test implements. Only a
captured fixture breaks that circularity.

---

## Captured fixtures

### Node RPC responses

Recorded from a live **Zebra 6.2.3** node on **testnet** on **2026-07-31**, by issuing the
listed call against `http://127.0.0.1:18232` and storing the JSON-RPC `result` member
(`error-block-out-of-range.json` stores the whole envelope, since the point of it is the
error member). No response contains a credential.

| File | RPC call | SHA-256 | What it pins |
| --- | --- | --- | --- |
| `getinfo-zebra-testnet.json` | `getinfo` | `d3d63822ec54a4613091c7c3089d2a9f3a6a36b6ef7b5118467b6453116d4645` | Node identity fields and the `testnet` flag |
| `getblockchaininfo-zebra-testnet.json` | `getblockchaininfo` | `197af7a1a74372b75e956d9060c8c58bb51d62069e44ddafa96a4dc08f35e372` | `chain: "test"`, the branch-id-keyed upgrade table including NU6.3 at 4,134,000, and the six-pool set |
| `getblock-verbose-280769.json` | `getblock "280769" 1` | `3f6450f468e6aa9ba229e47e74ad67292fa234cd5d5772266cb6c80b5af48f03` | Per-pool `chainValueZat` / `valueDeltaZat` / `monitored`, and the presence of `confirmations` |
| `getblock-verbose-280768-unmonitored.json` | `getblock "280768" 1` | `5f0eda127d2d43c6fc2a2f079c5ef705ee7df6ee69a31f73de1df317eebad134` | The height *before* the sapling pool first held value: `monitored: false` with `chainValueZat: 0` |
| `getblock-raw-280769.json` | `getblock "280769" 0` | `ae45602221c9189f450b8016aa6ab6956104b468bec9d09e18bc844afd0e54e6` | Verbosity 0 returns a hex string |
| `error-block-out-of-range.json` | `getblock "999999999" 1` | `984e7dbad2a424fc1322d3ceb9733a7e1310736aa35d8275b58fd8a1977545f8` | Application errors arrive with **HTTP 200**, an `error` member, and no `result` |

The pair at heights 280,768 and 280,769 is the evidence behind the `monitored` handling:
280,769 is the testnet height at which the sapling pool first received value, found by
bisection, and the flag flips exactly there.

**What these do not show.** They are pre-activation testnet blocks. They pin *response
shape*, not Ironwood behaviour, and cannot close Gate 1.

### Block bytes

*None yet.* When adding one, record:

```
File:            <relative path>
Network:         mainnet | testnet
Height:          <height>
Block hash:      <hash as reported by the node, display order>
Node:            <implementation and version>
RPC call:        getblock <height> 0
Captured:        <RFC 3339 UTC>
SHA-256:         <digest of the file>
Cross-check:     <the node's reported valueDeltaZat for each pool at this height>
```

The cross-check field is what makes a captured fixture useful: it records what the node
independently said the block's pool deltas were, so the test can assert that this crate's
reconstruction agrees.

---

## Synthetic fixtures

### Stored

| File | Class | Purpose |
| --- | --- | --- |
| `golden-report.json` | Synthetic | Canonical serialization of a report built from synthetic ledgers. Pins the report schema and the RFC 8785 output byte for byte. |
| `golden-report.sha256` | Synthetic | Digest of the above. |

The golden report is a **regression guard, not evidence.** It proves that this build's
canonical serialization has not changed. It says nothing about whether the figures inside it
were derived from real chain data — they were not.

If the canonical serialization changes deliberately, regenerate both files **and** increment
the report schema version. A silent change would mean a previously published report hash is
no longer reproducible by this build.

### Constructed inline

- empty input;
- truncated block bytes;
- non-hexadecimal input;
- odd-length hexadecimal.

These exercise the failure paths only. They are deliberately not used to assert any
positive claim about value extraction.

---

## Open dependency

The parse layer's correctness against real Ironwood bundles is **unproven** until at least
one captured fixture exists containing a version 6 transaction with an Ironwood bundle,
cross-checked against the capturing node's own reported per-block delta.
