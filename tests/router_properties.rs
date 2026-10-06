//! Property-based invariants for the router, fixed-point math and scheduling.

use proptest::prelude::*;
use rusty_prism::order::Side;
use rusty_prism::router::fixed::{diff_millibps, sqrt_fixed, Fixed, PPM};
use rusty_prism::router::slicing::SliceSchedule;
use rusty_prism::router::sor::{OrderRequest, SmartOrderRouter};
use rusty_prism::router::symbol::SymbolRegistry;
use rusty_prism::router::venue::{FeeTier, Venue};

fn fixed(value: f64) -> Fixed {
    Fixed::from_f64(value)
}

proptest! {
    // Allocation never exceeds the parent, slices are non-negative, and the
    // parts sum to the allocated total.
    #[test]
    fn routing_allocation_invariants(
        quantity in 1.0f64..50_000.0,
        adv in 1.0f64..5_000_000.0,
        buy in any::<bool>(),
    ) {
        let mut router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
        let side = if buy { Side::Buy } else { Side::Sell };
        let request = OrderRequest::market("AAPL", side, quantity, 100.0).with_adv(adv);
        let plan = router.route(&request);

        prop_assert!(plan.allocated_qty <= plan.requested_qty);
        prop_assert_eq!(
            plan.allocated_qty + plan.unallocated_qty,
            plan.requested_qty
        );
        let summed = plan
            .allocations
            .iter()
            .fold(Fixed::ZERO, |acc, allocation| acc + allocation.quantity);
        prop_assert_eq!(summed, plan.allocated_qty);
        for allocation in &plan.allocations {
            prop_assert!(allocation.quantity > Fixed::ZERO);
            // 5% ADV participation cap.
            let cap = fixed(adv * 0.05);
            prop_assert!(allocation.quantity <= cap);
        }
    }

    // TWAP schedules conserve quantity exactly and never go negative.
    #[test]
    fn slicing_conserves_quantity(
        total in 0.0f64..1_000_000.0,
        duration in 0i64..600,
        interval in 1i64..120,
    ) {
        let schedule = SliceSchedule::twap(fixed(total), duration, interval);
        prop_assert!(!schedule.slices.is_empty());
        prop_assert_eq!(schedule.scheduled_quantity(), fixed(total));
        for slice in &schedule.slices {
            prop_assert!(slice.quantity >= Fixed::ZERO);
        }
    }

    // Multiplying then dividing by the same factor recovers the input.
    #[test]
    fn fixed_mul_div_roundtrip(value in 0.0f64..1_000_000.0, factor in 0.01f64..100.0) {
        let v = fixed(value);
        let f = fixed(factor);
        let round_trip = v.mul(f).div(f);
        // Error is relative to the magnitude of the value plus the fixed-point
        // rounding floor.
        let tolerance = Fixed::from_raw((v.raw().abs() / 1_000_000).max(2));
        let delta = if round_trip > v { round_trip - v } else { v - round_trip };
        prop_assert!(delta <= tolerance, "value={} factor={}", value, factor);
    }

    // The integer square root brackets the true root.
    #[test]
    fn sqrt_is_bracketed(value in 0.0f64..1_000_000.0) {
        let x = fixed(value);
        let root = sqrt_fixed(x);
        prop_assert!(root.mul(root) <= x + Fixed::from_raw(1));
        let next = root + Fixed::from_raw(1);
        prop_assert!(next.mul(next) >= x - Fixed::from_raw(1));
    }

    // diff_millibps carries the sign of value - reference and is zero when the
    // two prices are equal.
    #[test]
    fn diff_millibps_sign(a in 1.0f64..10_000.0, b in 1.0f64..10_000.0) {
        let delta = diff_millibps(fixed(a), fixed(b));
        let raw_a = fixed(a).raw();
        let raw_b = fixed(b).raw();
        if raw_a > raw_b {
            prop_assert!(delta >= 0, "a={a} b={b} delta={delta}");
        } else if raw_a < raw_b {
            prop_assert!(delta <= 0, "a={a} b={b} delta={delta}");
        } else {
            prop_assert_eq!(delta, 0);
        }
    }

    // Symbol interning is stable regardless of insertion order.
    #[test]
    fn symbol_interning_is_stable(names in prop::collection::vec("[A-Z]{2,4}", 1..8)) {
        let mut registry = SymbolRegistry::new();
        let mut first = std::collections::HashMap::new();
        for name in &names {
            let id = registry.intern(name);
            first.insert(name.clone(), id);
        }
        // Re-interning yields the same ids.
        for name in &names {
            prop_assert_eq!(registry.intern(name), first[name]);
            prop_assert_eq!(registry.name(first[name]).unwrap(), name.as_str());
        }
    }
}

#[test]
fn fee_tiers_are_monotone_for_descending_schedules() {
    let venue = Venue::new(0, "V", 10, 1.0).with_fee_tiers(vec![
        FeeTier::from_bps(0.0, 1.0, 3.0),
        FeeTier::from_bps(1_000_000.0, 0.5, 2.0),
        FeeTier::from_bps(5_000_000.0, 0.0, 1.0),
    ]);
    let mut previous = i64::MAX;
    for volume in [0, 999_999, 1_000_000, 5_000_000, 50_000_000] {
        let fee = venue.effective_fees_at(volume).taker_millibps;
        assert!(fee <= previous, "fee increased at volume {volume}");
        previous = fee;
    }
}

#[test]
fn fill_rate_is_bounded_by_ppm() {
    let mut router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
    let request = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(5_000_000.0);
    let plan = router.route(&request);
    let coverage = plan.allocated_qty.ratio_ppm(plan.requested_qty);
    assert!((0..=PPM).contains(&coverage));
}
