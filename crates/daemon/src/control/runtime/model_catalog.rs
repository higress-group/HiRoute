use super::*;
use hiroute_application::model_catalog::{ModelCatalogPort, ModelRatings, RatingQueryError};
use hiroute_application_api::{ErrorCode, ModelCatalogQueryV1, ModelCatalogResultV1};
impl ModelCatalogPort for LocalControlAdapter {
    fn reference_query(
        &self,
        query: ModelCatalogQueryV1,
    ) -> Result<ModelCatalogResultV1, ErrorCode> {
        let catalog = self.current_catalog().ok_or(ErrorCode::DaemonUnavailable)?;
        let snapshot = catalog.rating_snapshot();
        match query {
            ModelCatalogQueryV1::Reference {
                model_configuration_id,
            } => {
                let model = hiroute_domain::model_reference_view(
                    catalog.model_data(),
                    catalog.native_reasoning(),
                    vec![snapshot.snapshot_ref()],
                    &model_configuration_id,
                )
                .ok_or(ErrorCode::ResourceNotFound)?;
                Ok(ModelCatalogResultV1::Reference {
                    model: Box::new(model),
                })
            }
            ModelCatalogQueryV1::Ratings { query } => {
                let ratings = ModelRatings::default();
                ratings
                    .install(snapshot.clone())
                    .map_err(|_| ErrorCode::DaemonUnavailable)?;
                let result = ratings.resolve(&query).map_err(|e| match e {
                    RatingQueryError::InvalidArguments => ErrorCode::InvalidArguments,
                    RatingQueryError::SnapshotUnavailable => ErrorCode::SnapshotUnavailable,
                })?;
                Ok(ModelCatalogResultV1::Ratings { result })
            }
        }
    }
}
