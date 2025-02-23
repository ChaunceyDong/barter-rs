use crate::engine::Processor;
use barter_data::event::MarketEvent;
use barter_execution::{
    AccountEvent,
    order::request::{OrderRequestCancel, OrderRequestOpen},
};
use barter_instrument::{exchange::ExchangeIndex, instrument::InstrumentIndex};
use barter_integration::Unrecoverable;
use derive_more::{Constructor, Display, From};
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, hash::Hash, marker::PhantomData};

/// RiskManager interface that reviews and optionally filters cancel and open order requests
/// generated by an [`AlgoStrategy`](super::strategy::algo::AlgoStrategy).
///
/// For example, a RiskManager implementation may wish to:
/// - Filter out orders that would result in too much exposure.
/// - Filter out orders that have a too high quantity.
/// - Adjust order quantities.
/// - Filter out orders that would cross the OrderBook.
/// - etc.
///
/// # Type Parameters
/// * `ExchangeKey` - Type used to identify an exchange (defaults to [`ExchangeIndex`]).
/// * `InstrumentKey` - Type used to identify an instrument (defaults to [`InstrumentIndex`]).
pub trait RiskManager<ExchangeKey = ExchangeIndex, InstrumentKey = InstrumentIndex> {
    type State;

    fn check(
        &self,
        state: &Self::State,
        cancels: impl IntoIterator<Item = OrderRequestCancel<ExchangeKey, InstrumentKey>>,
        opens: impl IntoIterator<Item = OrderRequestOpen<ExchangeKey, InstrumentKey>>,
    ) -> (
        impl IntoIterator<Item = RiskApproved<OrderRequestCancel<ExchangeKey, InstrumentKey>>>,
        impl IntoIterator<Item = RiskApproved<OrderRequestOpen<ExchangeKey, InstrumentKey>>>,
        impl IntoIterator<Item = RiskRefused<OrderRequestCancel<ExchangeKey, InstrumentKey>>>,
        impl IntoIterator<Item = RiskRefused<OrderRequestOpen<ExchangeKey, InstrumentKey>>>,
    );
}

/// New type that wraps [`Order`] requests that have passed [`RiskManager`] checks.
#[derive(
    Debug,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Deserialize,
    Serialize,
    Display,
    From,
    Constructor,
)]
pub struct RiskApproved<T>(pub T);

impl<T> RiskApproved<T> {
    pub fn into_item(self) -> T {
        self.0
    }
}

/// Type that wraps [`Order`] requests that have failed [`RiskManager`] checks, including the
/// failure reason.
#[derive(
    Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Constructor,
)]
pub struct RiskRefused<T, Reason = String> {
    pub item: T,
    pub reason: Reason,
}

impl<T, Reason> RiskRefused<T, Reason> {
    pub fn into_item(self) -> T {
        self.item
    }
}

impl<T, Reason> Unrecoverable for RiskRefused<T, Reason>
where
    Reason: Unrecoverable,
{
    fn is_unrecoverable(&self) -> bool {
        self.reason.is_unrecoverable()
    }
}

/// Naive implementation of the [`RiskManager`] interface, approving all orders *without any
/// risk checks*.
///
/// *THIS IS FOR DEMONSTRATION PURPOSES ONLY, NEVER USE FOR REAL TRADING OR IN PRODUCTION*.
#[derive(Debug, Clone)]
pub struct DefaultRiskManager<State> {
    phantom: PhantomData<State>,
}

impl<State> Default for DefaultRiskManager<State> {
    fn default() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<State, ExchangeKey, InstrumentKey> RiskManager<ExchangeKey, InstrumentKey>
    for DefaultRiskManager<State>
{
    type State = State;

    fn check(
        &self,
        _: &Self::State,
        cancels: impl IntoIterator<Item = OrderRequestCancel<ExchangeKey, InstrumentKey>>,
        opens: impl IntoIterator<Item = OrderRequestOpen<ExchangeKey, InstrumentKey>>,
    ) -> (
        impl IntoIterator<Item = RiskApproved<OrderRequestCancel<ExchangeKey, InstrumentKey>>>,
        impl IntoIterator<Item = RiskApproved<OrderRequestOpen<ExchangeKey, InstrumentKey>>>,
        impl IntoIterator<Item = RiskRefused<OrderRequestCancel<ExchangeKey, InstrumentKey>>>,
        impl IntoIterator<Item = RiskRefused<OrderRequestOpen<ExchangeKey, InstrumentKey>>>,
    ) {
        (
            cancels.into_iter().map(RiskApproved::new),
            opens.into_iter().map(RiskApproved::new),
            std::iter::empty(),
            std::iter::empty(),
        )
    }
}

/// Empty risk manager state.
///
/// *THIS IS FOR DEMONSTRATION PURPOSES ONLY, NEVER USE FOR REAL TRADING OR IN PRODUCTION*.
#[derive(
    Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default, Deserialize, Serialize,
)]
pub struct DefaultRiskManagerState;

impl<ExchangeKey, AssetKey, InstrumentKey>
    Processor<&AccountEvent<ExchangeKey, AssetKey, InstrumentKey>> for DefaultRiskManagerState
{
    type Audit = ();
    fn process(&mut self, _: &AccountEvent<ExchangeKey, AssetKey, InstrumentKey>) -> Self::Audit {}
}

impl<InstrumentKey, Kind> Processor<&MarketEvent<InstrumentKey, Kind>> for DefaultRiskManagerState {
    type Audit = ();
    fn process(&mut self, _: &MarketEvent<InstrumentKey, Kind>) -> Self::Audit {}
}
