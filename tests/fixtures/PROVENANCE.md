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

*None yet.*

Capture is pending a synced node. When adding one, record:

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

Constructed inline by the test suite rather than stored as files. Currently covering:

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
