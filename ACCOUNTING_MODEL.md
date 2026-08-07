# Accounting Model

How `zec-ironwood-reconcile` converts public transaction data into chain value-pool changes,
and how those changes are compared against the node that served the data.

This document is the arithmetic contract. Everything a report asserts follows from what is
written here, and a reader who disagrees with a report should be able to locate the
disagreement in one of the sections below.

Companion documents: [`EVIDENCE_FORMAT.md`](EVIDENCE_FORMAT.md) for the on-disk artifact,
[`LIMITATIONS.md`](LIMITATIONS.md) for what the output may be cited for,
[`ARCHITECTURE.md`](ARCHITECTURE.md) for how the code is arranged.

---

## 1. Units

All monetary values are **integer zatoshi**. There is no floating-point arithmetic anywhere
in this crate: the lint `clippy::float_arithmetic` is set to `deny` crate-wide, with no
exceptions, so the property is enforced by the compiler rather than by review.

The in-memory type is `Zatoshi`, a checked signed newtype over `i64`. A pool balance is
bounded by `MAX_MONEY` = 21,000,000 × 10^8 = **2,100,000,000,000,000** zatoshi, about three
orders of magnitude below `i64::MAX` ≈ 9.22 × 10^18, so `i64` is ample for any individual
balance or delta. Interval accumulation runs in `i128` and narrows back to `i64` with a
checked conversion.

**Overflow is an error, never a wrap and never a panic.** Every addition and accumulation is
checked and returns a `Result`. The release profile additionally sets `overflow-checks = true`,
so an unchecked arithmetic slip would abort rather than wrap silently.

### Why monetary values are JSON strings

Every monetary value in a manifest or a report is encoded as a **string**, and a number is
rejected on deserialization.

The reason is *not* that zatoshi amounts exceed what a JSON number can hold exactly. They do
not. RFC 8785 processes JSON numbers through IEEE-754 doubles, which represent integers
exactly up to 2^53 − 1 = **9,007,199,254,740,991**. `MAX_MONEY` is **2,100,000,000,000,000**,
which is below that limit by a factor of about four. Any valid amount is safely inside the
range.

The reason is that **the margin is not the guarantee**. Three things are not bounded by
`MAX_MONEY`:

- interval accumulation runs in `i128`, where intermediate sums are bounded by the
  accumulator, not by the money range;
- every figure in a bundle is **untrusted input**, a bundle can be authored by anyone, and
  a hostile or damaged one may carry any integer at all;
- a value out of range must produce a diagnosable error, not a rounded figure.

A numeric encoding would round such a value silently and the rounded figure would be what got
hashed. A string encoding cannot, and it removes any dependence on how a particular
canonicalizer handles numbers. This is a correctness requirement, not a formatting
preference.

---

## 2. Sign convention

The single highest-consequence rule in the project. It is **derived from specification, not
inferred from observed transactions**.

### 2.1 The rule

[ZIP 209](https://zips.z.cash/zip-0209) defines a shielded pool's chain value balance as the
**negation** of the sum of the `valueBalance` fields of that pool's bundles across the chain.
[ZIP 258](https://zips.z.cash/zip-0258) extends the same treatment to the Ironwood pool for
NU6.3.

For a pool *P* and a transaction *t*:

```
delta_P(t) = -valueBalance_P(t)
```

| `valueBalance` sign | Meaning | Effect on the pool balance |
| --- | --- | --- |
| negative | value **enters** the pool | balance increases |
| positive | value **leaves** the pool | balance decreases |
| zero, or bundle absent | no effect on that pool | unchanged |

The value is bounds-checked *before* it is negated, so a field holding `i64::MIN` is reported
as malformed data rather than overflowing during negation.

### 2.2 Corroboration

The `orchard` crate documents `Bundle::value_balance` as the net value moved into or out of
the pool, computed as the sum of spends minus outputs. A transaction creating more output
value than it spends is moving value into the pool and yields a negative `valueBalance`,
which under the rule above produces a positive pool delta. Specification and reference
implementation agree.

Both are now superseded as evidence by a direct on-chain demonstration: at testnet height
4,134,683 the reconstruction from the block's own bytes produces an Ironwood delta of
+125,000,000 zatoshi, which is the figure the network independently reports for that height.
A wrong sign would have produced −125,000,000.

### 2.3 Where it lives

`src/parse/value_balance.rs`, stated exactly once, with the ZIP citation in the module
documentation. Every downstream consumer receives an already-signed delta and never
re-derives direction. A second negation applied to a value balance anywhere else in the
codebase is a defect.

---

## 3. Extraction

For each transaction the tool:

1. deserializes it with `zcash_primitives::transaction::Transaction::read`;
2. reads `orchard_bundle()` and `ironwood_bundle()`, each an `Option<&Bundle<..>>`;
3. takes `Bundle::value_balance()` from each bundle that is present;
4. applies the sign convention to obtain an Orchard delta and an Ironwood delta;
5. records a per-transaction entry carrying the txid, height, index, version and both deltas.

An **absent bundle contributes zero** to that pool. This is a correct accounting statement,
not a skip: a transaction with no Ironwood bundle genuinely moves no value into or out of the
Ironwood pool. The zero is recorded explicitly rather than omitted, so every transaction in
the interval appears in the ledger and the ledger accounts for the whole block rather than
its shielded subset.

### 3.1 Transaction versions

An interval contains a mixture of versions. Version 6 is not mandatory after activation.

| Version | Orchard bundle | Ironwood bundle | Handling |
| --- | --- | --- | --- |
| v4 and earlier | absent | absent | both deltas zero |
| v5 | possible | absent | Orchard delta only |
| v6 | possible | possible | both |

`Transaction::read` dispatches on the version read from the wire. For v3 and v4 it uses the
`BranchId` supplied by the caller; for **v5 and v6 it is not given that parameter at all** and
reads the consensus branch identifier from the transaction's own header fragment. The tool
supplies the branch identifier the upgrade table gives for the block's height, then reads
`Transaction::consensus_branch_id()` back and compares. See
[`LIMITATIONS.md`](LIMITATIONS.md) §2 for what that readback does and does not prove.

An unrecognized version is a hard failure, never a skip. It is rejected during
deserialization and surfaces as a parse failure naming the height, with exit code 5.

---

## 4. Aggregation

### 4.1 Per block

For each height *h*, sum the per-transaction deltas:

```
blockDelta_P(h) = Σ over transactions t in block h of delta_P(t)
```

### 4.2 Across the interval

```
ΔP = Σ over h from start_height to end_height of blockDelta_P(h)
```

### 4.3 Expected balances

Given the anchor balances, the running expected balance after height *h* is

```
expected_P(h) = P_anchor + Σ over h' from start_height to h of blockDelta_P(h')
```

and the expected ending balance is `expected_P(end_height)`.

The anchor is the block at `start_height − 1`. Its balances are **declared**, taken from the
capturing node and recorded in the evidence, and are an **input** to the calculation rather
than a result of it. The tool reconstructs *changes* over an interval; it does not derive the
anchor from genesis and does not claim to. The consequence is stated in
[`LIMITATIONS.md`](LIMITATIONS.md) §2.

The anchor figure used by the arithmetic is read from `anchor/value-pools.json`, what the
node actually said, and never from the manifest. A manifest is written by whoever produced
the bundle, so taking the anchor from it would mean deriving a result from a number its own
author supplied. The manifest's stated anchor is compared against the evidence and any
disagreement is reported as the `manifest_matches_evidence` check.

---

## 5. Comparison against the node

Zebra's `getblock <height> 1` reports, per pool and per height, both `chainValueZat`, the
balance after that block, and `valueDeltaZat`, that block's change. The tool compares its
reconstruction against both, at every height:

| Axis | Reconstructed value | Node's value | Meaning of a mismatch |
| --- | --- | --- | --- |
| Delta | `blockDelta_P(h)` | `valueDeltaZat` at *h* | a divergence originating in block *h* |
| Balance | `expected_P(h)` | `chainValueZat` at *h* | cumulative divergence, first visible at *h* |

### What the two axes are, and are not

Both axes compare a figure **this crate reconstructed from block bytes** against a figure the
node reported. Neither is self-referential, and that is what makes either of them meaningful.

They are **not** two independent confirmations. Zebra derives `valueDeltaZat(h)` as
`chainValueZat(h) − chainValueZat(h−1)` from its own stored per-block records. So if the
delta axis agrees at every height and the anchor equals the node's balance at the anchor
height, the balance axis follows by telescoping and cannot fail independently. A reader
should count **one independent comparison per height**, not two.

Both are still worth computing, for different reasons:

- the **delta axis** localises a divergence to the block that caused it, and from there to a
  transaction, instead of reporting only that the endpoints differ;
- the **balance axis** catches an anchor that does not line up with the interval, which the
  delta axis alone would not notice.

A comparison the node did not make possible is recorded as `NOT_REPORTED`, distinct from
agreement and from disagreement. A height for which the node holds no per-block record yields
a response with the `valuePools` key omitted entirely, not zeroed, not empty, and agreement
asserted over a value that was never served would corroborate nothing.

Reconciliation **passes** only when, for both Orchard and Ironwood, the expected ending
balance equals the reported ending balance and every per-height comparison agrees. Exit code
0 is returned only in that case; a failed accounting comparison always exits 1.

---

## 6. Activation-specific rules

NU6.3 makes several properties checkable that are specific to this upgrade and do not
generalise to arbitrary historical windows. Where a property does not apply to the interval
being examined, it is recorded as **not applicable with a stated reason** rather than passed
silently, a check that passes vacuously asserts a rule held across heights where it did not
exist.

| Check id | Rule | Basis |
| --- | --- | --- |
| `ironwood_anchor_zero` | The Ironwood balance at the block immediately preceding activation is zero | ZIP 258 introduces the pool empty |
| `no_ironwood_before_activation` | No Ironwood value appears at any height below the activation height | ZIP 258 |
| `orchard_withdrawal_only` | For **every transaction** at or after activation, the Orchard pool balance contribution is non-negative | ZIP 258 |
| `no_negative_pool_balance` | Every reconstructed running balance stays at or above zero, for every pool at every height | ZIP 209 as extended by ZIP 258 |
| `consensus_branch_id_valid` | Every transaction's branch identifier matches the one expected for its height | ZIP 258 |
| `activation_context_valid` | The bundle's declared activation height matches the protocol constant for its network | ZIP 258 |

Two of these deserve their reasoning stated.

**`orchard_withdrawal_only` is checked per transaction, not per block.** ZIP 258 states the
rule as "for every transaction, v^OrchardPoolBalance ≥ 0". A block containing one transaction
of +100 and another of −200 nets to −100 and would pass a per-block test while the first
transaction broke consensus. The check names the offending txid and index.

**`ironwood_anchor_zero` reads the boundary balance from wherever the bundle establishes
it**, the anchor, or a height inside the interval. Tying it to the anchor alone made it
mutually exclusive with `no_ironwood_before_activation`, which needs a pre-activation height
*inside* the interval, so no single bundle could affirm both halves of the boundary claim.

`no_negative_pool_balance` is the tool independently re-deriving the pool non-negativity rule
from transaction data rather than trusting the node that enforced it. That is the substantive
consensus property this project can speak to.

### 6.1 Pool flows are reported, never asserted

The cumulative value leaving Orchard and the cumulative value entering Ironwood over the
interval are both computed and reported, as **two separate observations**. No relationship
between them is asserted, and none may be.

An intuitive invariant, that cumulative Ironwood inflow cannot exceed cumulative Orchard
outflow, is **false**. At testnet height 4,134,683, the first block in which Ironwood held
any value, the pool gained 125,000,000 zatoshi while Orchard released nothing. The block
contains a single transaction, the coinbase, distributing that height's block subsidy of
156,250,000 zatoshi:

| Pool | Delta |
| --- | --- |
| transparent | +12,500,000 |
| lockbox | +18,750,000 |
| ironwood | +125,000,000 |

**Ironwood receives newly issued value directly from coinbase transactions.** Its inflow can
therefore exceed Orchard's outflow, and that difference is ordinary issuance rather than
unexplained supply. This is consistent with ZIP 258, which forbids Orchard actions in
coinbase transactions while permitting Ironwood ones.

Had the inequality been asserted as a check, the tool would have emitted a false failure
against the first real Ironwood block on testnet. The report field is named
`pool_flows_observed` rather than anything suggesting a turnstile, and one of the nine
limitations every report carries states in plain terms that the two figures are not a
balance.

#### Mainnet shows the opposite ordering, and a third source

Mainnet funded Ironwood by moving value out of Orchard rather than by issuing it. At height
3,428,144, the first block after NU6.3 activated, Ironwood gained 1,000,000 zatoshi while
Orchard released 1,020,000 — the outflow **exceeded** the inflow, the reverse of the testnet
case, with the 20,000 difference appearing in the transparent pool.

The excess is not always issuance either. Across the captured mainnet heights the six pool
deltas sum to exactly 156,250,000 zatoshi, that era's block subsidy, at every height without
exception. At 3,428,146 Ironwood gained 962,355 while Orchard released 538,490, and the
transparent pool's delta fell short of its coinbase-only figure by exactly the 423,865
difference:

| Height | transparent | lockbox | orchard | ironwood | sum |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 3,428,143 | +137,500,000 | +18,750,000 | 0 | 0 | 156,250,000 |
| 3,428,144 | +137,520,000 | +18,750,000 | −1,020,000 | +1,000,000 | 156,250,000 |
| 3,428,145 | +139,225,221 | +18,750,000 | −101,725,221 | +100,000,000 | 156,250,000 |
| 3,428,146 | +137,076,135 | +18,750,000 | −538,490 | +962,355 | 156,250,000 |

So an Ironwood inflow larger than the Orchard outflow does not identify issuance as the
source; value reaching Ironwood from the transparent pool produces the same signature in a
two-pool view. Reading either ordering as evidence of anything would repeat the error §6.1
exists to record. The two figures are reported. Nothing is inferred from their relationship.

---

## 7. What the node's `monitored` flag contributes

Nothing. Zebra builds every pool entry through one constructor that sets `monitored` to
`chainValueZat != 0`, so the flag restates whether the balance is non-zero and carries no
information the balance does not. It is preserved in evidence and recorded in the manifest
because it is part of the response. **No check rests on it**, and no reader should treat
`false` as marking a balance unmeasured.

Whether a comparison is possible is decided by whether the node reported a balance at all,
which is a different question and is what the `end_balances_corroborated` check asks.

---

## 8. Unsupported cases

Never silently skipped. Each produces a deterministic failure carrying the height, the
transaction index, the txid where available, a stable error identifier, and a human-readable
explanation:

- unknown transaction versions;
- malformed length fields;
- truncated transactions;
- values outside the permitted numeric bounds;
- blocks whose transaction list cannot be completely parsed;
- a block whose coinbase-declared height disagrees with the height it was captured at;
- a pool-state record that does not describe the block it is filed against;
- absent or non-numeric reported pool values from the node;
- arithmetic overflow at any stage.

A transaction that cannot be fully parsed invalidates its block, and a block that cannot be
parsed invalidates the reconciliation. **Partial results are never reported as successful.**

---

## 9. What this model does not establish

It reconstructs pool *changes* over a bounded interval from a declared anchor. It does not
derive supply from genesis, verify zero-knowledge proofs, validate consensus rules beyond the
arithmetic properties listed in §6, establish circuit soundness, determine whether historical
counterfeiting occurred, or replace full-node validation.

Agreement between the reconstruction and the node's aggregate means two independently written
decoders, reading the same public bytes, computed the same result. Disagreement means one of
them contains a defect, and locates where. Neither outcome is a proof of supply correctness,
and every report says so in its own text.

The full boundary, including what independence does and does not cover and what has not yet
been demonstrated, is in [`LIMITATIONS.md`](LIMITATIONS.md).
