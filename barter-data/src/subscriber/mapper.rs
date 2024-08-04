use crate::{
    exchange::{subscription::ExchangeSub, Connector},
    instrument::InstrumentData,
    subscription::{Map, Subscription, SubscriptionKind, SubscriptionMeta},
    Identifier,
};
use barter_integration::subscription::SubscriptionId;
use fnv::FnvHashMap;
use serde::{Deserialize, Serialize};

/// Defines how to map a collection of Barter [`Subscription`]s into execution specific
/// [`SubscriptionMeta`], containing subscription payloads that are sent to the execution.
pub trait SubscriptionMapper {
    fn map<Exchange, Instrument, Kind>(
        subscriptions: &[Subscription<Exchange, Instrument, Kind>],
    ) -> SubscriptionMeta<Instrument::Key>
    where
        Exchange: Connector,
        Instrument: InstrumentData,
        Kind: SubscriptionKind,
        Subscription<Exchange, Instrument, Kind>:
            Identifier<Exchange::Channel> + Identifier<Exchange::Market>;
}

/// Standard [`SubscriptionMapper`] for
/// [`WebSocket`](barter_integration::protocol::websocket::WebSocket)s suitable for most exchanges.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
pub struct WebSocketSubMapper;

impl SubscriptionMapper for WebSocketSubMapper {
    fn map<Exchange, Instrument, Kind>(
        subscriptions: &[Subscription<Exchange, Instrument, Kind>],
    ) -> SubscriptionMeta<Instrument::Key>
    where
        Exchange: Connector,
        Instrument: InstrumentData,
        Kind: SubscriptionKind,
        Subscription<Exchange, Instrument, Kind>:
            Identifier<Exchange::Channel> + Identifier<Exchange::Market>,
        ExchangeSub<Exchange::Channel, Exchange::Market>: Identifier<SubscriptionId>,
    {
        // Allocate SubscriptionIds HashMap to track identifiers for each actioned Subscription
        let mut instrument_map = Map(FnvHashMap::with_capacity_and_hasher(
            subscriptions.len(),
            Default::default(),
        ));

        // Map Barter Subscriptions to execution specific subscriptions
        let exchange_subs = subscriptions
            .iter()
            .map(|subscription| {
                // Translate Barter Subscription to execution specific subscription
                let exchange_sub = ExchangeSub::new(subscription);

                // Determine the SubscriptionId associated with this execution specific subscription
                let subscription_id = exchange_sub.id();

                // Use ExchangeSub SubscriptionId as the manager to this Barter Subscription
                instrument_map
                    .0
                    .insert(subscription_id, subscription.instrument.key().clone());

                exchange_sub
            })
            .collect::<Vec<ExchangeSub<Exchange::Channel, Exchange::Market>>>();

        // Construct WebSocket message subscriptions requests
        let ws_subscriptions = Exchange::requests(exchange_subs);

        SubscriptionMeta {
            instrument_map,
            ws_subscriptions,
        }
    }
}
