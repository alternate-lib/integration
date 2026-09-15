use alternate_crypto::encoding;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use crate::ApiError;

/// Paginated list of results
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Paginated<T: utoipa::ToSchema> {
    /// Current set of results
    pub items: Vec<T>,
    /// Pagination cursor, only present if more results are available
    #[schema(nullable = false)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl<T, U: alternate_domain::Cursor> TryFrom<alternate_domain::Paginated<U>> for Paginated<T>
where
    T: From<U> + Serialize + utoipa::ToSchema,
    U::Data: Serialize,
{
    type Error = ApiError;

    fn try_from(value: alternate_domain::Paginated<U>) -> Result<Self, Self::Error> {
        Ok(Self {
            items: value.items.into_iter().map(T::from).collect(),
            cursor: value
                .cursor
                .map(|c| encode_cursor(&c))
                .transpose()
                .map_err(ApiError::server)?,
        })
    }
}

pub fn encode_cursor<T>(data: &T) -> Result<String, serde_json::Error>
where
    T: Serialize,
{
    let serialized = serde_json::to_vec(data)?;

    Ok(encoding::base64_url_encode(&serialized))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor<T>(T);

impl<T> Cursor<T> {
    pub fn new(data: T) -> Self {
        Self(data)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<'de, T> Deserialize<'de> for Cursor<T>
where
    T: DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;

        let decoded_bytes =
            encoding::base64_url_decode(&encoded).map_err(serde::de::Error::custom)?;

        let data = serde_json::from_slice::<T>(&decoded_bytes).map_err(serde::de::Error::custom)?;

        Ok(Cursor::new(data))
    }
}
