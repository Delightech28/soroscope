//! Resource-fee quoting, including an *estimate* of the refundable component.
//!
//! # What gets refunded
//!
//! On Soroban, the resource fee a transaction pays is not all gone forever. Rent
//! paid for **temporary** entries is returned when those entries expire, because
//! a temporary entry that is gone at the end of its TTL has occupied no space
//! after that point. Rent paid for **persistent** entries is *not* refunded —
//! that entry is the caller's data and keeps occupying the ledger.
//!
//! So the refundable part of a resource fee is exactly:
//!
//! ```text
//! estimated_refund = ceil(temp_rent_bytes * fee_per_write_1kb
//!                         / (data_size_1kb_increment * temporary_rent_rate_denominator))
//! ```
//!
//! which is the same expression `soroban-env-host::fees::compute_rent_fee` uses
//! for temporary entries. Note that `temporary_rent_rate_denominator` (4206) is
//! exactly twice `persistent_rent_rate_denominator` (2103) on pubnet, so rent on
//! a temporary byte costs half of rent on a persistent byte.
//!
//! # Why this is an estimate
//!
//! Two quantities are needed and the RPC only gives us one of them:
//!
//! * `cost.rentBytes` — the total rent the node charged. The RPC reports this.
//! * The **durability split** — how much of that rent was temporary vs
//!   persistent. The RPC does *not* report this.
//!
//! [`DurabilitySplit`] is therefore an explicit input, and when it is not known
//! the refund is `None` rather than `0`. Returning `0` would be a lie in the
//! dangerous direction: it would render as "this write refunds nothing" when we
//! actually mean "we do not know", and callers that optimise against refunds
//! would silently keep the pessimistic gross fee. Returning `None` lets the UI
//! say "refund unknown" and the caller fall back to the gross fee.
//!
//! Out of scope (deliberately): modelling *when* a refund actually lands, or
//! discounting it by the number of ledgers until expiry. The refund is reported
//! at face value and is always labelled as an estimate via
//! [`ResourceFeeQuote::refund_is_estimate`].

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use crate::simulation::SimulationStateSnapshot;

/// The checked-in pubnet fee parameters. Compiled into the binary so a quote is
/// reproducible and reviewable in a diff rather than depending on a live node.
pub const SOROBAN_FEE_CONFIG_JSON: &str = include_str!("../config/soroban-fees.json");

/// Fee parameters for one network, as checked in at
/// `core/config/soroban-fees.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SorobanFeeConfig {
    pub network: String,
    pub captured_at: String,
    pub protocol: u32,
    pub source: String,
    pub fee_per_instruction_increment: i64,
    pub instructions_increment: i64,
    pub fee_per_read_entry: i64,
    pub fee_per_write_entry: i64,
    pub fee_per_read_1kb: i64,
    pub fee_per_write_1kb: i64,
    pub fee_per_historical_1kb: i64,
    pub fee_per_contract_event_1kb: i64,
    pub fee_per_transaction_size_1kb: i64,
    pub data_size_1kb_increment: i64,
    pub tx_base_result_size: i64,
    pub ttl_entry_size: i64,
    pub persistent_rent_rate_denominator: i64,
    pub temporary_rent_rate_denominator: i64,
}

impl SorobanFeeConfig {
    /// Parse a config from raw JSON.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("invalid soroban-fees.json: {e}"))
    }

    /// The config compiled into the binary.
    pub fn checked_in() -> &'static SorobanFeeConfig {
        static CONFIG: OnceLock<SorobanFeeConfig> = OnceLock::new();
        CONFIG.get_or_init(|| {
            Self::from_json(SOROBAN_FEE_CONFIG_JSON).expect("built-in soroban-fees.json must parse")
        })
    }

    /// Bytes of temporary entry rent that a single ledger of TTL costs, i.e. the
    /// divisor in the rent formula.
    fn temporary_rent_divisor(&self) -> u128 {
        self.temporary_rent_divisor_with(self.temporary_rent_rate_denominator)
    }

    /// Bytes of persistent entry rent that a single ledger of TTL costs.
    fn persistent_rent_divisor(&self) -> u128 {
        self.temporary_rent_divisor_with(self.persistent_rent_rate_denominator)
    }

    fn temporary_rent_divisor_with(&self, rate_denominator: i64) -> u128 {
        let kb = self.data_size_1kb_increment.max(1) as u128;
        let denominator = rate_denominator.max(1) as u128;
        kb.saturating_mul(denominator).max(1)
    }
}

/// `ceil(numerator / denominator)` in `u128` space, saturating to `u64::MAX`.
///
/// A profiler must never panic or wrap on hostile numbers coming off the wire:
/// a wrapped fee reads as *cheap*, which is the one answer a cost tool must not
/// invent.
fn div_ceil(numerator: u128, denominator: u128) -> u64 {
    if denominator == 0 {
        return 0;
    }
    let quotient = numerator / denominator;
    let result = if numerator % denominator == 0 {
        quotient
    } else {
        quotient.saturating_add(1)
    };
    result.min(u64::MAX as u128) as u64
}

/// Per-component resource fee, in stroops. Sums to the gross fee.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceFeeBreakdown {
    pub instructions: u64,
    pub read_entries: u64,
    pub write_entries: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    /// Archival/historical write cost of the transaction result.
    pub historical_bytes: u64,
    /// Bandwidth cost of the transaction envelope itself.
    pub bandwidth_bytes: u64,
    pub contract_events: u64,
    pub temporary_rent: u64,
    pub persistent_rent: u64,
}

impl ResourceFeeBreakdown {
    pub fn total(&self) -> u64 {
        self.instructions
            .saturating_add(self.read_entries)
            .saturating_add(self.write_entries)
            .saturating_add(self.read_bytes)
            .saturating_add(self.write_bytes)
            .saturating_add(self.historical_bytes)
            .saturating_add(self.bandwidth_bytes)
            .saturating_add(self.contract_events)
            .saturating_add(self.temporary_rent)
            .saturating_add(self.persistent_rent)
    }
}

/// How the written ledger bytes split between temporary and persistent entries.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct DurabilitySplit {
    pub temporary_write_bytes: u64,
    pub persistent_write_bytes: u64,
    /// `false` when the simulation did not expose a durability breakdown. When
    /// `false` the byte counts above are meaningless and the refund is `None`.
    pub known: bool,
}

impl DurabilitySplit {
    pub fn unknown() -> Self {
        Self {
            known: false,
            ..Default::default()
        }
    }

    pub fn temporary_only(bytes: u64) -> Self {
        Self {
            temporary_write_bytes: bytes,
            persistent_write_bytes: 0,
            known: true,
        }
    }

    pub fn persistent_only(bytes: u64) -> Self {
        Self {
            temporary_write_bytes: 0,
            persistent_write_bytes: bytes,
            known: true,
        }
    }

    /// A split of `temporary` of `total` bytes.
    pub fn mixed(temporary: u64, persistent: u64) -> Self {
        Self {
            temporary_write_bytes: temporary,
            persistent_write_bytes: persistent,
            known: true,
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.temporary_write_bytes
            .saturating_add(self.persistent_write_bytes)
    }

    /// Recover a durability split from the ledger entries a simulation touched.
    ///
    /// Each snapshot key is a base64 `LedgerKey`; `ContractData` keys carry their
    /// durability in the clear, and the paired base64 `LedgerEntry` gives a size
    /// we can weight by. Entry size is approximated by the XDR length of the
    /// stored entry — good enough to establish a *ratio* between durabilities,
    /// which is all the refund estimate needs.
    ///
    /// Returns [`DurabilitySplit::unknown`] when the snapshot is missing, holds
    /// no `ContractData` keys, or a key fails to decode. Guessing here would
    /// produce a confidently wrong refund.
    pub fn from_state_snapshot(snapshot: Option<&SimulationStateSnapshot>) -> Self {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use soroban_sdk::xdr::{ContractDataDurability, LedgerKey, Limits, ReadXdr};

        let Some(snapshot) = snapshot else {
            return Self::unknown();
        };

        let mut temporary = 0u64;
        let mut persistent = 0u64;
        let mut saw_contract_data = false;

        for (key_b64, entry_b64) in &snapshot.ledger_entries {
            let Ok(key_bytes) = BASE64.decode(key_b64) else {
                continue;
            };
            let Ok(LedgerKey::ContractData(contract_data)) =
                LedgerKey::from_xdr(&key_bytes, Limits::none())
            else {
                continue;
            };
            saw_contract_data = true;

            // Weight by the stored entry's encoded size; fall back to the key
            // size alone if the value is unreadable, so a partial snapshot
            // still yields a split rather than nothing.
            let size = match BASE64.decode(entry_b64).map(|bytes| bytes.len() as u64) {
                Ok(len) => len.max(1),
                Err(_) => 1,
            };

            match contract_data.durability {
                ContractDataDurability::Temporary => temporary = temporary.saturating_add(size),
                ContractDataDurability::Persistent => persistent = persistent.saturating_add(size),
            }
        }

        if !saw_contract_data || temporary.saturating_add(persistent) == 0 {
            return Self::unknown();
        }

        Self::mixed(temporary, persistent)
    }
}

/// Why [`ResourceFeeQuote::estimated_refund`] is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    /// The durability split and rent total were both available.
    Estimated,
    /// The simulation did not expose a durability breakdown.
    UnknownDurability,
    /// The node did not report `cost.rentBytes`, so there is no rent to split.
    UnknownRentBytes,
}

/// The resource costs to price. Mirrors what the RPC reports, plus the two
/// entry counts the RPC omits from `SorobanResources` (defaulted to zero when
/// unknown).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct FeeQuoteInput {
    pub cpu_instructions: u64,
    pub ledger_read_bytes: u64,
    pub ledger_write_bytes: u64,
    pub transaction_size_bytes: u64,
    pub read_entries: u64,
    pub write_entries: u64,
    pub contract_event_bytes: u64,
    /// Total rent the node charged (`cost.rentBytes`). `None` when the RPC did
    /// not report it.
    pub rent_bytes: Option<u64>,
}

impl FeeQuoteInput {
    /// Build from the resources on a simulation result.
    pub fn from_soroban_resources(
        resources: &crate::simulation::SorobanResources,
        rent_bytes: Option<u64>,
    ) -> Self {
        Self {
            cpu_instructions: resources.cpu_instructions,
            ledger_read_bytes: resources.ledger_read_bytes,
            ledger_write_bytes: resources.ledger_write_bytes,
            transaction_size_bytes: resources.transaction_size_bytes,
            read_entries: 0,
            write_entries: 0,
            contract_event_bytes: 0,
            rent_bytes,
        }
    }
}

/// A priced simulation: the gross fee always, plus an estimated refund when the
/// durability split is known.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceFeeQuote {
    /// Full resource fee. Always present, even when the refund is unknown, so
    /// callers can fall back to the worst case.
    pub gross_resource_fee: u64,
    /// Refundable portion of the gross fee. `None` means "unknown", **not** zero.
    pub estimated_refund: Option<u64>,
    /// `gross_resource_fee - estimated_refund`. `None` when the refund is unknown.
    pub estimated_net: Option<u64>,
    pub refund_status: RefundStatus,
    /// Always `true`. The refund is computed from a durability *estimate* and
    /// from a checked-in config, not from what the node will actually credit.
    pub refund_is_estimate: bool,
    /// `false` when rent bytes were unavailable, in which case
    /// `gross_resource_fee` omits rent and is a lower bound.
    pub gross_includes_rent: bool,
    pub durability_split: DurabilitySplit,
    /// Split of `cost.rentBytes` attributed to temporary entries, when known.
    pub temporary_rent_bytes: Option<u64>,
    pub breakdown: ResourceFeeBreakdown,
}

impl ResourceFeeQuote {
    /// Price `input` and, where possible, estimate the refund implied by
    /// `durability_split`.
    pub fn estimate(
        input: &FeeQuoteInput,
        durability_split: DurabilitySplit,
        config: &SorobanFeeConfig,
    ) -> Self {
        let (temporary_rent_bytes, persistent_rent_bytes) = match input.rent_bytes {
            Some(total) => match split_rent_bytes(total, durability_split) {
                Some((temporary, persistent)) => (temporary, persistent),
                // Durability unknown: price *all* rent at the persistent rate.
                // That is the pessimistic reading, so `gross` stays a safe
                // upper bound for a caller with no better information.
                None => (0, total),
            },
            None => (0, 0),
        };

        let breakdown = ResourceFeeBreakdown {
            instructions: fee_per_increment(
                input.cpu_instructions as u128,
                config.fee_per_instruction_increment as u128,
                config.instructions_increment as u128,
            ),
            // A write entry is also billed as a read entry, matching
            // `soroban-env-host::fees::compute_transaction_resource_fee`.
            read_entries: (config.fee_per_read_entry.max(0) as u64)
                .saturating_mul(input.read_entries.saturating_add(input.write_entries)),
            write_entries: (config.fee_per_write_entry.max(0) as u64)
                .saturating_mul(input.write_entries),
            read_bytes: fee_per_increment(
                input.ledger_read_bytes as u128,
                config.fee_per_read_1kb as u128,
                config.data_size_1kb_increment as u128,
            ),
            write_bytes: fee_per_increment(
                input.ledger_write_bytes as u128,
                config.fee_per_write_1kb as u128,
                config.data_size_1kb_increment as u128,
            ),
            historical_bytes: fee_per_increment(
                input
                    .transaction_size_bytes
                    .saturating_add(config.tx_base_result_size.max(0) as u64)
                    as u128,
                config.fee_per_historical_1kb as u128,
                config.data_size_1kb_increment as u128,
            ),
            bandwidth_bytes: fee_per_increment(
                input.transaction_size_bytes as u128,
                config.fee_per_transaction_size_1kb as u128,
                config.data_size_1kb_increment as u128,
            ),
            contract_events: fee_per_increment(
                input.contract_event_bytes as u128,
                config.fee_per_contract_event_1kb as u128,
                config.data_size_1kb_increment as u128,
            ),
            temporary_rent: rent_fee(
                temporary_rent_bytes,
                config.fee_per_write_1kb,
                config.temporary_rent_divisor(),
            ),
            persistent_rent: rent_fee(
                persistent_rent_bytes,
                config.fee_per_write_1kb,
                config.persistent_rent_divisor(),
            ),
        };

        let gross_resource_fee = breakdown.total();

        let (estimated_refund, refund_status) = match (input.rent_bytes, durability_split.known) {
            (Some(total), true) => {
                let temporary = split_rent_bytes(total, durability_split)
                    .map(|(temporary, _)| temporary)
                    .unwrap_or(0);
                (
                    Some(rent_fee(
                        temporary,
                        config.fee_per_write_1kb,
                        config.temporary_rent_divisor(),
                    )),
                    RefundStatus::Estimated,
                )
            }
            (Some(_), false) => (None, RefundStatus::UnknownDurability),
            (None, _) => (None, RefundStatus::UnknownRentBytes),
        };

        let estimated_net =
            estimated_refund.map(|refund| gross_resource_fee.saturating_sub(refund));

        Self {
            gross_resource_fee,
            estimated_refund,
            estimated_net,
            refund_status,
            refund_is_estimate: true,
            gross_includes_rent: input.rent_bytes.is_some(),
            durability_split,
            temporary_rent_bytes: if refund_status == RefundStatus::Estimated {
                split_rent_bytes(input.rent_bytes.unwrap_or(0), durability_split)
                    .map(|(temporary, _)| temporary)
            } else {
                None
            },
            breakdown,
        }
    }
}

fn fee_per_increment(resource_value: u128, fee_rate: u128, increment: u128) -> u64 {
    div_ceil(resource_value.saturating_mul(fee_rate), increment.max(1))
}

fn rent_fee(rent_bytes: u64, fee_per_write_1kb: i64, divisor: u128) -> u64 {
    if rent_bytes == 0 {
        return 0;
    }
    div_ceil(
        (rent_bytes as u128).saturating_mul(fee_per_write_1kb.max(0) as u128),
        divisor,
    )
}

/// Split total rent bytes between durabilities in proportion to written bytes.
///
/// The two halves are made to sum back to `total` exactly (the persistent half
/// takes the remainder) so the gross fee is not inflated by a rounding pair.
fn split_rent_bytes(total: u64, durability_split: DurabilitySplit) -> Option<(u64, u64)> {
    if !durability_split.known {
        return None;
    }
    let split_total = durability_split.total_bytes();
    if split_total == 0 {
        // A known split of zero written bytes: no rent was paid, so the refund
        // is a definite zero rather than an unknown.
        return Some((0, 0));
    }
    let temporary = div_ceil(
        (total as u128).saturating_mul(durability_split.temporary_write_bytes as u128),
        split_total as u128,
    )
    .min(total);
    Some((temporary, total.saturating_sub(temporary)))
}

/// Convenience: derive a [`DurabilitySplit`] and price in one step from a
/// completed simulation.
pub fn quote_simulation(
    simulation: &crate::simulation::SimulationResult,
    rent_bytes: Option<u64>,
    config: &SorobanFeeConfig,
) -> ResourceFeeQuote {
    let input = FeeQuoteInput::from_soroban_resources(&simulation.resources, rent_bytes);
    let split = DurabilitySplit::from_state_snapshot(simulation.state_snapshot.as_ref());
    ResourceFeeQuote::estimate(&input, split, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config() -> &'static SorobanFeeConfig {
        SorobanFeeConfig::checked_in()
    }

    /// Resources shared by every fixture so only the rent terms move. Hand
    /// computed against the checked-in pubnet config:
    ///
    /// | term                        | math                                        | stroops |
    /// |-----------------------------|---------------------------------------------|---------|
    /// | instructions                | ceil(100000 * 25 / 10000)                   | 250     |
    /// | read entries                | 0                                           | 0       |
    /// | write entries               | 0                                           | 0       |
    /// | read bytes                  | ceil(1024 * 1786 / 1024)                    | 1786    |
    /// | write bytes                 | ceil(1024 * 12000 / 1024)                   | 12000   |
    /// | historical bytes            | ceil(1324 * 16235 / 1024)                   | 20992   |
    /// | bandwidth bytes             | ceil(1024 * 1624 / 1024)                    | 1624    |
    /// | contract events             | 0                                           | 0       |
    /// | **base (no rent)**          |                                             | 36652   |
    fn base_input(rent_bytes: Option<u64>) -> FeeQuoteInput {
        FeeQuoteInput {
            cpu_instructions: 100_000,
            ledger_read_bytes: 1024,
            ledger_write_bytes: 1024,
            transaction_size_bytes: 1024,
            read_entries: 0,
            write_entries: 0,
            contract_event_bytes: 0,
            rent_bytes,
        }
    }

    const BASE_FEE: u64 = 36_652;

    #[test]
    fn checked_in_config_parses_and_matches_pubnet_snapshot() {
        let c = config();
        assert_eq!(c.network, "pubnet");
        assert_eq!(c.protocol, 22);
        assert_eq!(c.fee_per_write_1kb, 12_000);
        assert_eq!(c.data_size_1kb_increment, 1024);
        // Temp rent is exactly half of persistent rent per byte on pubnet.
        assert_eq!(
            c.temporary_rent_rate_denominator,
            c.persistent_rent_rate_denominator * 2
        );
    }

    #[test]
    fn base_terms_are_hand_computed() {
        let quote = ResourceFeeQuote::estimate(
            &base_input(Some(0)),
            DurabilitySplit::temporary_only(1024),
            config(),
        );
        assert_eq!(quote.breakdown.instructions, 250);
        assert_eq!(quote.breakdown.read_bytes, 1786);
        assert_eq!(quote.breakdown.write_bytes, 12_000);
        assert_eq!(quote.breakdown.historical_bytes, 20_992);
        assert_eq!(quote.breakdown.bandwidth_bytes, 1624);
        assert_eq!(quote.breakdown.temporary_rent, 0);
        assert_eq!(quote.gross_resource_fee, BASE_FEE);
    }

    /// Temporary-only write of 10240 rent bytes.
    /// refund = ceil(10240 * 12000 / (1024 * 4206)) = ceil(122880000/4306944) = 29
    #[test]
    fn temporary_only_write_has_non_zero_refund() {
        let quote = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::temporary_only(10_240),
            config(),
        );
        assert_eq!(quote.refund_status, RefundStatus::Estimated);
        assert_eq!(quote.estimated_refund, Some(29));
        assert_eq!(quote.temporary_rent_bytes, Some(10_240));
        assert_eq!(quote.breakdown.temporary_rent, 29);
        assert_eq!(quote.breakdown.persistent_rent, 0);
        assert_eq!(quote.gross_resource_fee, BASE_FEE + 29);
        assert_eq!(quote.estimated_net, Some(BASE_FEE));
        assert!(quote.refund_is_estimate);
    }

    /// Same byte count, but durable: rent is billed at the persistent rate and
    /// nothing is ever refunded.
    /// persistent rent = ceil(10240 * 12000 / (1024 * 2103))
    ///                   = ceil(122880000/2153472) = 58
    #[test]
    fn persistent_only_write_of_same_size_has_smaller_or_zero_refund() {
        let persistent = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::persistent_only(10_240),
            config(),
        );
        assert_eq!(persistent.estimated_refund, Some(0));
        assert_eq!(persistent.breakdown.temporary_rent, 0);
        assert_eq!(persistent.breakdown.persistent_rent, 58);
        assert_eq!(persistent.gross_resource_fee, BASE_FEE + 58);
        assert_eq!(persistent.estimated_net, Some(BASE_FEE + 58));

        let temporary = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::temporary_only(10_240),
            config(),
        );
        assert!(temporary.estimated_refund > persistent.estimated_refund);
    }

    /// Half temporary, half persistent. The temporary rent bytes are
    /// ceil(10240 * 5120 / 10240) = 5120, so
    /// refund = ceil(5120 * 12000 / 4306944) = ceil(61440000/4306944) = 15
    /// and the persistent half is
    /// ceil(5120 * 12000 / 2153472) = ceil(61440000/2153472) = 29.
    #[test]
    fn mixed_writes_sum_both_durabilities() {
        let mixed = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::mixed(5_120, 5_120),
            config(),
        );
        assert_eq!(mixed.temporary_rent_bytes, Some(5_120));
        assert_eq!(mixed.breakdown.temporary_rent, 15);
        assert_eq!(mixed.breakdown.persistent_rent, 29);
        assert_eq!(mixed.estimated_refund, Some(15));
        assert_eq!(mixed.gross_resource_fee, BASE_FEE + 15 + 29);
        assert_eq!(mixed.estimated_net, Some(BASE_FEE + 29));

        // Refund lands strictly between the two pure cases.
        let temporary_only = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::temporary_only(10_240),
            config(),
        );
        let persistent_only = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::persistent_only(10_240),
            config(),
        );
        assert!(persistent_only.estimated_refund < mixed.estimated_refund);
        assert!(mixed.estimated_refund < temporary_only.estimated_refund);
    }

    #[test]
    fn unknown_durability_reports_null_refund_not_zero() {
        let quote = ResourceFeeQuote::estimate(
            &base_input(Some(10_240)),
            DurabilitySplit::unknown(),
            config(),
        );
        assert_eq!(quote.refund_status, RefundStatus::UnknownDurability);
        assert_eq!(quote.estimated_refund, None);
        assert_eq!(quote.estimated_net, None);
        assert_eq!(quote.temporary_rent_bytes, None);
        // Gross is still reported, priced at the pessimistic persistent rate.
        assert_eq!(quote.gross_resource_fee, BASE_FEE + 58);
        assert!(quote.gross_includes_rent);
    }

    #[test]
    fn missing_rent_bytes_reports_null_refund_and_lower_bound_gross() {
        let quote = ResourceFeeQuote::estimate(
            &base_input(None),
            DurabilitySplit::temporary_only(10_240),
            config(),
        );
        assert_eq!(quote.refund_status, RefundStatus::UnknownRentBytes);
        assert_eq!(quote.estimated_refund, None);
        assert_eq!(quote.estimated_net, None);
        assert!(!quote.gross_includes_rent);
        assert_eq!(quote.gross_resource_fee, BASE_FEE);
    }

    #[test]
    fn zero_rent_with_known_split_is_a_definite_zero_refund() {
        let quote = ResourceFeeQuote::estimate(
            &base_input(Some(0)),
            DurabilitySplit::mixed(0, 0),
            config(),
        );
        assert_eq!(quote.refund_status, RefundStatus::Estimated);
        assert_eq!(quote.estimated_refund, Some(0));
        assert_eq!(quote.gross_resource_fee, BASE_FEE);
    }

    #[test]
    fn unknown_snapshot_yields_unknown_split() {
        assert_eq!(
            DurabilitySplit::from_state_snapshot(None),
            DurabilitySplit::unknown()
        );
        let empty = SimulationStateSnapshot {
            ledger_entries: HashMap::new(),
            ttl_entries: HashMap::new(),
            latest_ledger: 1,
        };
        assert_eq!(
            DurabilitySplit::from_state_snapshot(Some(&empty)),
            DurabilitySplit::unknown()
        );
    }

    #[test]
    fn rent_split_sums_back_to_the_reported_total() {
        for total in [0u64, 1, 512, 10_240, 65_536, 1_000_000] {
            for (temporary, persistent) in [(1u64, 1u64), (3, 7), (10_240, 1), (1, 10_240)] {
                let Some((temp_bytes, pers_bytes)) =
                    split_rent_bytes(total, DurabilitySplit::mixed(temporary, persistent))
                else {
                    panic!("known split must split");
                };
                assert_eq!(
                    temp_bytes.saturating_add(pers_bytes),
                    total,
                    "split leaked {total} stroops of rent at {temporary}/{persistent}"
                );
            }
        }
    }

    #[test]
    fn hostile_numbers_do_not_wrap_to_a_cheap_fee() {
        let hostile = FeeQuoteInput {
            cpu_instructions: u64::MAX,
            ledger_read_bytes: u64::MAX,
            ledger_write_bytes: u64::MAX,
            transaction_size_bytes: u64::MAX,
            read_entries: 1_000_000,
            write_entries: 1_000_000,
            contract_event_bytes: u64::MAX,
            rent_bytes: Some(u64::MAX),
        };
        let quote = ResourceFeeQuote::estimate(
            &hostile,
            DurabilitySplit::mixed(u64::MAX, u64::MAX),
            config(),
        );

        // The point of this test: an overflowing fee must saturate at the
        // expensive end, never wrap round to something that reads as cheap.
        assert_eq!(quote.gross_resource_fee, u64::MAX);
        assert_eq!(quote.breakdown.total(), u64::MAX);
        assert!(quote.gross_resource_fee > 1_000_000_000_000_000);

        let refund = quote.estimated_refund.expect("split is known");
        assert!(refund > 0, "a saturating refund is never a free write");
        assert!(refund < quote.gross_resource_fee);
        assert_eq!(
            quote.estimated_net,
            Some(quote.gross_resource_fee.saturating_sub(refund))
        );
    }

    #[test]
    fn div_ceil_rounds_up_and_guards_zero_denominator() {
        assert_eq!(div_ceil(0, 5), 0);
        assert_eq!(div_ceil(5, 5), 1);
        assert_eq!(div_ceil(6, 5), 2);
        assert_eq!(div_ceil(7, 0), 0);
        assert_eq!(div_ceil(u128::MAX, 1), u64::MAX);
    }

    /// The RPC reports `cost.rentBytes` as a decimal string. Without it the
    /// refund can never be estimated, so this is the field that decides whether
    /// a profile gets a real number or an honest `null`.
    #[test]
    fn rent_bytes_are_read_from_the_rpc_cost_object() {
        use crate::simulation::{SimulationEngine, SimulationRpcResult};

        let engine = SimulationEngine::new("https://example.invalid".to_string());
        let parse = |json: &str| -> Option<u64> {
            let rpc: SimulationRpcResult = serde_json::from_str(json).expect("rpc result");
            engine
                .parse_simulation_result(rpc)
                .expect("parses")
                .rent_bytes
        };

        assert_eq!(
            parse(
                r#"{"transactionData":"","latestLedger":1,
                    "cost":{"cpuInsns":"100000","memBytes":"2000","rentBytes":"10240"}}"#
            ),
            Some(10_240)
        );
        // Pre-protocol-20 nodes omit the field entirely.
        assert_eq!(
            parse(
                r#"{"transactionData":"","latestLedger":1,
                    "cost":{"cpuInsns":"100000","memBytes":"2000"}}"#
            ),
            None
        );
        // A junk value is a warning, not a panic and not a silent zero.
        assert_eq!(
            parse(
                r#"{"transactionData":"","latestLedger":1,
                    "cost":{"cpuInsns":"100000","memBytes":"2000","rentBytes":"nope"}}"#
            ),
            None
        );
    }

    /// End to end: a parsed simulation quotes with the durability split from its
    /// own ledger snapshot.
    #[test]
    fn a_parsed_simulation_quotes_end_to_end() {
        use crate::simulation::{SimulationEngine, SimulationRpcResult};

        let engine = SimulationEngine::new("https://example.invalid".to_string());
        let rpc: SimulationRpcResult = serde_json::from_str(
            r#"{"transactionData":"","latestLedger":1,
                "cost":{"cpuInsns":"100000","memBytes":"2000","rentBytes":"10240"}}"#,
        )
        .expect("rpc result");
        let simulation = engine.parse_simulation_result(rpc).expect("parses");

        // The footprint is empty, so no durability split is available and the
        // refund must be unknown rather than zero.
        let quote = quote_simulation(
            &simulation,
            simulation.rent_bytes,
            SorobanFeeConfig::checked_in(),
        );
        assert_eq!(quote.refund_status, RefundStatus::UnknownDurability);
        assert_eq!(quote.estimated_refund, None);
        assert!(quote.gross_includes_rent);
    }
}
