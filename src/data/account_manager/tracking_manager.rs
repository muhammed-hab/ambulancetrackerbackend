use std::time::Duration;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use thiserror::Error;
use crate::data::account_manager::{AccountId, PhoneNumber};
use crate::data::ambulance_tracker::Ambulance;

pub struct TrackedAmbulance {
	pub ambulance: Ambulance,
	pub user_label: String,
	pub urgency: String,
	pub phones_tracking: (PhoneNumber, Duration),
	pub eta: DateTime<Utc>,
	pub user_eta_notify: Option<Duration>,
}

#[derive(Debug, Error)]
pub enum UserLookupError {
	#[error("user not found")]
	UserNotFound,
	#[error("other error")]
	OtherError(Box<dyn std::error::Error>),
}
#[derive(Debug, Error)]
pub enum AmbulanceLookupError {
	#[error("ambulance not found")]
	AmbulanceNotFound,
	#[error("user not found")]
	UserNotFound,
	#[error("other error")]
	OtherError(Box<dyn std::error::Error>),
}

pub trait TrackingManager {

	/// Returns a list of which ambulances a user is currently tracking
	async fn get_user_tracking(&self, id: AccountId) -> Result<TrackedAmbulance, UserLookupError>;

	/// Begins tracking an ambulance
	async fn track_ambulance(&self, id: AccountId, ambulance_id: Uuid, user_label: &str, urgency: &str, phones: (Uuid, Duration)) -> Result<(), AmbulanceLookupError>;
	
	/// Dismisses the user eta alert
	async fn dismiss_eta_alert(&self, id: AccountId, ambulance_id: Uuid) -> Result<(), AmbulanceLookupError>;
	
	/// Stops tracking the ambulance for the user
	async fn stop_tracking_ambulance(&self, id: AccountId, ambulance_id: Uuid) -> Result<(), AmbulanceLookupError>;
}
