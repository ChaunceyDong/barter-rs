use super::ClientOrderId;
use barter_integration::model::{
    instrument::{symbol::Symbol, Instrument},
    Exchange, Side,
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fmt::{Display, Formatter},
};

/// Type of [`Order`].
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub enum OrderKind {
    Market,
    Limit,
    PostOnly,
    ImmediateOrCancel,
}

impl Display for OrderKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                OrderKind::Market => "market",
                OrderKind::Limit => "limit",
                OrderKind::PostOnly => "post_only",
                OrderKind::ImmediateOrCancel => "immediate_or_cancel",
            }
        )
    }
}

/// Todo:
#[derive(Clone, Eq, PartialEq, PartialOrd, Debug, Deserialize, Serialize)]
pub struct Order<State> {
    pub exchange: Exchange,
    pub instrument: Instrument,
    pub cid: ClientOrderId,
    pub side: Side,
    pub state: State,
}

/// The initial state of an [`Order`]. Sent to the [`ExecutionClient`](crate::ExecutionClient) for
/// actioning.
#[derive(Copy, Clone, PartialEq, PartialOrd, Debug, Deserialize, Serialize)]
pub struct RequestOpen {
    pub kind: OrderKind,
    pub price: f64,
    pub quantity: f64,
}

impl Order<RequestOpen> {
    pub fn required_available_balance(&self) -> (&Symbol, f64) {
        match self.side {
            Side::Buy => (
                &self.instrument.quote,
                self.state.price * self.state.quantity,
            ),
            Side::Sell => (&self.instrument.base, self.state.quantity),
        }
    }
}

/// State of an [`Order`] after a [`RequestOpen`] has been sent to the
/// [`ExecutionClient`](crate::ExecutionClient), but a confirmation response has not been received.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct InFlight;

/// State of an [`Order`] after a request has been made for it to be [`Cancelled`].
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct RequestCancel {
    pub id: OrderId,
}

impl<Id> From<Id> for RequestCancel
where
    Id: Into<OrderId>,
{
    fn from(id: Id) -> Self {
        Self { id: id.into() }
    }
}

/// Todo:
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Open {
    pub id: OrderId,
    pub price: f64,
    pub quantity: f64,
    pub filled_quantity: f64,
}

impl Open {
    pub fn remaining_quantity(&self) -> f64 {
        self.quantity - self.filled_quantity
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Deserialize, Serialize)]
pub enum OrderFill {
    Full,
    Partial,
}

impl Ord for Order<Open> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other)
            .unwrap_or_else(|| panic!("{:?}.partial_cmp({:?}) impossible", self, other))
    }
}

impl PartialOrd for Order<Open> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self.side, other.side) {
            (Side::Buy, Side::Buy) => match self.state.price.partial_cmp(&other.state.price)? {
                Ordering::Equal => self
                    .state
                    .remaining_quantity()
                    .partial_cmp(&other.state.remaining_quantity()),
                non_equal => Some(non_equal),
            },
            (Side::Sell, Side::Sell) => match other.state.price.partial_cmp(&self.state.price)? {
                Ordering::Equal => other
                    .state
                    .remaining_quantity()
                    .partial_cmp(&self.state.remaining_quantity()),
                non_equal => Some(non_equal),
            },
            _ => None,
        }
    }
}

impl Eq for Order<Open> {}

/// State of an [`Order`] after being [`Cancelled`].
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct Cancelled {
    pub id: OrderId,
}

impl<Id> From<Id> for Cancelled
where
    Id: Into<OrderId>,
{
    fn from(id: Id) -> Self {
        Self { id: id.into() }
    }
}

/// [`Order`] identifier generated by an exchange. Cannot assume this is unique across each
/// [`Exchange`](barter_integration::model::Exchange),
/// [`Market`](barter_integration::model::Market), or
/// [`Instrument`](barter_integration::model::Instrument).
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct OrderId(pub String);

impl<S> From<S> for OrderId
where
    S: Display,
{
    fn from(id: S) -> Self {
        Self(id.to_string())
    }
}

impl From<&Order<RequestOpen>> for Order<InFlight> {
    fn from(request: &Order<RequestOpen>) -> Self {
        Self {
            exchange: request.exchange.clone(),
            instrument: request.instrument.clone(),
            cid: request.cid,
            side: request.side,
            state: InFlight,
        }
    }
}

impl From<(OrderId, Order<RequestOpen>)> for Order<Open> {
    fn from((id, request): (OrderId, Order<RequestOpen>)) -> Self {
        Self {
            exchange: request.exchange.clone(),
            instrument: request.instrument.clone(),
            cid: request.cid,
            side: request.side,
            state: Open {
                id,
                price: request.state.price,
                quantity: request.state.quantity,
                filled_quantity: 0.0,
            },
        }
    }
}

impl From<Order<Open>> for Order<Cancelled> {
    fn from(order: Order<Open>) -> Self {
        Self {
            exchange: order.exchange.clone(),
            instrument: order.instrument.clone(),
            cid: order.cid,
            side: order.side,
            state: Cancelled { id: order.state.id },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::order_open;
    use uuid::Uuid;

    #[test]
    fn test_open_order_remaining_quantity() {
        let order = order_open(ClientOrderId(Uuid::new_v4()), Side::Buy, 10.0, 10.0, 5.0);
        assert_eq!(order.state.remaining_quantity(), 5.0)
    }

    #[test]
    fn test_partial_ord_order_open() {
        struct TestCase {
            input_one: Order<Open>,
            input_two: Order<Open>,
            expected: Option<Ordering>,
        }

        let cid = ClientOrderId(Uuid::new_v4());

        let tests = vec![
            // -- Side::Buy Order<Open> --
            TestCase {
                // TC0: Input One has higher price and higher quantity -> Greater
                input_one: order_open(cid, Side::Buy, 1100.0, 2.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC1: Input One has higher price but same quantity -> Greater
                input_one: order_open(cid, Side::Buy, 1100.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC2: Input One has higher price but lower quantity -> Greater
                input_one: order_open(cid, Side::Buy, 1100.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1000.0, 2.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC3: Input One has same price and higher quantity -> Greater
                input_one: order_open(cid, Side::Buy, 1000.0, 2.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC4: Input One has same price and same quantity -> Equal
                input_one: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Equal),
            },
            TestCase {
                // TC5: Input One has same price but lower quantity -> Less
                input_one: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1000.0, 2.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // TC6: Input One has lower price but higher quantity -> Less
                input_one: order_open(cid, Side::Buy, 1000.0, 2.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1100.0, 1.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // TC7: Input One has lower price and same quantity -> Less
                input_one: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1100.0, 1.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // TC8: Input One has lower price but lower quantity -> Less
                input_one: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Buy, 1100.0, 2.0, 0.0),
                expected: Some(Ordering::Less),
            },
            // -- Side::Sell Order<Open> --
            TestCase {
                // TC9: Input One has higher price and higher quantity -> Lesser
                input_one: order_open(cid, Side::Sell, 1100.0, 2.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // TC10: Input One has higher price but same quantity -> Lesser
                input_one: order_open(cid, Side::Sell, 1100.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // T11: Input One has higher price but lower quantity -> Lesser
                input_one: order_open(cid, Side::Sell, 1100.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1000.0, 2.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // TC12: Input One has same price and higher quantity -> Lesser
                input_one: order_open(cid, Side::Sell, 1000.0, 2.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Less),
            },
            TestCase {
                // TC13: Input One has same price and same quantity -> Equal
                input_one: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                expected: Some(Ordering::Equal),
            },
            TestCase {
                // TC14: Input One has same price but lower quantity -> Greater
                input_one: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1000.0, 2.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC15: Input One has lower price but higher quantity -> Greater
                input_one: order_open(cid, Side::Sell, 1000.0, 2.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1100.0, 1.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC16: Input One has lower price and same quantity -> Greater
                input_one: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1100.0, 1.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            TestCase {
                // TC17: Input One has lower price but lower quantity -> Greater
                input_one: order_open(cid, Side::Sell, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1100.0, 2.0, 0.0),
                expected: Some(Ordering::Greater),
            },
            // -- Inputs Are Not Comparable Due To Different Sides
            TestCase {
                // TC18: Input One has lower price but lower quantity -> Greater
                input_one: order_open(cid, Side::Buy, 1000.0, 1.0, 0.0),
                input_two: order_open(cid, Side::Sell, 1100.0, 2.0, 0.0),
                expected: None,
            },
        ];

        for (index, test) in tests.into_iter().enumerate() {
            let actual = test.input_one.partial_cmp(&test.input_two);
            match (actual, test.expected) {
                (None, None) => {
                    // Test passed
                }
                (Some(actual), Some(expected)) => {
                    assert_eq!(actual, expected, "TC{} failed", index)
                }
                (actual, expected) => {
                    // Test failed
                    panic!("TC{index} failed because actual != expected. \nActual: {actual:?}\nExpected: {expected:?}\n");
                }
            }
        }
    }

    #[test]
    fn test_sort_vector_order_open() {
        struct TestCase {
            input: Vec<Order<Open>>,
            expected: Vec<Order<Open>>,
        }

        let cid = ClientOrderId(Uuid::new_v4());

        let tests = vec![
            TestCase {
                // TC0: Vector Empty
                input: vec![],
                expected: vec![],
            },
            // -- Vector: Side::Buy Order<Open> --
            TestCase {
                // TC1: Vector of Side::Buy Order<Open> already sorted
                input: vec![
                    order_open(cid, Side::Buy, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 300.0, 1.0, 0.0),
                ],
                expected: vec![
                    order_open(cid, Side::Buy, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 300.0, 1.0, 0.0),
                ],
            },
            TestCase {
                // TC2: Vector of Side::Buy Order<Open> reverse sorted
                input: vec![
                    order_open(cid, Side::Buy, 300.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 100.0, 1.0, 0.0),
                ],
                expected: vec![
                    order_open(cid, Side::Buy, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 300.0, 1.0, 0.0),
                ],
            },
            TestCase {
                // TC3: Vector of Side::Buy Order<Open> unsorted sorted
                input: vec![
                    order_open(cid, Side::Buy, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 300.0, 1.0, 0.0),
                ],
                expected: vec![
                    order_open(cid, Side::Buy, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Buy, 300.0, 1.0, 0.0),
                ],
            },
            // -- Vector: Side::Sell Order<Open> --
            TestCase {
                // TC1: Vector of Side::Sell Order<Open> already sorted
                input: vec![
                    order_open(cid, Side::Sell, 300.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 100.0, 1.0, 0.0),
                ],
                expected: vec![
                    order_open(cid, Side::Sell, 300.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 100.0, 1.0, 0.0),
                ],
            },
            TestCase {
                // TC2: Vector of Side::Sell Order<Open> reverse sorted
                input: vec![
                    order_open(cid, Side::Sell, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 300.0, 1.0, 0.0),
                ],
                expected: vec![
                    order_open(cid, Side::Sell, 300.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 100.0, 1.0, 0.0),
                ],
            },
            TestCase {
                // TC3: Vector of Side::Sell Order<Open> unsorted sorted
                input: vec![
                    order_open(cid, Side::Sell, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 100.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 300.0, 1.0, 0.0),
                ],
                expected: vec![
                    order_open(cid, Side::Sell, 300.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 200.0, 1.0, 0.0),
                    order_open(cid, Side::Sell, 100.0, 1.0, 0.0),
                ],
            },
        ];

        for (index, mut test) in tests.into_iter().enumerate() {
            test.input.sort();
            assert_eq!(test.input, test.expected, "TC{} failed", index);
        }
    }
}
