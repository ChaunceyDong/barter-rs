use crate::{Timed, engine::Processor};
use barter_data::{
    event::{DataKind, MarketEvent},
    subscription::book::OrderBookL1,
};
use barter_instrument::instrument::InstrumentIndex;
use derive_more::Constructor;
use rust_decimal::{Decimal, prelude::FromPrimitive};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Defines a state object for tracking and managing custom instrument level data.
///
/// Implementations must handle market event processing and provide logic for accessing the latest
/// instrument market price.
///
/// This trait enables users to define their own instrument level data, and specify the type of
/// [`MarketEvent`] that is required to update it. The custom instrument data could include
/// market data, strategy-specific data, risk-specific data, or any other instrument level data.
///
/// For an example, see the [`DefaultInstrumentMarketData`] implementation.
pub trait InstrumentDataState<InstrumentKey = InstrumentIndex>
where
    Self: Debug
        + Clone
        + Send
        + for<'a> Processor<&'a MarketEvent<InstrumentKey, Self::MarketEventKind>>,
{
    /// [`MarketEvent<_, EventKind>`](MarketEvent) expected by this instrument data state.
    type MarketEventKind: Debug + Clone + Send;

    /// Latest price for an instrument, if available.
    ///
    /// Return the latest market price for an instrument, if available.
    ///
    /// An instrument price could be derived in many ways, but some common examples include:
    /// - Most recent `PublicTrade` price.
    /// - Volume-weighted mid-price from an `OrderBookL1`.
    /// - Volume-weighted mid-price from an `OrderBookL2`.
    fn price(&self) -> Option<Decimal>;
}

/// Basic [`InstrumentDataState`] implementation that tracks the [`OrderBookL1`] and last traded
/// price for an instrument.
///
/// This is a simple example of instrument level data. Trading strategies typically maintain more
/// comprehensive data, such as candles, technical indicators, market depth (L2 book), volatility metrics,
/// or strategy-specific state data.
#[derive(
    Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default, Deserialize, Serialize, Constructor,
)]
pub struct DefaultInstrumentMarketData {
    pub l1: OrderBookL1,
    pub last_traded_price: Option<Timed<Decimal>>,
}

impl InstrumentDataState for DefaultInstrumentMarketData {
    type MarketEventKind = DataKind;

    fn price(&self) -> Option<Decimal> {
        self.l1
            .volume_weighed_mid_price()
            .or(self.last_traded_price.as_ref().map(|timed| timed.value))
    }
}

impl<InstrumentKey> Processor<&MarketEvent<InstrumentKey, DataKind>>
    for DefaultInstrumentMarketData
{
    type Audit = ();

    fn process(&mut self, event: &MarketEvent<InstrumentKey, DataKind>) -> Self::Audit {
        match &event.kind {
            DataKind::Trade(trade) => {
                if self
                    .last_traded_price
                    .as_ref()
                    .is_none_or(|price| price.time < event.time_exchange)
                {
                    if let Some(price) = Decimal::from_f64(trade.price) {
                        self.last_traded_price
                            .replace(Timed::new(price, event.time_exchange));
                    }
                }
            }
            DataKind::OrderBookL1(l1) => {
                if self.l1.last_update_time < event.time_exchange {
                    self.l1 = l1.clone()
                }
            }
            _ => {}
        }
    }
}
