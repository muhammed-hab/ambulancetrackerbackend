use crate::eta::eta_finder::EtaFinder;
use geo_types::{Geometry, Point};
use geozero::wkb;
use sqlx::types::chrono::Utc;
use sqlx::types::Uuid;
use sqlx::PgPool;
use std::error::Error;
use std::time::Duration;

pub struct ArchiveEta(PgPool, Box<dyn EtaFinder + 'static + Sync + Send>);

/// A wrapper over an ETA finder which uses the SQL backend to archive an ETA whenever a new one is
/// calculated. Expects that [migrations/1_archive.sql] has been executed already.
#[async_trait::async_trait]
impl EtaFinder for ArchiveEta {
	async fn calculate_eta(&self, ambulance_id: Uuid, from: Point, to: Point) -> Result<Duration, Box<dyn Error>> {
		let eta = self.1.calculate_eta(ambulance_id, from, to).await?;

		sqlx::query("INSERT INTO archive_etas(ambulance_id, current_location, destination, eta, calculated_at) VALUES ($1, $2, $3, $4, $5)")
			.bind(ambulance_id)
			.bind(wkb::Encode::<Geometry>(from.into()))
			.bind(wkb::Encode::<Geometry>(to.into()))
			.bind(eta)
			.bind(Utc::now())
			.execute(&self.0)
			.await?;

		Ok(eta)
	}
}

impl ArchiveEta {
	pub fn new(pool: PgPool, finder: Box<dyn EtaFinder + 'static + Sync + Send>) -> Self {
		Self(pool, finder)
	}
}