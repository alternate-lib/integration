use std::collections::BTreeMap;

use alternate_codec::Codec;
use alternate_queue::{QueueConsumer, QueueMessage, QueueProducer};
use serde::{Serialize, de::DeserializeOwned};

pub trait QueueProducerTyped<C: Codec> {
    type MessageId;
    type Error: std::error::Error;

    fn send_typed<P: Serialize + Send>(
        &self,
        message: QueueMessageTyped<P>,
    ) -> impl Future<Output = Result<Self::MessageId, Self::Error>> + Send;
}

impl<QP: QueueProducer + Sync, C: Codec> QueueProducerTyped<C> for QP {
    type MessageId = QP::MessageId;
    type Error = QueueTypedError<C::Error, QP::Error>;

    async fn send_typed<P: Serialize>(
        &self,
        message: QueueMessageTyped<P>,
    ) -> Result<Self::MessageId, Self::Error> {
        let raw = C::encode(&message.payload).map_err(QueueTypedError::Codec)?;

        let message_id = self
            .send(QueueMessage {
                id: message.id,
                payload: raw,
                attributes: message.attributes,
            })
            .await?;

        Ok(message_id)
    }
}

pub trait QueueConsumerTyped<C: Codec> {
    type Receipt;
    type Error: std::error::Error;

    fn receive_typed<P: DeserializeOwned>(
        &self,
    ) -> impl Future<Output = Result<Option<QueueDeliveryTyped<P, Self::Receipt>>, Self::Error>> + Send;
}

impl<QC: QueueConsumer + Sync, C: Codec> QueueConsumerTyped<C> for QC {
    type Receipt = QC::Receipt;
    type Error = QueueTypedError<C::Error, QC::Error>;

    async fn receive_typed<P: DeserializeOwned>(
        &self,
    ) -> Result<Option<QueueDeliveryTyped<P, Self::Receipt>>, Self::Error> {
        match self.receive().await? {
            Some(delivery) => Ok(Some(
                C::decode(&delivery.payload)
                    .map(|p| QueueDeliveryTyped {
                        id: delivery.id,
                        payload: p,
                        attempts: delivery.attempts,
                        attributes: delivery.attributes,
                        receipt: delivery.receipt,
                    })
                    .map_err(QueueTypedError::Codec)?,
            )),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueMessageTyped<P: Serialize> {
    pub id: Option<String>,
    pub payload: P,
    pub attributes: BTreeMap<String, String>,
}

impl<P: Serialize> QueueMessageTyped<P> {
    pub fn new(payload: P) -> Self {
        Self {
            id: None,
            payload,
            attributes: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct QueueDeliveryTyped<P: DeserializeOwned, R> {
    pub id: String,
    pub payload: P,
    pub attempts: usize,
    pub attributes: BTreeMap<String, String>,
    pub receipt: R,
}

#[derive(Debug, thiserror::Error)]
pub enum QueueTypedError<CodecErr: std::error::Error, QueueErr: std::error::Error> {
    #[error("codec: {0}")]
    Codec(CodecErr),

    #[error(transparent)]
    Queue(#[from] QueueErr),
}
