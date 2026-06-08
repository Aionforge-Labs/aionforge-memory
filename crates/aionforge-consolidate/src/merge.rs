//! Convergence ordering primitives for consolidation merge (write-and-consolidation §2,
//! 06 §2).
//!
//! These pure functions decide, from values alone, which side of a functional supersession
//! or a mutually-exclusive contradiction is the current one — never from the substrate's
//! arrival clock, the content-hash `Fact.id`, or the originating agent (all of which are
//! fixed by whichever episode wins the dedup race, so they are arrival-fragile). Because the
//! decision is a pure function of the assertion values, the same set of assertions resolves
//! the same way under any consolidation order, which is what makes the merge converge.

use std::cmp::Ordering;

use aionforge_domain::time::Timestamp;
use aionforge_domain::value::ObjectValue;

/// Whether the new assertion wins the single functional slot over the incumbent under the K1
/// order: the later event time (`valid_from`) wins, and an exact `valid_from` tie is settled
/// by the smaller [`object_order_key`]. Pure and total over the two (always distinct)
/// functional objects, so exactly one side wins regardless of which was committed first.
pub(crate) fn new_wins_functional_slot(
    new_object: &ObjectValue,
    new_valid_from: &Timestamp,
    incumbent_object: &ObjectValue,
    incumbent_valid_from: &Timestamp,
) -> bool {
    if *new_valid_from > *incumbent_valid_from {
        true
    } else if *new_valid_from < *incumbent_valid_from {
        false
    } else {
        object_order_key(new_object) < object_order_key(incumbent_object)
    }
}

/// Whether the new fact is the contradiction victim — the `CONTRADICTS` source the
/// `current_support_facts` provider excludes from recall. The victim is the LOWER-TRUST side;
/// an exact trust tie is settled by the smaller [`object_order_key`]. Trust is compared with
/// [`f64::total_cmp`], not `<`, so a NaN or otherwise degenerate trust cannot make the victim
/// depend on evaluation order. A pure function of the unordered pair {(trust, object)} — it
/// never reads which side is the incumbent — so the victim is identical under any
/// consolidation order, which is what makes the contradiction converge (06 §2).
pub(crate) fn new_is_contradiction_victim(
    new_object: &ObjectValue,
    new_trust: f64,
    incumbent_object: &ObjectValue,
    incumbent_trust: f64,
) -> bool {
    match new_trust.total_cmp(&incumbent_trust) {
        Ordering::Less => true,     // the new fact is lower trust — it is the victim
        Ordering::Greater => false, // the incumbent is lower trust — it is the victim
        // Equal trust: the smaller object order is the victim (a deterministic, non-semantic
        // tiebreak — all convergence needs between two distinct, equally-trusted objects).
        Ordering::Equal => object_order_key(new_object) < object_order_key(incumbent_object),
    }
}

/// A deterministic, injective ordering key over an object value, for a simultaneous-tie
/// settle. It deliberately avoids the content-hash `Fact.id` (which folds in the source
/// episode and so is fixed by whichever episode wins the `(subject, predicate, object)` dedup
/// race — arrival-fragile) and the originating agent (arrival-fragile for the same reason):
/// the key is a pure function of the value alone, so two distinct objects always get distinct
/// keys and the winner is identical under any consolidation order. The order is DETERMINISTIC,
/// not semantic — numbers and datetimes sort by their canonical text, not their magnitude —
/// which is all a tiebreak between two already-distinct objects needs. Exhaustive and
/// infallible by construction: it never collapses two values to one key, so the total order
/// cannot silently degenerate (the kind prefix keeps the variants disjoint; entity ids and
/// the JSON `Debug` fallback stay injective).
pub(crate) fn object_order_key(object: &ObjectValue) -> String {
    match object {
        ObjectValue::Entity(id) => format!("entity:{id}"),
        ObjectValue::Text(text) => format!("string:{text}"),
        ObjectValue::Number(number) => format!("number:{:016x}", number.to_bits()),
        ObjectValue::Bool(value) => format!("bool:{value}"),
        ObjectValue::DateTime(timestamp) => format!("datetime:{timestamp}"),
        ObjectValue::Json(value) => format!(
            "json:{}",
            serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aionforge_domain::ids::Id;

    fn ts(text: &str) -> Timestamp {
        text.parse().expect("valid zoned datetime literal")
    }

    fn t1() -> Timestamp {
        ts("2026-06-06T09:00:00Z[UTC]")
    }

    fn t2() -> Timestamp {
        ts("2026-06-06T11:00:00Z[UTC]")
    }

    fn text(value: &str) -> ObjectValue {
        ObjectValue::Text(value.to_string())
    }

    #[test]
    fn object_order_key_is_injective_across_variants() {
        // One value per variant plus a couple within-variant pairs — all distinct keys, so no
        // two distinct objects can ever collapse to the same tiebreak.
        let values = [
            ObjectValue::Entity(Id::from_content_hash(b"a")),
            ObjectValue::Entity(Id::from_content_hash(b"b")),
            text("up"),
            text("down"),
            ObjectValue::Number(1.0),
            ObjectValue::Number(2.0),
            ObjectValue::Number(-0.0),
            ObjectValue::Number(0.0),
            ObjectValue::Bool(true),
            ObjectValue::Bool(false),
            ObjectValue::DateTime(t1()),
            ObjectValue::DateTime(t2()),
            ObjectValue::Json(serde_json::json!({"k": 1})),
            ObjectValue::Json(serde_json::json!({"k": 2})),
        ];
        let mut keys: Vec<String> = values.iter().map(object_order_key).collect();
        let total = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys.len(),
            total,
            "every distinct object gets a distinct key"
        );
    }

    #[test]
    fn object_order_key_is_stable_for_the_same_value() {
        assert_eq!(object_order_key(&text("x")), object_order_key(&text("x")));
        // -0.0 and 0.0 are distinct bit patterns, so they do not collapse.
        assert_ne!(
            object_order_key(&ObjectValue::Number(-0.0)),
            object_order_key(&ObjectValue::Number(0.0))
        );
    }

    #[test]
    fn the_later_event_time_wins_the_functional_slot() {
        // SF@t2 vs NYC@t1 — the later event time wins regardless of which is "new".
        assert!(new_wins_functional_slot(
            &text("SF"),
            &t2(),
            &text("NYC"),
            &t1()
        ));
        assert!(!new_wins_functional_slot(
            &text("NYC"),
            &t1(),
            &text("SF"),
            &t2()
        ));
    }

    #[test]
    fn an_equal_event_time_is_settled_by_the_smaller_object_and_is_symmetric() {
        // Equal valid_from: 'NYC' < 'SF', so NYC wins whichever side it is on — exactly one
        // side wins, so the functional slot converges.
        assert!(new_wins_functional_slot(
            &text("NYC"),
            &t2(),
            &text("SF"),
            &t2()
        ));
        assert!(!new_wins_functional_slot(
            &text("SF"),
            &t2(),
            &text("NYC"),
            &t2()
        ));
    }

    #[test]
    fn the_lower_trust_side_is_the_contradiction_victim() {
        // The lower-trust side is the victim regardless of which side is the incumbent.
        assert!(new_is_contradiction_victim(
            &text("a"),
            0.4,
            &text("b"),
            0.9
        ));
        assert!(!new_is_contradiction_victim(
            &text("a"),
            0.9,
            &text("b"),
            0.4
        ));
    }

    #[test]
    fn an_equal_trust_contradiction_victim_is_the_smaller_object_and_is_symmetric() {
        // Equal trust: 'down' < 'up', so 'down' is the victim whichever side it is on. The
        // victim is identical in both orders, so the contradiction converges.
        let down_is_new = new_is_contradiction_victim(&text("down"), 0.9, &text("up"), 0.9);
        let down_is_incumbent = !new_is_contradiction_victim(&text("up"), 0.9, &text("down"), 0.9);
        assert!(down_is_new, "'down' is the victim when it is the new fact");
        assert!(
            down_is_incumbent,
            "'down' is still the victim when it is the incumbent"
        );
    }
}
