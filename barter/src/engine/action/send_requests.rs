use crate::{
    engine::{
        error::{EngineError, RecoverableEngineError, UnrecoverableEngineError},
        execution_tx::ExecutionTxMap,
        Engine,
    },
    execution::request::ExecutionRequest,
};
use barter_execution::order::{Order, RequestCancel, RequestOpen};
use barter_instrument::{exchange::ExchangeIndex, instrument::InstrumentIndex};
use barter_integration::{channel::Tx, collection::none_one_or_many::NoneOneOrMany, Unrecoverable};
use derive_more::Constructor;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tracing::error;

/// Trait that defines how the [`Engine`] sends order requests.
///
/// # Type Parameters
/// * `ExchangeKey` - Type used to identify an exchange (defaults to [`ExchangeIndex`]).
/// * `InstrumentKey` - Type used to identify an instrument (defaults to [`InstrumentIndex`]).
pub trait SendRequests<ExchangeKey = ExchangeIndex, InstrumentKey = InstrumentIndex> {
    fn send_requests<Kind>(
        &self,
        requests: impl IntoIterator<Item = Order<ExchangeKey, InstrumentKey, Kind>>,
    ) -> SendRequestsOutput<ExchangeKey, InstrumentKey, Kind>
    where
        Kind: Debug + Clone,
        ExecutionRequest<ExchangeKey, InstrumentKey>: From<Order<ExchangeKey, InstrumentKey, Kind>>;

    fn send_request<Kind>(
        &self,
        request: &Order<ExchangeKey, InstrumentKey, Kind>,
    ) -> Result<(), EngineError>
    where
        Kind: Debug + Clone,
        ExecutionRequest<ExchangeKey, InstrumentKey>: From<Order<ExchangeKey, InstrumentKey, Kind>>;
}

impl<Clock, State, ExecutionTxs, Strategy, Risk, ExchangeKey, InstrumentKey>
    SendRequests<ExchangeKey, InstrumentKey> for Engine<Clock, State, ExecutionTxs, Strategy, Risk>
where
    ExecutionTxs: ExecutionTxMap<ExchangeKey, InstrumentKey>,
    ExchangeKey: Debug + Clone,
    InstrumentKey: Debug + Clone,
{
    fn send_requests<Kind>(
        &self,
        requests: impl IntoIterator<Item = Order<ExchangeKey, InstrumentKey, Kind>>,
    ) -> SendRequestsOutput<ExchangeKey, InstrumentKey, Kind>
    where
        Kind: Debug + Clone,
        ExecutionRequest<ExchangeKey, InstrumentKey>: From<Order<ExchangeKey, InstrumentKey, Kind>>,
    {
        // Send order requests
        let (sent, errors): (Vec<_>, Vec<_>) = requests
            .into_iter()
            .map(|request| {
                self.send_request(&request)
                    .map_err(|error| (request.clone(), error))
                    .map(|_| request)
            })
            .partition_result();

        SendRequestsOutput::new(NoneOneOrMany::from(sent), NoneOneOrMany::from(errors))
    }

    fn send_request<Kind>(
        &self,
        request: &Order<ExchangeKey, InstrumentKey, Kind>,
    ) -> Result<(), EngineError>
    where
        Kind: Debug + Clone,
        ExecutionRequest<ExchangeKey, InstrumentKey>: From<Order<ExchangeKey, InstrumentKey, Kind>>,
    {
        match self
            .execution_txs
            .find(&request.exchange)?
            .send(ExecutionRequest::from(request.clone()))
        {
            Ok(()) => Ok(()),
            Err(error) if error.is_unrecoverable() => {
                error!(
                    exchange = ?request.exchange,
                    ?request,
                    ?error,
                    "failed to send ExecutionRequest due to terminated channel"
                );
                Err(EngineError::Unrecoverable(
                    UnrecoverableEngineError::ExecutionChannelTerminated(format!(
                        "{:?} execution channel terminated: {:?}",
                        request.exchange, error
                    )),
                ))
            }
            Err(error) => {
                error!(
                    exchange = ?request.exchange,
                    ?request,
                    ?error,
                    "failed to send ExecutionRequest due to unhealthy channel"
                );
                Err(EngineError::Recoverable(
                    RecoverableEngineError::ExecutionChannelUnhealthy(format!(
                        "{:?} execution channel unhealthy: {:?}",
                        request.exchange, error
                    )),
                ))
            }
        }
    }
}

/// Summary of cancel and open order requests sent by the [`Engine`] to the `ExecutionManager`.
#[derive(
    Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Constructor,
)]
pub struct SendCancelsAndOpensOutput<ExchangeKey, InstrumentKey> {
    /// Cancel order requests that were sent for execution.
    pub cancels: SendRequestsOutput<ExchangeKey, InstrumentKey, RequestCancel>,
    /// Open order requests that were sent for execution.
    pub opens: SendRequestsOutput<ExchangeKey, InstrumentKey, RequestOpen>,
}

impl<ExchangeKey, InstrumentKey> SendCancelsAndOpensOutput<ExchangeKey, InstrumentKey> {
    /// Returns `true` if no `SendCancelsAndOpensOutput` is completely empty.
    pub fn is_empty(&self) -> bool {
        self.cancels.is_empty() && self.opens.is_empty()
    }

    /// Returns any unrecoverable errors that occurred during order request sending.
    pub fn unrecoverable_errors(&self) -> NoneOneOrMany<UnrecoverableEngineError> {
        self.cancels
            .unrecoverable_errors()
            .extend(self.opens.unrecoverable_errors())
    }
}

impl<ExchangeKey, InstrumentKey> Default for SendCancelsAndOpensOutput<ExchangeKey, InstrumentKey> {
    fn default() -> Self {
        Self {
            cancels: SendRequestsOutput::default(),
            opens: SendRequestsOutput::default(),
        }
    }
}

/// Summary of order requests (cancel _or_ open) sent by the [`Engine`] to the `ExecutionManager`.
#[derive(
    Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Constructor,
)]
pub struct SendRequestsOutput<ExchangeKey, InstrumentKey, Kind> {
    pub sent: NoneOneOrMany<Order<ExchangeKey, InstrumentKey, Kind>>,
    pub errors: NoneOneOrMany<(Order<ExchangeKey, InstrumentKey, Kind>, EngineError)>,
}

impl<ExchangeKey, InstrumentKey, Kind> SendRequestsOutput<ExchangeKey, InstrumentKey, Kind> {
    /// Returns `true` if no `SendRequestsOutput` is completely empty.
    pub fn is_empty(&self) -> bool {
        self.sent.is_none() && self.errors.is_none()
    }

    /// Returns any unrecoverable errors that occurred during order request sending.
    pub fn unrecoverable_errors(&self) -> NoneOneOrMany<UnrecoverableEngineError> {
        self.errors
            .iter()
            .filter_map(|(_order, error)| match error {
                EngineError::Unrecoverable(error) => Some(error.clone()),
                _ => None,
            })
            .collect()
    }
}

impl<ExchangeKey, InstrumentKey, Kind> Default
    for SendRequestsOutput<ExchangeKey, InstrumentKey, Kind>
{
    fn default() -> Self {
        Self {
            sent: NoneOneOrMany::default(),
            errors: NoneOneOrMany::default(),
        }
    }
}
