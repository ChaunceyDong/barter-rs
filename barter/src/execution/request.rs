use barter_execution::order::{Order, RequestCancel, RequestOpen};
use barter_instrument::{exchange::ExchangeIndex, instrument::InstrumentIndex};
use derive_more::From;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Convenient type alias for an [`ExecutionRequest`] keyed with [`ExchangeIndex`]
/// and [`InstrumentIndex`].
pub type IndexedExecutionRequest = ExecutionRequest<ExchangeIndex, InstrumentIndex>;

/// Represents an `Engine` request to the `ExecutionManager`.
#[derive(Debug, Clone, PartialEq, PartialOrd, Deserialize, Serialize, From)]
pub enum ExecutionRequest<ExchangeKey, InstrumentKey> {
    /// Request to cancel an existing `Order`.
    Cancel(Order<ExchangeKey, InstrumentKey, RequestCancel>),

    /// Request to open an new `Order`.
    Open(Order<ExchangeKey, InstrumentKey, RequestOpen>),
}

#[derive(Debug)]
#[pin_project::pin_project]
pub(super) struct RequestFuture<Request, ResponseFut> {
    request: Request,
    #[pin]
    response_future: tokio::time::Timeout<ResponseFut>,
}

impl<Request, ResponseFut> Future for RequestFuture<Request, ResponseFut>
where
    Request: Clone,
    ResponseFut: Future,
{
    type Output = Result<ResponseFut::Output, Request>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.response_future
            .poll(cx)
            .map(|result| result.map_err(|_| this.request.clone()))
    }
}

impl<Request, ResponseFut> RequestFuture<Request, ResponseFut>
where
    ResponseFut: Future,
{
    pub fn new(future: ResponseFut, timeout: std::time::Duration, request: Request) -> Self {
        Self {
            request,
            response_future: tokio::time::timeout(timeout, future),
        }
    }
}
