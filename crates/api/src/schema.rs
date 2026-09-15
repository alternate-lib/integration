use uuid::Uuid;

#[derive(Debug, Clone, serde::Deserialize, utoipa::IntoParams)]
#[into_params(names("id"))]
pub struct Id(
    /// Unique identifier of the resource
    pub Uuid,
);

#[derive(utoipa::ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
pub struct Binary(());

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct CreatedResource {
    /// Unique identifier of the resource
    pub id: Uuid,
}
