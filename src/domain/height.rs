//! Block heights and the bounded interval being reconciled.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ReconcileError;

/// A block height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlockHeight(u32);

impl BlockHeight {
    pub const fn new(height: u32) -> Self {
        Self(height)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub fn checked_next(self) -> Result<Self, ReconcileError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(ReconcileError::ArithmeticOverflow)
    }

    pub fn checked_previous(self) -> Result<Self, ReconcileError> {
        self.0
            .checked_sub(1)
            .map(Self)
            .ok_or(ReconcileError::ArithmeticOverflow)
    }
}

impl fmt::Display for BlockHeight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The interval a bundle covers.
///
/// The anchor is the block immediately preceding `start`. Its balances are the declared
/// starting point of all subsequent arithmetic; the tool reconstructs changes relative to
/// it and does not derive it from genesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeightInterval {
    anchor_height: BlockHeight,
    start_height: BlockHeight,
    end_height: BlockHeight,
}

impl HeightInterval {
    /// Constructs an interval from inclusive bounds, deriving the anchor as `start - 1`.
    pub fn new(start_height: BlockHeight, end_height: BlockHeight) -> Result<Self, ReconcileError> {
        if end_height < start_height {
            return Err(ReconcileError::InvalidInterval {
                reason: format!("end height {end_height} precedes start height {start_height}"),
            });
        }

        let anchor_height =
            start_height
                .checked_previous()
                .map_err(|_| ReconcileError::InvalidInterval {
                    reason: "start height 0 has no preceding anchor block".to_owned(),
                })?;

        Ok(Self {
            anchor_height,
            start_height,
            end_height,
        })
    }

    pub const fn anchor_height(self) -> BlockHeight {
        self.anchor_height
    }

    pub const fn start_height(self) -> BlockHeight {
        self.start_height
    }

    pub const fn end_height(self) -> BlockHeight {
        self.end_height
    }

    /// Number of blocks in the interval, inclusive of both bounds.
    pub fn block_count(self) -> u32 {
        self.end_height
            .get()
            .saturating_sub(self.start_height.get())
            .saturating_add(1)
    }

    pub fn heights(self) -> impl Iterator<Item = BlockHeight> {
        (self.start_height.get()..=self.end_height.get()).map(BlockHeight::new)
    }

    pub fn contains(self, height: BlockHeight) -> bool {
        height >= self.start_height && height <= self.end_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval(start: u32, end: u32) -> HeightInterval {
        HeightInterval::new(BlockHeight::new(start), BlockHeight::new(end)).unwrap()
    }

    #[test]
    fn anchor_is_the_block_before_start() {
        let interval = interval(3_428_143, 3_429_143);
        assert_eq!(interval.anchor_height().get(), 3_428_142);
    }

    #[test]
    fn block_count_is_inclusive_of_both_bounds() {
        assert_eq!(interval(3_428_143, 3_429_143).block_count(), 1001);
        assert_eq!(interval(3_428_143, 3_428_143).block_count(), 1);
        assert_eq!(interval(3_428_143, 3_428_243).block_count(), 101);
    }

    #[test]
    fn heights_yields_every_block_once() {
        let heights: Vec<u32> = interval(100, 104).heights().map(BlockHeight::get).collect();
        assert_eq!(heights, vec![100, 101, 102, 103, 104]);
    }

    #[test]
    fn end_before_start_is_rejected() {
        let result = HeightInterval::new(BlockHeight::new(200), BlockHeight::new(100));
        assert!(matches!(
            result,
            Err(ReconcileError::InvalidInterval { .. })
        ));
    }

    #[test]
    fn genesis_start_has_no_anchor() {
        let result = HeightInterval::new(BlockHeight::new(0), BlockHeight::new(10));
        assert!(matches!(
            result,
            Err(ReconcileError::InvalidInterval { .. })
        ));
    }

    #[test]
    fn containment_excludes_the_anchor() {
        let interval = interval(3_428_143, 3_429_143);
        assert!(!interval.contains(interval.anchor_height()));
        assert!(interval.contains(interval.start_height()));
        assert!(interval.contains(interval.end_height()));
    }
}
