use std::time::Duration;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct Ambulance {
	pub id: Uuid,
	pub name: String,
	pub location: geo_types::Point,
	pub last_updated: DateTime<Utc>
}

#[derive(Debug, Error)]
pub enum AmbulanceTrackerError {
	#[error("ambulance not found")]
	AmbulanceNotFound,
	#[error("other error: {0}")]
	Other(Box<dyn std::error::Error>),
}

#[async_trait::async_trait]
pub trait AmbulanceTracker {

	/// Adds a new ambulance to be tracked, returning the new entry's information
	async fn add_ambulance(&self, name: &str, location: geo_types::Point, fetched: DateTime<Utc>)
		-> Result<Ambulance, Box<dyn std::error::Error>>;

	/// Updates an ambulances current location if and only if the fetched time is after the previous
	/// fetched time.
	async fn update_ambulance(&self, id: Uuid, location: geo_types::Point, fetched: DateTime<Utc>)
		-> Result<(), AmbulanceTrackerError>;

	/// Returns a list of ambulances which have had location updates within the specified duration
	async fn get_recently_updated(&self, last_updated: Duration)
		-> Result<Vec<Ambulance>, Box<dyn std::error::Error>>;

	/// Returns the ambulance
	async fn get_ambulance(&self, id: Uuid) -> Result<Option<Ambulance>, Box<dyn std::error::Error>>;

}
