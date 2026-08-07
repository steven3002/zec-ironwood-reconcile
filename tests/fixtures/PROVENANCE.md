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

### Block bytes, the activation boundary

`bundles/testnet-activation-boundary/` is a complete evidence bundle captured from a live
**Zebra 6.2.3** node on **testnet**, on **2026-08-01**.

```
Bundle id:     testnet-4133998-4134010
Interval:      4133999..=4134010, anchored at 4133998
Node:          zebra 6.2.3
```

**Why this interval.** It is the only shape in which all three activation checks reach a
verdict at once. It begins one block *below* NU6.3 activation at 4,134,000 and ends ten
blocks above it, so:

- `no_ironwood_before_activation` has a pre-activation height to range over (4,133,999);
- `orchard_withdrawal_only` has post-activation heights to range over;
- `ironwood_anchor_zero` has the block before activation inside the interval.

Before this fixture existed, every captured interval lay wholly on one side of the boundary
and all three checks had only ever been observed reporting *not applicable*, which reads
like coverage in a summary and is none. `tests/activation_boundary.rs` pins each verdict.

**The cross-check.** Zebra reports, for every height in the interval, an Orchard balance of
25,292,367,414,135 zatoshi unchanged across activation, an Orchard delta of zero, and an
Ironwood balance of zero. The reconstruction from the blocks' own bytes agrees at every
height on both axes.

**What it does not show.** Ironwood receives no value until 4,134,683, so this bundle
demonstrates the boundary *rules*, not Ironwood value movement. That is what
`bundles/testnet-ironwood/` is for.

### Block bytes, the Ironwood bundle

`bundles/testnet-ironwood/` is a complete evidence bundle captured from a live **Zebra
6.2.3** node on **testnet**, on **2026-08-01**.

```
Bundle id:     testnet-4134682-4134686
Interval:      4134683..=4134686, anchored at 4134682
Anchor hash:   000000284e4096a7da0b56dc3cebc48c323ce0ede20ee0daddd7a4f97548b5af
End hash:      00000b3fc6629f6fe286d7a91bc0f59e06c9b061448bb54fd3d422ed32974438
Node:          zebra 6.2.3
```

**Why this interval.** Height 4,134,683 is where value first entered the Ironwood pool on
testnet, 683 blocks after NU6.3 activated at 4,134,000. It was located by scanning the
node's reported `valueDeltaZat` for the ironwood pool from the activation height upward.

**The cross-check.** Zebra independently reports, for height 4,134,683:

```json
{"id": "ironwood", "chainValueZat": 125000000, "valueDeltaZat": 125000000, "monitored": true}
```

Reconstructing the same figure from the block's own version 6 transaction bytes is what
this fixture exists to demonstrate, and `tests/end_to_end.rs` asserts it. It is the one
claim a synthetic fixture cannot support, because a synthetic fixture is built from the same
understanding of the format as the code that reads it.

The interval also carries a real Orchard balance (25,292,367,414,135 zatoshi) that is
compared at every height, and the node reports it was tracking both reconstructed pools, so
the end-balance agreement is a measurement rather than a placeholder.

**What it does not cover.** Four blocks on testnet. It does not establish behaviour across
a reorganisation, at scale, or on mainnet, and it is not a substitute for the independent
external reproduction that release requires.

### Block bytes, the mainnet activation boundary

`bundles/mainnet-activation-boundary/` is a complete evidence bundle captured from a live
**Zebra 6.2.3** node on **mainnet**, on **2026-08-07**.

```
Bundle id:     mainnet-3428141-3428146
Interval:      3428142..=3428146, anchored at 3428141
Anchor hash:   0000000000acbb4e293519d3025062019b696f87e263347f2b04b4bc567bda6b
End hash:      00000000003b1684e4f65ba8077d7ce7f4416049be72b94ec1716b105ed14252
Node:          zebra 6.2.3
Tip at capture: 3439807
```

**Why this interval.** NU6.3 activated on mainnet at 3,428,143, and value first entered the
Ironwood pool at 3,428,144, the very next block. An interval anchored one block below
activation and running three blocks past the first inflow is therefore the only shape in
which every check reaches an affirmative verdict *over moving pools*: the three activation
checks each have the heights they range over, and the accounting checks have real value
movement in both pools to compare.

`bundles/testnet-activation-boundary/` also records no `NotApplicable` and no `Warn`, but
its Orchard and Ironwood deltas are zero at every height, so it affirms the boundary rules
against a motionless ledger. This bundle affirms the same rules with value moving at four of
its five heights, which is the combination neither fixture had before.

**The finding.** Mainnet and testnet funded Ironwood by different mechanisms, and the
contrast is the reason this fixture is worth its bytes:

| | testnet 4,134,683 | mainnet 3,428,144 |
| --- | --- | --- |
| Blocks after activation | 683 | 1 |
| Transactions in the block | 1, a coinbase | 3 |
| Ironwood delta | +125,000,000 | +1,000,000 |
| Orchard delta | 0 | −1,020,000 |
| Where the value came from | issuance | the Orchard pool |

On mainnet the Orchard outflow exceeds the Ironwood inflow by 20,000 zatoshi, and the
transparent pool's delta rises by exactly that amount: +137,520,000 at 3,428,144 against
+137,500,000 at 3,428,143. The matching figure is the measurement; reading a transaction fee
into it is an interpretation of the measurement and not something the node states. Testnet
had only ever shown the issuance mechanism, which is why an earlier turnstile invariant had
to be disproved rather than asserted. Both mechanisms now have captured evidence behind them;
neither is a rule.

**The cross-check.** Zebra independently reports, for height 3,428,144:

```json
{"id": "ironwood", "chainValueZat": 1000000, "valueDeltaZat": 1000000, "monitored": true}
{"id": "orchard",  "chainValueZat": 366123340319978, "valueDeltaZat": -1020000, "monitored": true}
```

The reconstruction from the blocks' own version 6 transaction bytes agrees at every height
in the interval, on both the running balance and the per-block delta.
`tests/mainnet_evidence.rs` pins that agreement and the bundle's published report hash,
`0a2ca229…`.

The `monitored` flag also flips `false` → `true` at exactly 3,428,144, the height Ironwood
first held value. That is the same behaviour the testnet sapling pair at 280,768/280,769
records, now observed on a second pool and a second network.

**What it does not cover.** Five blocks. It does not establish behaviour across a
reorganisation or at scale, and it is not a substitute for the independent external
reproduction that release requires. No machine other than the two the publisher operates has
reproduced its hash.

### Mainnet bundles that are published but not committed

Two further mainnet bundles were captured on 2026-08-07 and are published as release
artifacts only:

| Bundle id | Interval | Archive | Why it is not committed |
| --- | --- | --- | --- |
| `mainnet-3428143-3428147` | 3428144–3428147 | 283 KiB | Its heights are almost entirely those of the committed bundle, and starting *at* activation leaves two activation checks `NotApplicable`. It adds one height and no assertion the committed bundle cannot make |
| `mainnet-3439599-3439699` | 3439600–3439699 | 3.6 MiB | Six times the size of every other fixture in this repository combined, for a claim about scale that a release artifact carries just as well |

The second is nonetheless the only evidence covering **Ironwood outflows**: 25 of its 100
heights have a negative Ironwood delta, a direction no committed fixture exercises, and its
anchor holds a non-zero Ironwood balance. If a defect were ever suspected in the outflow
direction, that bundle is where to look, and a committed fixture covering it would be
justified then. Committing it now would multiply the repository's fixture weight to guard a
path no reported defect has touched.

### Further block fixtures

When adding one, record:

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
were derived from real chain data, they were not.

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

## Open dependency, closed

The parse layer's correctness against real Ironwood bundles was unproven until a captured
fixture existed containing a version 6 transaction with an Ironwood bundle, cross-checked
against the capturing node's own reported per-block delta.

`bundles/testnet-ironwood/` is that fixture, and the cross-check agrees. What remains open
is breadth, not existence: mainnet, larger intervals, and reproduction by someone else.
