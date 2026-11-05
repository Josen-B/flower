//! Pure ordering operations shared by activation memory and the local store.
use super::{PromotionEvent, RankingCandidate, RankingOrder};
use std::collections::{HashMap, HashSet};

/// Reconcile an active candidate set with a saved order, without inventing choices.
pub fn ranking_indices(candidates: &[RankingCandidate], saved: &RankingOrder) -> Vec<usize> {
    let lookup: HashMap<_, _> = candidates.iter().enumerate().map(|(i, c)| (c, i)).collect();
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(candidates.len());
    for candidate in &saved.candidates {
        if let Some(&index) = lookup.get(candidate) {
            if seen.insert(index) {
                result.push(index);
            }
        }
    }
    result.extend((0..candidates.len()).filter(|i| !seen.contains(i)));
    result
}

/// Call only after validation and a successful safe commit. Returns whether a
/// choice moved; the newest domain is retained even for a first-position choice.
pub fn promote_order(orders: &mut Vec<RankingOrder>, event: &PromotionEvent) -> bool {
    let mut order = if let Some(index) = orders.iter().position(|o| o.input == event.order.input) {
        let previous = orders.remove(index);
        let indices = ranking_indices(&event.order.candidates, &previous);
        RankingOrder {
            input: event.order.input.clone(),
            candidates: indices
                .into_iter()
                .map(|i| event.order.candidates[i].clone())
                .collect(),
        }
    } else {
        event.order.clone()
    };
    let mut moved = false;
    if let Some(index) = order.candidates.iter().position(|c| c == &event.selected) {
        if index > 0 {
            order.candidates.swap(index - 1, index);
            moved = true;
        }
    }
    if orders.len() >= 256 {
        orders.remove(0);
    }
    orders.push(order);
    moved
}
