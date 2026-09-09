use std::time::Duration;

use alternate_codec::Codec;
use alternate_kv::{KvClient, KvExpiry};
use serde::{Serialize, de::DeserializeOwned};

pub trait KvClientTyped<C: Codec>: KvClient {
    fn get_typed<V: DeserializeOwned>(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<V>, KvTypedError<C::Error, Self::Error>>>;

    fn set_typed<V: Serialize>(
        &self,
        key: &str,
        value: &V,
    ) -> impl Future<Output = Result<(), KvTypedError<C::Error, Self::Error>>>;
}

impl<KC: KvClient, C: Codec> KvClientTyped<C> for KC {
    async fn get_typed<V: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<V>, KvTypedError<C::Error, Self::Error>> {
        match self.get(key).await? {
            Some(raw) => Ok(Some(C::decode(&raw).map_err(KvTypedError::Codec)?)),
            None => Ok(None),
        }
    }

    async fn set_typed<V: Serialize>(
        &self,
        key: &str,
        value: &V,
    ) -> Result<(), KvTypedError<C::Error, Self::Error>> {
        let raw = C::encode(&value).map_err(KvTypedError::Codec)?;

        self.set(key, &raw).await?;

        Ok(())
    }
}

pub trait KvExpiryTyped<C: Codec>: KvExpiry {
    fn set_with_ttl_typed<V: Serialize>(
        &self,
        key: &str,
        value: V,
        ttl: Duration,
    ) -> impl Future<Output = Result<(), KvTypedError<C::Error, Self::Error>>>;
}

impl<KE: KvExpiry, C: Codec> KvExpiryTyped<C> for KE {
    async fn set_with_ttl_typed<V: Serialize>(
        &self,
        key: &str,
        value: V,
        ttl: Duration,
    ) -> Result<(), KvTypedError<C::Error, Self::Error>> {
        let raw = C::encode(&value).map_err(KvTypedError::Codec)?;

        self.set_with_ttl(key, &raw, ttl).await?;

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KvTypedError<CodecErr: std::error::Error, ClientErr: std::error::Error> {
    #[error("codec: {0}")]
    Codec(CodecErr),

    #[error(transparent)]
    Client(#[from] ClientErr),
}
