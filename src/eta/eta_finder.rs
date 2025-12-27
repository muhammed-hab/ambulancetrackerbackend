use std::time::Duration;
use geo_types::Point;
use sqlx::types::Uuid;

#[async_trait::async_trait]
pub trait EtaFinder {

	async fn calculate_eta(&self, ambulance_id: Uuid, from: Point, to: Point) -> Result<Duration, Box<dyn std::error::Error>>;

}
