//! End-to-end execution: route through the SOR, send child orders over FIX
//! gateways, and feed the resulting book state back into the router.
//!
//! [`IntegratedRouter`] closes the loop that the pure router deliberately leaves
//! open:
//!
//! 1. refresh the router's market-data view from each venue gateway,
//! 2. route the parent order,
//! 3. submit each child allocation to its venue through the FIX gateway,
//! 4. fold the venue's post-trade book and traded volume back into the router.

use super::gateway::{ChildOrder, VenueExecution, VenueGateway};
use crate::order::Side;
use crate::router::fixed::{diff_millibps, Fixed, SCALE_SQUARED_I128};
use crate::router::slicing::SliceSchedule;
use crate::router::sor::{OrderRequest, RoutePlan, SmartOrderRouter};
use crate::router::symbol::{SymbolId, SymbolRegistry};
use crate::router::topology::simulated_topology;
use crate::router::venue::VenueId;

/// Aggregated result of routing and executing through the gateways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegratedReport {
    pub plan: RoutePlan,
    pub executions: Vec<VenueExecution>,
    pub requested_qty: Fixed,
    pub filled_qty: Fixed,
    pub avg_price: Fixed,
    pub arrival_price: Fixed,
    pub realized_slippage_millibps: i64,
    pub total_fees: i128,
}

impl IntegratedReport {
    pub fn unfilled_qty(&self) -> Fixed {
        (self.requested_qty - self.filled_qty).max(Fixed::ZERO)
    }

    pub fn realized_slippage_bps(&self) -> f64 {
        self.realized_slippage_millibps as f64 / crate::router::fixed::MILLI_BPS as f64
    }
}

/// A router wired to a set of venue gateways.
pub struct IntegratedRouter {
    router: SmartOrderRouter,
    gateways: Vec<Box<dyn VenueGateway>>,
}

impl IntegratedRouter {
    pub fn new(router: SmartOrderRouter, gateways: Vec<Box<dyn VenueGateway>>) -> Self {
        IntegratedRouter { router, gateways }
    }

    /// Build an integrated router and seed each gateway from a fresh simulated
    /// topology. The router and the venues therefore start with the same book.
    pub fn from_topology(symbols: &[&str], reference_price: f64, seed: u64) -> Self {
        let mut registry = SymbolRegistry::new();
        let venues = simulated_topology(&mut registry, symbols, reference_price, seed);

        let mut gateways: Vec<Box<dyn VenueGateway>> = Vec::with_capacity(venues.len());
        for venue in &venues {
            let mut gateway = super::gateway::FixVenueGateway::new(
                venue.id,
                &venue.name,
                venue.effective_fees().taker_millibps,
            );
            for (symbol_index, book) in venue.books.iter().enumerate() {
                if let (Some(name), Some(book)) = (registry.name(symbol_index as SymbolId), book) {
                    gateway.seed_book(name, book);
                }
            }
            gateways.push(Box::new(gateway));
        }

        IntegratedRouter::new(SmartOrderRouter::new(venues, registry), gateways)
    }

    pub fn router(&self) -> &SmartOrderRouter {
        &self.router
    }

    pub fn router_mut(&mut self) -> &mut SmartOrderRouter {
        &mut self.router
    }

    pub fn gateways(&self) -> &[Box<dyn VenueGateway>] {
        &self.gateways
    }

    /// Refresh the router's books from every gateway's current resting book.
    pub fn sync_market_data(&mut self, symbol: SymbolId) {
        let name = match self.router.symbols().name(symbol) {
            Some(name) => name.to_string(),
            None => return,
        };
        for gateway in &self.gateways {
            let book = gateway.book_snapshot(&name);
            if let Some(venue) = self
                .router
                .venues_mut()
                .iter_mut()
                .find(|venue| venue.id == gateway.venue_id())
            {
                venue.set_book(symbol, book);
            }
        }
    }

    /// Route one parent order and execute each child against its venue.
    pub fn execute(&mut self, request: &OrderRequest) -> IntegratedReport {
        let symbol = self
            .router
            .symbols()
            .id(&request.symbol)
            .unwrap_or(u32::MAX);
        self.sync_market_data(symbol);
        let plan = self.router.route(request);
        let executions = self.execute_plan(&plan, request, symbol);
        build_report(plan, executions, request)
    }

    /// Execute a slicing schedule through the gateways.
    pub fn execute_schedule(
        &mut self,
        request: &OrderRequest,
        schedule: &SliceSchedule,
    ) -> Vec<IntegratedReport> {
        let symbol = self
            .router
            .symbols()
            .id(&request.symbol)
            .unwrap_or(u32::MAX);
        let mut reports = Vec::with_capacity(schedule.slices.len());
        for slice in &schedule.slices {
            let mut child = request.clone();
            child.quantity = slice.quantity;
            child.horizon_s = schedule.interval_s.max(1);
            reports.push(self.execute_at(&child, symbol));
        }
        reports
    }

    fn execute_at(&mut self, request: &OrderRequest, symbol: SymbolId) -> IntegratedReport {
        self.sync_market_data(symbol);
        let plan = self.router.route(request);
        let executions = self.execute_plan(&plan, request, symbol);
        build_report(plan, executions, request)
    }

    fn execute_plan(
        &mut self,
        plan: &RoutePlan,
        request: &OrderRequest,
        _symbol: SymbolId,
    ) -> Vec<VenueExecution> {
        let mut executions = Vec::with_capacity(plan.allocations.len());
        for allocation in &plan.allocations {
            let child = ChildOrder {
                venue_id: allocation.venue_id,
                symbol: request.symbol.clone(),
                side: request.side,
                quantity: allocation.quantity,
                limit_price: request.limit_price,
            };
            let venue_id = allocation.venue_id;
            let execution = match self
                .gateways
                .iter_mut()
                .find(|gateway| gateway.venue_id() == venue_id)
            {
                Some(gateway) => gateway.submit(&child),
                None => continue,
            };
            if let Some(venue) = self
                .router
                .venues_mut()
                .iter_mut()
                .find(|venue| venue.id == venue_id)
            {
                venue.monthly_volume += execution.notional;
            }
            executions.push(execution);
        }
        executions
    }
}

/// Build a consolidated report from the gateway executions.
pub fn build_report(
    plan: RoutePlan,
    executions: Vec<VenueExecution>,
    request: &OrderRequest,
) -> IntegratedReport {
    let mut filled_qty = Fixed::ZERO;
    let mut notional: i128 = 0;
    let mut total_fees: i128 = 0;
    for execution in &executions {
        filled_qty += execution.filled_qty;
        notional += execution.notional;
        total_fees += super::gateway::execution_fee(execution);
    }
    let avg_price = if filled_qty.raw() > 0 {
        Fixed::from_raw((notional * SCALE_SQUARED_I128 / filled_qty.raw() as i128) as i64)
    } else {
        Fixed::ZERO
    };
    let realized_slippage_millibps = if filled_qty.raw() > 0 {
        let raw = diff_millibps(avg_price, request.arrival_price);
        match request.side {
            Side::Buy => raw,
            Side::Sell => -raw,
        }
    } else {
        0
    };

    IntegratedReport {
        plan,
        executions,
        requested_qty: request.quantity,
        filled_qty,
        avg_price,
        arrival_price: request.arrival_price,
        realized_slippage_millibps,
        total_fees,
    }
}

/// Convenience: route a single child to a specific venue id.
pub fn venue_ids(gateways: &[Box<dyn VenueGateway>]) -> Vec<VenueId> {
    gateways.iter().map(|gateway| gateway.venue_id()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integrated() -> IntegratedRouter {
        IntegratedRouter::from_topology(&["AAPL"], 100.0, 42)
    }

    #[test]
    fn test_end_to_end_market_order_fills() {
        let mut integrated = integrated();
        let request = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(5_000_000.0);
        let report = integrated.execute(&request);
        assert_eq!(report.filled_qty, Fixed::from_f64(1_000.0));
        assert!(!report.executions.is_empty());
        assert!(report.total_fees > 0);
        assert!(report.avg_price > Fixed::from_f64(100.0));
    }

    #[test]
    fn test_market_data_syncs_after_execution() {
        let mut integrated = integrated();
        let symbol = integrated.router().symbols().id("AAPL").unwrap();
        let before: Fixed = integrated
            .router()
            .venues()
            .iter()
            .filter_map(|venue| venue.book(symbol))
            .fold(Fixed::ZERO, |acc, book| {
                acc + book
                    .asks
                    .iter()
                    .fold(Fixed::ZERO, |inner, level| inner + level.size)
            });
        let request = OrderRequest::market("AAPL", Side::Buy, 2_000.0, 100.0).with_adv(5_000_000.0);
        integrated.execute(&request);
        // The next sync must observe the liquidity consumed at the venue.
        integrated.sync_market_data(symbol);
        let after: Fixed = integrated
            .router()
            .venues()
            .iter()
            .filter_map(|venue| venue.book(symbol))
            .fold(Fixed::ZERO, |acc, book| {
                acc + book
                    .asks
                    .iter()
                    .fold(Fixed::ZERO, |inner, level| inner + level.size)
            });
        assert!(after < before);
    }

    #[test]
    fn test_schedule_executes_multiple_child_orders() {
        let mut integrated = integrated();
        let request = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(5_000_000.0);
        let schedule = SliceSchedule::twap(Fixed::from_f64(1_000.0), 50, 10);
        let reports = integrated.execute_schedule(&request, &schedule);
        assert_eq!(reports.len(), 5);
        let total: Fixed = reports
            .iter()
            .fold(Fixed::ZERO, |acc, report| acc + report.filled_qty);
        assert_eq!(total, Fixed::from_f64(1_000.0));
    }

    #[test]
    fn test_gateway_ids_match_venue_ids() {
        let integrated = integrated();
        let ids = venue_ids(integrated.gateways());
        assert_eq!(ids.len(), integrated.router().venues().len());
        for (id, venue) in ids.iter().zip(integrated.router().venues()) {
            assert_eq!(*id, venue.id);
        }
    }
}
