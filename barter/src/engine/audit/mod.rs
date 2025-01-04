use crate::{
    engine::{
        action::ActionOutput,
        audit::{context::EngineContext, shutdown::ShutdownAudit},
        clock::EngineClock,
        error::UnrecoverableEngineError,
        state::{instrument::market_data::MarketDataState, EngineState},
        Engine, EngineOutput, UpdateFromAccountOutput, UpdateFromMarketOutput,
    },
    strategy::{on_disconnect::OnDisconnectStrategy, on_trading_disabled::OnTradingDisabled},
    EngineEvent,
};
use barter_data::event::DataKind;
use barter_integration::collection::one_or_many::OneOrMany;
use derive_more::Constructor;
use serde::{Deserialize, Serialize};

/// Defines data structures that represent the context an `Engine` [`AuditTick`] was generated.
pub mod context;

/// Defines an `Engine` shutdown audit.
pub mod shutdown;

/// Defines a `StateReplicaManager` that can be used to maintain an `EngineState` replica.
///
/// Useful for supporting non-hot path trading system components such as UIs, web apps, etc.
pub mod state_replica;

/// Convenient type alias for the default `Engine` `AuditTick`.
pub type DefaultAuditTick<
    MarketState: MarketDataState,
    StrategyState,
    RiskState,
    OnTradingDisabled,
    OnDisconnect,
> = AuditTick<
    EngineAudit<
        EngineState<MarketState, StrategyState, RiskState>,
        EngineEvent<MarketState::EventKind>,
        EngineOutput<OnTradingDisabled, OnDisconnect>,
    >,
    EngineContext,
>;

/// Interface that defines how a component (eg/ `Engine`) generates [`AuditTick`]s.
pub trait Auditor<AuditKind>
where
    AuditKind: From<Self::Snapshot>,
{
    /// Full state snapshot.
    type Snapshot;

    type Output;

    /// Shutdown audit.
    ///
    /// For example, the `Engine` [`ShutdownAudit`].
    type Shutdown<Event>;

    /// `AuditTick` context.
    ///
    /// For example, the `Engine` uses [`EngineContext`].
    type Context;

    /// Returns a full state snapshot.
    fn snapshot(&self) -> Self::Snapshot;

    /// Build an `AuditTick`.
    fn audit<Kind>(&mut self, kind: Kind) -> AuditTick<AuditKind, Self::Context>
    where
        AuditKind: From<Kind>;
}

impl<Audit, Clock, State, ExecutionTxs, Strategy, Risk> Auditor<Audit>
    for Engine<Clock, State, ExecutionTxs, Strategy, Risk>
where
    Audit: From<State>,
    Clock: EngineClock,
    State: Clone,
    Strategy: OnTradingDisabled<Clock, State, ExecutionTxs, Risk>
        + OnDisconnectStrategy<Clock, State, ExecutionTxs, Risk>,
{
    type Snapshot = State;
    type Output = EngineOutput<Strategy::OnTradingDisabled, Strategy::OnDisconnect>;
    type Shutdown<Event> = ShutdownAudit<Event, Self::Output>;
    type Context = EngineContext;

    fn snapshot(&self) -> Self::Snapshot {
        self.state.clone()
    }

    fn audit<Kind>(&mut self, kind: Kind) -> AuditTick<Audit, Self::Context>
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

/// `Engine` audit event & it's associated context. Sent via the AuditStream.
///
/// For example, see the `Engine` [`DefaultAuditTick`].
#[derive(
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    Deserialize,
    Serialize,
    Constructor,
)]
pub struct AuditTick<Kind, Context> {
    pub event: Kind,
    pub context: Context,
}

/// Represents [`AuditTick`] types that are generated by the `Engine` and sent via the AuditStream.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum EngineAudit<State, Event, Output> {
    /// `EngineState` snapshot.
    Snapshot(State),
    /// `Engine` processed an `Event`, and produced an output.
    Process(ProcessAudit<Event, Output>),
    /// `Engine` shutdown.
    Shutdown(ShutdownAudit<Event, Output>),
}

impl<State, Event, Output> EngineAudit<State, Event, Output> {
    pub fn snapshot<S>(state: S) -> Self
    where
        S: Into<State>,
    {
        Self::Snapshot(state.into())
    }

    pub fn process<E>(event: E) -> Self
    where
        E: Into<Event>,
    {
        Self::Process(ProcessAudit::Process(event.into()))
    }

    pub fn process_with_output<E, O>(event: E, output: O) -> Self
    where
        E: Into<Event>,
        O: Into<Output>,
    {
        Self::Process(ProcessAudit::ProcessWithOutput(
            event.into(),
            OneOrMany::One(output.into()),
        ))
    }

    pub fn shutdown_commanded<E>(event: E) -> Self
    where
        E: Into<Event>,
    {
        Self::Shutdown(ShutdownAudit::Commanded(event.into()))
    }

    pub fn shutdown_on_err<E, O>(
        event: E,
        unrecoverable: OneOrMany<UnrecoverableEngineError>,
        output: O,
    ) -> Self
    where
        E: Into<Event>,
        O: Into<Output>,
    {
        Self::Shutdown(ShutdownAudit::ErrorWithProcess(
            ProcessAudit::ProcessWithOutput(event.into(), OneOrMany::One(output.into())),
            unrecoverable,
        ))
    }

    pub fn shutdown_on_err_with_process(
        process: ProcessAudit<Event, Output>,
        unrecoverable: OneOrMany<UnrecoverableEngineError>,
    ) -> Self {
        Self::Shutdown(ShutdownAudit::ErrorWithProcess(process, unrecoverable))
    }
}

/// Represents an `Engine` audit of a processed `Event`. May produce [`OneOrMany`] `Output`s.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum ProcessAudit<Event, Output> {
    /// `Engine` processed an `Event`, and produced no output.
    Process(Event),
    /// `Engine` processed an `Event`, and produced an OneOrMany outputs.
    ProcessWithOutput(Event, OneOrMany<Output>),
}

impl<Event, OnTradingDisabled, OnDisconnect>
    ProcessAudit<Event, EngineOutput<OnTradingDisabled, OnDisconnect>>
{
    pub fn with_command_output<E>(event: E, output: ActionOutput) -> Self
    where
        E: Into<Event>,
    {
        Self::ProcessWithOutput(
            event.into(),
            OneOrMany::One(EngineOutput::Commanded(output)),
        )
    }

    pub fn with_trading_state_update<E>(event: E, disabled: Option<OnTradingDisabled>) -> Self
    where
        E: Into<Event>,
    {
        if let Some(disabled) = disabled {
            Self::ProcessWithOutput(
                event.into(),
                OneOrMany::One(EngineOutput::OnTradingDisabled(disabled)),
            )
        } else {
            Self::Process(event.into())
        }
    }

    pub fn with_account_update<E>(event: E, account: UpdateFromAccountOutput<OnDisconnect>) -> Self
    where
        E: Into<Event>,
    {
        match account {
            UpdateFromAccountOutput::None => Self::Process(event.into()),
            UpdateFromAccountOutput::OnDisconnect(disconnect) => Self::ProcessWithOutput(
                event.into(),
                OneOrMany::One(EngineOutput::AccountDisconnect(disconnect)),
            ),
            UpdateFromAccountOutput::PositionExit(position) => Self::ProcessWithOutput(
                event.into(),
                OneOrMany::One(EngineOutput::PositionExit(position)),
            ),
        }
    }

    pub fn with_market_update<E>(event: E, account: UpdateFromMarketOutput<OnDisconnect>) -> Self
    where
        E: Into<Event>,
    {
        match account {
            UpdateFromMarketOutput::None => Self::Process(event.into()),
            UpdateFromMarketOutput::OnDisconnect(disconnect) => Self::ProcessWithOutput(
                event.into(),
                OneOrMany::One(EngineOutput::MarketDisconnect(disconnect)),
            ),
        }
    }
}

impl<Event, Output> ProcessAudit<Event, Output> {
    pub fn add_additional<O>(self, output: O) -> Self
    where
        O: Into<Output>,
    {
        match self {
            ProcessAudit::Process(event) => {
                Self::ProcessWithOutput(event, OneOrMany::One(output.into()))
            }
            ProcessAudit::ProcessWithOutput(event, existing) => {
                Self::ProcessWithOutput(event, existing.extend(OneOrMany::One(output.into())))
            }
        }
    }
}

impl<Market, Strategy, Risk, Event, Output> From<EngineState<Market, Strategy, Risk>>
    for EngineAudit<EngineState<Market, Strategy, Risk>, Event, Output>
{
    fn from(value: EngineState<Market, Strategy, Risk>) -> Self {
        Self::Snapshot(value)
    }
}

impl<State, Event, Output> From<ProcessAudit<Event, Output>> for EngineAudit<State, Event, Output> {
    fn from(value: ProcessAudit<Event, Output>) -> Self {
        Self::Process(value)
    }
}

impl<State, OnTradingDisabled, OnDisconnect>
    From<ShutdownAudit<EngineEvent<DataKind>, EngineOutput<OnTradingDisabled, OnDisconnect>>>
    for EngineAudit<State, EngineEvent<DataKind>, EngineOutput<OnTradingDisabled, OnDisconnect>>
{
    fn from(
        value: ShutdownAudit<EngineEvent<DataKind>, EngineOutput<OnTradingDisabled, OnDisconnect>>,
    ) -> Self {
        Self::Shutdown(value)
    }
}

impl<State, Event, Output> From<&EngineAudit<State, Event, Output>>
    for Option<ShutdownAudit<Event, Output>>
where
    Event: Clone,
    Output: Clone,
{
    fn from(value: &EngineAudit<State, Event, Output>) -> Self {
        match value {
            EngineAudit::Shutdown(shutdown) => Some(shutdown.clone()),
            _ => None,
        }
    }
}
