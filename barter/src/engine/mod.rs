use crate::{
    engine::{
        action::{
            cancel_orders::CancelOrders,
            close_positions::ClosePositions,
            generate_algo_orders::{GenerateAlgoOrders, GenerateAlgoOrdersOutput},
            send_requests::SendRequests,
            ActionOutput,
        },
        audit::{context::EngineContext, Audit, AuditTick, Auditor, DefaultAudit},
        clock::EngineClock,
        command::Command,
        execution_tx::ExecutionTxMap,
        state::{
            instrument::market_data::MarketDataState,
            order::in_flight_recorder::InFlightRequestRecorder, trading::TradingState, EngineState,
        },
    },
    execution::AccountStreamEvent,
    risk::RiskManager,
    statistic::summary::TradingSummaryGenerator,
    strategy::{
        algo::AlgoStrategy, close_positions::ClosePositionsStrategy,
        on_disconnect::OnDisconnectStrategy, on_trading_disabled::OnTradingDisabled,
    },
    EngineEvent, Sequence,
};
use audit::shutdown::ShutdownAudit;
use barter_data::{event::MarketEvent, streams::consumer::MarketStreamEvent};
use barter_execution::AccountEvent;
use barter_instrument::{exchange::ExchangeIndex, instrument::InstrumentIndex};
use barter_integration::channel::{ChannelTxDroppable, Tx};
use chrono::{DateTime, Utc};
use derive_more::From;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tracing::info;

/// Defines how the [`Engine`] actions a [`Command`], and the associated outputs.
pub mod action;

/// Todo:
pub mod audit;

/// Defines the [`EngineClock`] interface used to determine the current `Engine` time.
///
/// This flexibility enables back-testing runs to use approximately correct historical timestamps.
pub mod clock;

/// Defines an [`Engine`] [`Command`] - used to give trading directives to the `Engine` from an
/// external process (eg/ ClosePositions).
pub mod command;

/// Defines all possible errors that can occur in the [`Engine`].
pub mod error;

/// Defines the [`ExecutionTxMap`] interface that models a collection of transmitters used to route
/// can [`ExecutionRequest`] to the appropriate `ExecutionManagers`.
pub mod execution_tx;

/// Todo:
pub mod state;

/// Defines how a component processing an input Event and generates an appropriate Output.
pub trait Processor<Event> {
    type Output;
    fn process(&mut self, event: Event) -> Self::Output;
}

/// Primary `Engine` entry point that processes input `Events` and forwards audits to the provided
/// `AuditTx`.
///
/// Runs until shutdown, returning a [`ShutdownAudit`] detailing the reason for the shutdown
/// (eg/ `Events` `FeedEnded`, `Command::Shutdown`, etc.).
///
/// # Arguments
/// * `Events` - Iterator of events for the `Engine` to process.
/// * `Engine` - Event processor that produces audit events as output.
/// * `AuditTx` - Channel for sending produced audit events.
pub fn run<Events, Engine, AuditTx>(
    feed: &mut Events,
    engine: &mut Engine,
    audit_tx: &mut ChannelTxDroppable<AuditTx>,
) -> ShutdownAudit<Events::Item>
where
    Events: Iterator,
    Events::Item: Debug + Clone,
    Engine: Processor<Events::Item> + Auditor<Engine::Output, Context = EngineContext>,
    Engine::Output: From<Engine::Snapshot> + From<ShutdownAudit<Events::Item>>,
    AuditTx: Tx<Item = AuditTick<Engine::Output, EngineContext>>,
    Option<ShutdownAudit<Events::Item>>: for<'a> From<&'a Engine::Output>,
{
    info!("Engine running");

    // Send initial Engine state snapshot
    audit_tx.send(engine.audit(engine.snapshot()));

    // Run Engine process loop until shutdown
    let shutdown_audit = loop {
        let Some(event) = feed.next() else {
            audit_tx.send(engine.audit(ShutdownAudit::FeedEnded));
            break ShutdownAudit::FeedEnded;
        };

        // Process Event & check if Output indicates shutdown is required
        let audit_kind = engine.process(event);
        let shutdown = Option::<ShutdownAudit<Events::Item>>::from(&audit_kind);

        // Send AuditTick to AuditManager
        audit_tx.send(engine.audit(audit_kind));

        if let Some(shutdown) = shutdown {
            break shutdown;
        }
    };

    // Send Shutdown audit
    audit_tx.send(engine.audit(shutdown_audit.clone()));

    info!(?shutdown_audit, "Engine shutting down");
    shutdown_audit
}

/// Algorithmic trading `Engine`.
///
/// The `Engine`:
/// * Processes input [`EngineEvent`] (or custom events if implemented).
/// * Maintains the internal [`EngineState`] (market data state, open orders, positions, etc.).
/// * Generates algo orders (if `TradingState::Enabled`).
///
/// # Type Parameters
/// * `Clock` - [`EngineClock`] implementation.
/// * `State` - Engine `State` implementation (eg/ [`EngineState`]).
/// * `ExecutionTxs` - [`ExecutionTxMap`] implementation for sending execution requests.
/// * `Strategy` - Trading Strategy implementation (see [`super::strategy`]).
/// * `Risk` - [`RiskManager`] implementation.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Engine<Clock, State, ExecutionTxs, Strategy, Risk> {
    pub clock: Clock,
    pub meta: EngineMeta,
    pub state: State,
    pub execution_txs: ExecutionTxs,
    pub strategy: Strategy,
    pub risk: Risk,
}

/// Running [`Engine`] metadata.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub struct EngineMeta {
    /// [`EngineClock`] start timestamp of the current [`Engine`] `run`.
    pub time_start: DateTime<Utc>,
    /// Monotonically increasing [`Sequence`] associated with the number of events processed.
    pub sequence: Sequence,
}

impl<Clock, MarketState, StrategyState, RiskState, ExecutionTxs, Strategy, Risk>
    Processor<EngineEvent<MarketState::EventKind>>
    for Engine<
        Clock,
        EngineState<MarketState, StrategyState, RiskState>,
        ExecutionTxs,
        Strategy,
        Risk,
    >
where
    Clock: EngineClock + for<'a> Processor<&'a EngineEvent<MarketState::EventKind>>,
    MarketState: MarketDataState,
    StrategyState: for<'a> Processor<&'a AccountEvent>
        + for<'a> Processor<&'a MarketEvent<InstrumentIndex, MarketState::EventKind>>,
    RiskState: for<'a> Processor<&'a AccountEvent>
        + for<'a> Processor<&'a MarketEvent<InstrumentIndex, MarketState::EventKind>>,
    ExecutionTxs: ExecutionTxMap<ExchangeIndex, InstrumentIndex>,
    Strategy: OnTradingDisabled<
            Clock,
            EngineState<MarketState, StrategyState, RiskState>,
            ExecutionTxs,
            Risk,
        > + OnDisconnectStrategy<
            Clock,
            EngineState<MarketState, StrategyState, RiskState>,
            ExecutionTxs,
            Risk,
        > + AlgoStrategy<State = EngineState<MarketState, StrategyState, RiskState>>
        + ClosePositionsStrategy<State = EngineState<MarketState, StrategyState, RiskState>>,
    Risk: RiskManager<State = EngineState<MarketState, StrategyState, RiskState>>,
{
    type Output = DefaultAudit<
        MarketState,
        StrategyState,
        RiskState,
        Strategy::OnTradingDisabled,
        Strategy::OnDisconnect,
    >;

    fn process(&mut self, event: EngineEvent<MarketState::EventKind>) -> Self::Output {
        self.clock.process(&event);

        match &event {
            EngineEvent::Shutdown => return Audit::shutdown_commanded(event),
            EngineEvent::Command(command) => {
                let output = self.action(command);

                return if let Some(unrecoverable) = output.unrecoverable_errors() {
                    Audit::shutdown_on_err_with_output(event, unrecoverable, output)
                } else {
                    Audit::process_with_output(event, output)
                };
            }
            EngineEvent::TradingStateUpdate(trading_state) => {
                if let Some(trading_disabled) =
                    self.update_from_trading_state_update(*trading_state)
                {
                    return Audit::process_with_output(
                        event,
                        EngineOutput::OnTradingDisabled(trading_disabled),
                    );
                }
            }
            EngineEvent::Account(account) => {
                if let Some(disconnected) = self.update_from_account_stream(account) {
                    return Audit::process_with_output(
                        event,
                        EngineOutput::OnDisconnect(disconnected),
                    );
                }
            }
            EngineEvent::Market(market) => {
                if let Some(disconnected) = self.update_from_market_stream(market) {
                    return Audit::process_with_output(
                        event,
                        EngineOutput::OnDisconnect(disconnected),
                    );
                }
            }
        };

        if let TradingState::Enabled = self.state.trading {
            let output = self.generate_algo_orders();
            if let Some(unrecoverable) = output.unrecoverable_errors() {
                Audit::shutdown_on_err_with_output(event, unrecoverable, output)
            } else {
                Audit::process_with_output(event, output)
            }
        } else {
            Audit::process(event)
        }
    }
}

impl<Clock, MarketState, StrategyState, RiskState, ExecutionTxs, Strategy, Risk>
    Engine<Clock, EngineState<MarketState, StrategyState, RiskState>, ExecutionTxs, Strategy, Risk>
{
    /// Action an `Engine` [`Command`], producing an [`ActionOutput`] of work done.
    pub fn action(&mut self, command: &Command) -> ActionOutput
    where
        ExecutionTxs: ExecutionTxMap,
        Strategy:
            ClosePositionsStrategy<State = EngineState<MarketState, StrategyState, RiskState>>,
        Risk: RiskManager,
    {
        match &command {
            Command::SendCancelRequests(requests) => {
                info!(
                    ?requests,
                    "Engine actioning user Command::SendCancelRequests"
                );
                let output = self.send_requests(requests.clone());
                self.state.record_in_flight_cancels(&output.sent);
                ActionOutput::CancelOrders(output)
            }
            Command::SendOpenRequests(requests) => {
                info!(?requests, "Engine actioning user Command::SendOpenRequests");
                let output = self.send_requests(requests.clone());
                self.state.record_in_flight_opens(&output.sent);
                ActionOutput::OpenOrders(output)
            }
            Command::ClosePositions(filter) => {
                info!(?filter, "Engine actioning user Command::ClosePositions");
                ActionOutput::ClosePositions(self.close_positions(filter))
            }
            Command::CancelOrders(filter) => {
                info!(?filter, "Engine actioning user Command::CancelOrders");
                ActionOutput::CancelOrders(self.cancel_orders(filter))
            }
        }
    }

    /// Update the `Engine` [`TradingState`].
    ///
    /// If the `TradingState` transitions to `TradingState::Disabled`, the `Engine` will call
    /// the configured [`OnTradingDisabled`] strategy logic.
    pub fn update_from_trading_state_update(
        &mut self,
        update: TradingState,
    ) -> Option<Strategy::OnTradingDisabled>
    where
        Strategy: OnTradingDisabled<
            Clock,
            EngineState<MarketState, StrategyState, RiskState>,
            ExecutionTxs,
            Risk,
        >,
    {
        self.state
            .trading
            .update(update)
            .transitioned_to_disabled()
            .then(|| Strategy::on_trading_disabled(self))
    }

    /// Update the [`Engine`] from an [`AccountStreamEvent`].
    ///
    /// If the input `AccountStreamEvent` indicates the exchange execution link has disconnected,
    /// the `Engine` will call the configured [`OnDisconnectStrategy`] strategy logic.
    pub fn update_from_account_stream(
        &mut self,
        event: &AccountStreamEvent,
    ) -> Option<Strategy::OnDisconnect>
    where
        StrategyState: for<'a> Processor<&'a AccountEvent>,
        RiskState: for<'a> Processor<&'a AccountEvent>,
        Strategy: OnDisconnectStrategy<
            Clock,
            EngineState<MarketState, StrategyState, RiskState>,
            ExecutionTxs,
            Risk,
        >,
    {
        match event {
            AccountStreamEvent::Reconnecting(exchange) => {
                self.state
                    .connectivity
                    .update_from_account_reconnecting(exchange);
                Some(Strategy::on_disconnect(self, *exchange))
            }
            AccountStreamEvent::Item(event) => {
                let _position = self.state.update_from_account(event);
                None
            }
        }
    }

    /// Update the [`Engine`] from a [`MarketStreamEvent`].
    ///
    /// If the input `MarketStreamEvent` indicates the exchange market data link has disconnected,
    /// the `Engine` will call the configured [`OnDisconnectStrategy`] strategy logic.
    pub fn update_from_market_stream(
        &mut self,
        event: &MarketStreamEvent<InstrumentIndex, MarketState::EventKind>,
    ) -> Option<Strategy::OnDisconnect>
    where
        MarketState: MarketDataState,
        StrategyState: for<'a> Processor<&'a MarketEvent<InstrumentIndex, MarketState::EventKind>>,
        RiskState: for<'a> Processor<&'a MarketEvent<InstrumentIndex, MarketState::EventKind>>,
        Strategy: OnDisconnectStrategy<
            Clock,
            EngineState<MarketState, StrategyState, RiskState>,
            ExecutionTxs,
            Risk,
        >,
    {
        match event {
            MarketStreamEvent::Reconnecting(exchange) => {
                self.state
                    .connectivity
                    .update_from_market_reconnecting(exchange);
                Some(Strategy::on_disconnect(self, *exchange))
            }
            MarketStreamEvent::Item(event) => {
                self.state.update_from_market(event);
                None
            }
        }
    }

    /// Returns a [`TradingSummaryGenerator`] for the current trading session.
    pub fn trading_summary_generator(&self, risk_free_return: Decimal) -> TradingSummaryGenerator
    where
        Clock: EngineClock,
    {
        TradingSummaryGenerator::init(
            risk_free_return,
            self.meta.time_start,
            self.clock.time(),
            &self.state.instruments,
            &self.state.assets,
        )
    }
}

impl<Clock, State, ExecutionTxs, Strategy, Risk> Engine<Clock, State, ExecutionTxs, Strategy, Risk>
where
    Clock: EngineClock,
{
    /// Construct a new `Engine`.
    ///
    /// An initial [`EngineMeta`] is constructed form the provided `clock` and `Sequence(0)`.
    pub fn new(
        clock: Clock,
        state: State,
        execution_txs: ExecutionTxs,
        strategy: Strategy,
        risk: Risk,
    ) -> Self {
        Self {
            meta: EngineMeta {
                time_start: clock.time(),
                sequence: Sequence(0),
            },
            clock,
            state,
            execution_txs,
            strategy,
            risk,
        }
    }

    /// Return `Engine` clock time.
    pub fn time(&self) -> DateTime<Utc> {
        self.clock.time()
    }

    /// Reset the internal `EngineMeta` to the `clock` time and `Sequence(0)`.
    pub fn reset_metadata(&mut self) {
        self.meta.time_start = self.clock.time();
        self.meta.sequence = Sequence(0);
    }
}

impl<Audit, Clock, State, ExecutionTx, StrategyT, Risk> Auditor<Audit>
    for Engine<Clock, State, ExecutionTx, StrategyT, Risk>
where
    Audit: From<State>,
    Clock: EngineClock,
    State: Clone,
{
    type Context = EngineContext;
    type Snapshot = State;
    type Shutdown<Event> = ShutdownAudit<Event>;

    fn snapshot(&self) -> Self::Snapshot {
        self.state.clone()
    }

    fn audit<Kind>(&mut self, kind: Kind) -> AuditTick<Audit, EngineContext>
    where
        Audit: From<Kind>,
    {
        AuditTick {
            event: Audit::from(kind),
            context: EngineContext {
                sequence: self.meta.sequence.fetch_add(),
                time: self.clock.time(),
            },
        }
    }
}

/// Output produced by [`Engine`] operations, used to construct an `Engine` [`Audit`].
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum EngineOutput<
    OnTradingDisabled,
    OnDisconnect,
    ExchangeKey = ExchangeIndex,
    InstrumentKey = InstrumentIndex,
> {
    Commanded(ActionOutput<ExchangeKey, InstrumentKey>),
    OnTradingDisabled(OnTradingDisabled),
    OnDisconnect(OnDisconnect),
    AlgoOrders(GenerateAlgoOrdersOutput<ExchangeKey, InstrumentKey>),
}

impl<OnTradingDisabled, OnDisconnect, ExchangeKey, InstrumentKey>
    From<ActionOutput<ExchangeKey, InstrumentKey>>
    for EngineOutput<OnTradingDisabled, OnDisconnect, ExchangeKey, InstrumentKey>
{
    fn from(value: ActionOutput<ExchangeKey, InstrumentKey>) -> Self {
        Self::Commanded(value)
    }
}

impl<OnTradingDisabled, OnDisconnect, ExchangeKey, InstrumentKey>
    From<GenerateAlgoOrdersOutput<ExchangeKey, InstrumentKey>>
    for EngineOutput<OnTradingDisabled, OnDisconnect, ExchangeKey, InstrumentKey>
{
    fn from(value: GenerateAlgoOrdersOutput<ExchangeKey, InstrumentKey>) -> Self {
        Self::AlgoOrders(value)
    }
}
