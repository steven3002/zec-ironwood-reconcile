# Limitations

The boundary of what `zec-ironwood-reconcile` output may be cited for.

Section 1 is part of the report schema. Those nine statements are compiled into the binary
and written verbatim into every report the tool produces, in both the JSON and the Markdown
rendering, so a report cannot circulate separated from its own limitations. They are
reproduced here for a reader who has not yet run the tool. If this file and a report ever
disagree, the report is authoritative, and `tests/completeness.rs` fails the build, because
the disagreement is a defect in this file.

Sections 2 and 3 are further limitations that are documented rather than embedded in the
schema.

---

## 1. The limitations every report carries

- Does not verify zero-knowledge proofs.
- Does not independently validate all Zcash consensus rules.
- Does not prove whether historical counterfeiting occurred.
- Does not prove total Zcash supply from genesis.
- Does not prove the Ironwood circuit is sound.
- Does not prove that the Orchard pool was never exploited.
- Does not replace full-node consensus validation.
- Does not constitute a formal security audit.
- The Orchard outflow and Ironwood inflow figures are separate observations, not a balance.
  Ironwood also receives newly issued value directly from coinbase transactions, so Ironwood
  inflow exceeding Orchard outflow is expected and is not unexplained supply.

### What each one means in practice

**Zero-knowledge proofs.** The tool reads the `valueBalance` field of a transaction's
Orchard and Ironwood bundles. That field is a cleartext signed integer in the consensus
encoding. Nothing verifies the proof that the bundle is internally consistent, so a
transaction whose proof was invalid but whose `valueBalance` was well formed contributes to
the reconstruction exactly as a valid one would.

**Consensus rules.** Three properties specific to the NU6.3 upgrade are checked, the
Orchard withdrawal-only rule per transaction, pool non-negativity, and the Ironwood pool
being empty before activation. Everything else a full node validates is not checked:
signatures, script execution, difficulty, timestamps, note commitments, nullifier
uniqueness, and the rest.

**Historical counterfeiting.** The tool reconstructs *changes* across a bounded interval
from a declared starting anchor. It says nothing about any height outside that interval.

**Supply from genesis.** The anchor balances are taken from the capturing node and are an
input to the arithmetic, not a result of it. See section 2 for the consequence.

**Circuit soundness and Orchard exploitation.** Both are properties of cryptographic
constructions this tool does not examine.

**Full-node validation.** Agreement between the reconstruction and the node means two
independently written decoders read the same public bytes and reached the same figure.
Disagreement means one of them contains a defect, and locates the block where it first
shows. Neither outcome establishes that the chain is valid.

**Formal security audit.** No third party has reviewed this code.

**Pool flows are not a balance.** This is the correction of a model the project itself once
held. At testnet height 4,134,683 the Ironwood pool gained 125,000,000 zatoshi in a block
containing a single transaction, the coinbase, which distributed that height's block
subsidy of 156,250,000 zatoshi as transparent +12,500,000, lockbox +18,750,000 and ironwood
+125,000,000, while Orchard released nothing. Ironwood inflow is therefore not bounded by
Orchard outflow, and the two figures are reported side by side without any asserted
relationship between them.

Mainnet funded Ironwood the other way and confirms the same conclusion from the opposite
side. At mainnet height 3,428,144 Ironwood gained 1,000,000 zatoshi while Orchard released
1,020,000, so here the outflow *exceeded* the inflow. Neither ordering is a rule. Both
networks are now represented in the captured evidence precisely so that neither observation
can be mistaken for one.

---

## 2. Limitations of the method

**A uniformly shifted bundle reconciles clean.** Because the tool reconstructs changes from
a declared anchor rather than deriving supply from genesis, a bundle in which the anchor
balance *and* every per-height reported balance are shifted by the same constant produces a
fully passing report. Every delta still agrees, and every balance still agrees with the
shifted figure it is compared against. This is the operational consequence of "does not
prove total Zcash supply from genesis". Defending against it requires checking the anchor
against a source other than the bundle.

**The branch-identifier readback is an assertion for some versions and a tautology for
others.** Version 5 and 6 transactions carry their consensus branch identifier in their own
bytes, so reading it back and comparing it against the identifier expected for the height is
a real check. Version 3 and 4 transactions do not: the identifier is supplied to the reader
and echoed back unchanged, so for those the comparison cannot fail. It is still performed,
so that a block mixing versions does not have the assertion silently skipped where it holds.

**The `monitored` flag carries no information.** Zebra builds every pool entry through one
constructor that sets `monitored` to `chainValueZat != 0`, so the flag restates whether the
balance is non-zero and says nothing about which pools the node tracks. It is stored in
evidence because it is part of the response. No check reads it, and a reader should not
treat `false` as marking a balance unmeasured.

**`--tip-distance 100` is a conservative choice, not a derived one.** The capture guard
requires the node's tip to be at least this far beyond the requested interval, so that a
reorganisation is unlikely to invalidate what was captured. No specification states a depth
beyond which a Zcash reorganisation cannot occur, and no authoritative source for one was
found. The default is a judgement, and the flag exists so an operator can substitute their
own.

**The comparison offers one independent axis per height, not two.** A report compares the
reconstruction against the node on two axes at every height: reconstructed block delta
against `valueDeltaZat`, and reconstructed running balance against `chainValueZat`. These are
not two independent confirmations. Zebra derives `valueDeltaZat(h)` by subtracting its own
stored `chainValueZat(h-1)` from `chainValueZat(h)`, so given a correct anchor the balance
axis is entailed by the delta axis. Both are still computed and both are worth having, the
delta axis localises a divergence to the block that caused it, and the balance axis catches
an anchor that does not line up, but a reader should count one independent comparison per
height. Both axes do compare *this crate's reconstruction* against the node's figures, so
neither is self-referential.

**Independence from the implementation being checked is real but partial.** No Zebra crate
appears in this binary's dependency graph, and Zebra decodes transactions and computes pool
value balances with its own hand-written implementation in `zebra-chain`. That is what makes
the comparison meaningful. It is not true that the two share no code: Zebra depends on
`zcash_primitives` at the same version this tool links, and most of the two locked
dependency graphs coincide, including the cryptographic crates `halo2_proofs`,
`pasta_curves` and `reddsa`. Those crates do not decode a value balance, both sides read it
as a little-endian `i64` in their own code, but a defect inside them would be invisible to
both.

**Scope of the interval.** A capture is bounded, and the tool will refuse an interval larger
than its configured maximum. Results describe the captured interval and nothing outside it.

---

## 3. What has not been done

Stated because a limitations document that omits the status of its own evidence is
incomplete.

- **No independent party has reproduced a result.** This is the largest open gap and no
  amount of further work by the publisher closes it.
- **No mainnet report hash has been reproduced outside the publisher's own machines.** Three
  mainnet bundles reconcile identically on two hosts, but those hosts run the same Ubuntu
  release on the same architecture from the same pinned toolchain and produce byte-identical
  binaries. That demonstrates the evidence survives a change of host, and nothing about a
  change of platform.
- **macOS and `aarch64` have never run this suite.** A CI job covering `windows-latest` and
  `macos-latest` is configured and has never executed. Cross-platform determinism is
  demonstrated for the testnet report hash only, on Ubuntu, WSL2 and native Windows, all
  `x86_64`.
- **The mainnet evidence covers 106 distinct blocks.** Six across the activation boundary and
  100 in steady state, out of a chain more than 3.4 million blocks long. Results describe the
  captured intervals and nothing outside them.

What *is* demonstrated is the complete loop on real chain data from **both networks**:
capture, reconcile, archive, publish a hash, verify offline, over intervals spanning the NU6.3
activation boundary on each, over the first real Ironwood inflow on each, and over 100
steady-state mainnet blocks, with the reconstruction matching the figures the network reports
at every height. See [`REPRODUCING.md`](REPRODUCING.md) for what a reproduction must report
for it to count.
