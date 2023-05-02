use std::collections::{HashMap, HashSet};

use crate::order::{Order, Side};

#[derive(Debug, Clone)]
pub struct ExecutionList {
    pub lookup: HashMap<u32, HashSet<usize>>,
    pub matches: HashMap<usize, (Order, Order)>,
}

impl ExecutionList {
    pub fn new() -> ExecutionList {
        ExecutionList {
            lookup: HashMap::new(),
            matches: HashMap::new(),
        }
    }

    pub fn insert(&mut self, execution_id: usize, execution: (Order, Order)) {
        self.lookup
            .entry(execution.0.id)
            .or_insert_with(HashSet::new)
            .insert(execution_id);
        self.lookup
            .entry(execution.1.id)
            .or_insert_with(HashSet::new)
            .insert(execution_id);
        self.matches.insert(execution_id, execution);
    }

    pub fn get_matches_for_id(&self, order_id: u32) -> HashSet<(Order, Order)> {
        if let Some(key) = self.lookup.get(&order_id) {
            key.iter()
                .filter_map(|&value| self.matches.get(&value).cloned())
                .collect()
        } else {
            HashSet::new()
        }
    }
}

#[test]
fn test_new_execution_list() {
    let executions = ExecutionList::new();
    assert!(executions.lookup.is_empty());
    assert!(executions.matches.is_empty());
}

#[test]
fn test_insert_execution() {
    let mut executions = ExecutionList::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let execution = (order1.clone(), order2.clone());
    executions.insert(1, execution.clone());
    assert_eq!(executions.lookup.len(), 2);
    assert_eq!(executions.matches.len(), 1);
    assert_eq!(executions.lookup.get(&order1.id).unwrap().len(), 1);
    assert_eq!(executions.lookup.get(&order2.id).unwrap().len(), 1);
    assert_eq!(executions.matches.get(&1).unwrap().0.id, execution.0.id);
    assert_eq!(executions.matches.get(&1).unwrap().1.id, execution.1.id);
}

#[test]
fn test_lookup_with_order_id() {
    let mut executions = ExecutionList::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let execution = (order1.clone(), order2.clone());
    executions.insert(1, execution.clone());
    assert_eq!(
        executions.get_matches_for_id(order1.id).get(&execution),
        Some(&execution)
    );
    assert_eq!(
        executions.get_matches_for_id(order2.id).get(&execution),
        Some(&execution)
    );
}
