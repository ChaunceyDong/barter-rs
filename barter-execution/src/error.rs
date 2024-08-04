use crate::order::ClientOrderId;
use barter_instrument::{
    asset::{name::AssetNameExchange, AssetIndex},
    instrument::{name::InstrumentNameExchange, InstrumentIndex},
};
use barter_integration::error::SocketError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type IndexedClientError = ClientError<AssetIndex, InstrumentIndex>;
pub type UnindexedClientError = ClientError<AssetNameExchange, InstrumentNameExchange>;
pub type IndexedApiError = ApiError<AssetIndex, InstrumentIndex>;
pub type UnindexedApiError = ApiError<AssetNameExchange, InstrumentNameExchange>;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Error)]
pub enum ClientError<AssetKey, InstrumentKey> {
    #[error("Connectivity: {0}")]
    Connectivity(#[from] ConnectivityError),

    #[error("API: {0}")]
    Api(#[from] ApiError<AssetKey, InstrumentKey>),

    #[error("failed to fetch AccountSnapshot: {0}")]
    AccountSnapshot(String),

    #[error("failed to init AccountStream: {0}")]
    AccountStream(String),
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Error)]
pub enum ConnectivityError {
    #[error("{0}")]
    Socket(String),

    #[error("ExecutionRequest timed out")]
    Timeout,
}

impl From<SocketError> for ConnectivityError {
    fn from(value: SocketError) -> Self {
        Self::Socket(value.to_string())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Error)]
pub enum ApiError<AssetKey, InstrumentKey> {
    #[error("rate limit exceeded")]
    RateLimit,
    #[error("asset {0} invalid: {1}")]
    AssetInvalid(AssetKey, String),
    #[error("instrument {0} invalid: {1}")]
    InstrumentInvalid(InstrumentKey, String),
    #[error("asset {0} balance insufficient: {1}")]
    BalanceInsufficient(AssetKey, String),
    #[error("order rejected with ClientOrderId: {0}")]
    OrderRejected(ClientOrderId),
    #[error("order already cancelled with ClientOrderId: {0}")]
    OrderAlreadyCancelled(ClientOrderId),
    #[error("order already fully filled with ClientOrderId: {0}")]
    OrderAlreadyFullyFilled(ClientOrderId),
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize, Error)]
pub enum KeyError {
    #[error("ExchangeId: {0}")]
    ExchangeId(String),

    #[error("AssetKey: {0}")]
    AssetKey(String),

    #[error("InstrumentKey: {0}")]
    InstrumentKey(String),
}
