//! Integer monetary values.
//!
//! Every monetary quantity in this crate is a [`Zatoshi`]. The type exists to make three
//! properties structural rather than a matter of review discipline:
//!
//! * no floating-point value can be converted into a monetary value;
//! * no arithmetic can overflow silently, because every operation is checked;
//! * no monetary value can be serialized as a JSON number.
//!
//! The last point is a correctness requirement, not a style preference. RFC 8785
//! canonicalization coerces JSON numbers to IEEE-754 doubles, which lose integer precision
//! above 2^53. Zatoshi values reach the same order of magnitude, so a numeric encoding
//! would be silently lossy inside the hashed report.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::ReconcileError;

/// Maximum representable Zcash money value, in zatoshi: 21,000,000 ZEC at 10^8 zatoshi
/// per ZEC. Chain value pool balances are bounded by this; individual deltas are signed
/// and bounded by its negation.
pub const MAX_MONEY: i64 = 21_000_000 * 100_000_000;

/// A signed integer quantity of zatoshi.
///
/// Negative values are meaningful: a pool delta is negative when value leaves the pool.
/// Pool *balances* are separately required to be non-negative, which is asserted by the
/// check registry rather than by this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Zatoshi(i64);

impl Zatoshi {
    pub const ZERO: Self = Self(0);

    /// Constructs a value without bounds checking.
    ///
    /// Used for intermediate deltas read from transaction data, which are validated
    /// against the money bounds separately so that an out-of-range field produces a
    /// diagnosable parser error rather than a construction failure with no context.
    pub const fn from_raw(value: i64) -> Self {
        Self(value)
    }

    /// Constructs a value, rejecting anything outside the representable money range.
    pub fn new_checked(value: i64) -> Result<Self, ReconcileError> {
        if !(-MAX_MONEY..=MAX_MONEY).contains(&value) {
            return Err(ReconcileError::ValueOutOfBounds { value });
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> i64 {
        self.0
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Returns whether the value lies within the representable money range.
    pub const fn is_within_money_bounds(self) -> bool {
        self.0 <= MAX_MONEY && self.0 >= -MAX_MONEY
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, ReconcileError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(ReconcileError::ArithmeticOverflow)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, ReconcileError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(ReconcileError::ArithmeticOverflow)
    }

    /// Negates the value, which is how a `valueBalance` field becomes a pool delta.
    ///
    /// A bundle's `valueBalance` is positive when value leaves the shielded pool, so the
    /// pool delta is its negation.
    pub fn checked_neg(self) -> Result<Self, ReconcileError> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or(ReconcileError::ArithmeticOverflow)
    }

    /// Sums an iterator with overflow checking at every step.
    pub fn try_sum<I: IntoIterator<Item = Self>>(values: I) -> Result<Self, ReconcileError> {
        values
            .into_iter()
            .try_fold(Self::ZERO, |acc, value| acc.checked_add(value))
    }
}

impl fmt::Display for Zatoshi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for Zatoshi {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Zatoshi {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(ZatoshiVisitor)
    }
}

struct ZatoshiVisitor;

impl Visitor<'_> for ZatoshiVisitor {
    type Value = Zatoshi;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a decimal integer encoded as a JSON string")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Zatoshi, E> {
        value
            .parse::<i64>()
            .map(Zatoshi)
            .map_err(|_| E::custom(format!("not a decimal integer: {value}")))
    }

    /// Rejects JSON numbers explicitly so that a schema regression surfaces as a
    /// deserialization failure rather than a silently truncated value.
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Zatoshi, E> {
        Err(E::custom(
            "monetary values must be JSON strings, not numbers",
        ))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Zatoshi, E> {
        Err(E::custom(
            "monetary values must be JSON strings, not numbers",
        ))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Zatoshi, E> {
        Err(E::custom(
            "monetary values must be JSON strings, not numbers",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_money_matches_protocol_constant() {
        assert_eq!(MAX_MONEY, 2_100_000_000_000_000);
    }

    #[test]
    fn addition_overflow_is_an_error_not_a_wrap() {
        let max = Zatoshi::from_raw(i64::MAX);
        assert!(matches!(
            max.checked_add(Zatoshi::from_raw(1)),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn subtraction_overflow_is_an_error_not_a_wrap() {
        let min = Zatoshi::from_raw(i64::MIN);
        assert!(matches!(
            min.checked_sub(Zatoshi::from_raw(1)),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn negation_of_min_is_an_error() {
        assert!(matches!(
            Zatoshi::from_raw(i64::MIN).checked_neg(),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn bounds_check_rejects_values_beyond_max_money() {
        assert!(Zatoshi::new_checked(MAX_MONEY).is_ok());
        assert!(Zatoshi::new_checked(-MAX_MONEY).is_ok());
        assert!(matches!(
            Zatoshi::new_checked(MAX_MONEY + 1),
            Err(ReconcileError::ValueOutOfBounds { .. })
        ));
    }

    #[test]
    fn serializes_as_a_json_string() {
        let json = serde_json::to_string(&Zatoshi::from_raw(-17_600_000_000_000)).unwrap();
        assert_eq!(json, "\"-17600000000000\"");
    }

    #[test]
    fn deserializes_from_a_json_string() {
        let value: Zatoshi = serde_json::from_str("\"-17600000000000\"").unwrap();
        assert_eq!(value.get(), -17_600_000_000_000);
    }

    #[test]
    fn rejects_a_json_number() {
        assert!(serde_json::from_str::<Zatoshi>("17600000000000").is_err());
    }

    #[test]
    fn round_trips_values_above_the_double_precision_limit() {
        // 2^53 + 1 is not representable as an f64; a numeric encoding would corrupt it.
        let value = Zatoshi::from_raw(9_007_199_254_740_993);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<Zatoshi>(&json).unwrap(), value);
    }

    #[test]
    fn sum_propagates_overflow() {
        let values = [Zatoshi::from_raw(i64::MAX), Zatoshi::from_raw(1)];
        assert!(Zatoshi::try_sum(values).is_err());
    }

    #[test]
    fn sum_of_empty_is_zero() {
        assert_eq!(Zatoshi::try_sum([]).unwrap(), Zatoshi::ZERO);
    }
}
