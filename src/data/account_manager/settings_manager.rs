use std::time::Duration;
use sqlx::types::Uuid;
use thiserror::Error;
use crate::data::account_manager::AccountId;

#[derive(Debug, Clone)]
pub struct PhoneNumber {
	pub phone_id: Uuid,
	pub number: String,
	pub label: String
}
impl PhoneNumber {
	pub fn new(phone_id: Uuid, number: String, label: String) -> PhoneNumber {
		Self { phone_id, number, label }
	}
}

#[derive(Debug, Clone)]
pub struct UserSettings {
	pub hospital_location: geo_types::Point,
	pub default_eta_alert: Duration
}
impl UserSettings {
	pub fn new(hospital_location: geo_types::Point, default_eta_alert: Duration) -> UserSettings {
		Self {hospital_location, default_eta_alert}
	}
}

#[derive(Debug, Error)]
pub enum SettingsError {
	#[error("The specified user cannot be found")]
	UserNotFound,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>),
}

#[derive(Debug, Error)]
pub enum DeletePhoneError {
	#[error("The specified user cannot be found")]
	UserNotFound,
	#[error("the specified phone cannot be found")]
	PhoneNotFound,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>),
}

#[async_trait::async_trait]
pub trait SettingsManager {

	/// Retrieves a user's settings
	async fn get_settings(&self, user_id: AccountId) -> Result<UserSettings, SettingsError>;

	/// Updates a user's settings, replacing it entirely
	async fn set_settings(&self, user_id: AccountId, settings: UserSettings) -> Result<(), SettingsError>;

	/// Returns a list of a user's phones
	async fn get_phones(&self, user_id: AccountId) -> Result<Vec<PhoneNumber>, SettingsError>;

	/// Creates a new phone for a user. Duplicates are allowed.
	async fn new_phone(&self, user_id: AccountId, phone: &str, label: &str) -> Result<(), SettingsError>;

	/// Deletes a phone
	async fn delete_phone(&self, user_id: AccountId, phone_id: Uuid) -> Result<(), DeletePhoneError>;
}