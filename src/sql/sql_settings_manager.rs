use geo_types::Geometry;
use geozero::wkb;
use sqlx::{Error, PgPool};
use sqlx::postgres::types::PgInterval;
use sqlx::types::Uuid;
use crate::data::{AccountId, DeletePhoneError, PhoneNumber, SettingsError, SettingsManager, UserSettings};
use crate::sql::interval_conversion::convert_interval;

pub struct SQLSettingsManager(PgPool);

#[inline(always)]
fn phone_pretty(phone: &str) -> String {
	format!("({}) {}-{}", &phone[0..3], &phone[3..6], &phone[6..10])
}

#[async_trait::async_trait]
impl SettingsManager for SQLSettingsManager {
	async fn get_settings(&self, user_id: AccountId) -> Result<UserSettings, SettingsError> {
		match
			sqlx::query_as::<_, (wkb::Decode<Geometry>, PgInterval)>("SELECT hospital, pref_eta FROM accounts WHERE user_id = $1")
				.bind(user_id.0)
				.fetch_optional(&self.0)
				.await
				.map_err(|e| SettingsError::Other(e.into()))? {
			Some((hospital_location, pref_eta)) => Ok(UserSettings {
				hospital_location: hospital_location.geometry.map(|p| p.try_into().expect("invalid database backing")),
				default_eta_alert: convert_interval(pref_eta)
			}),
			None => Err(SettingsError::UserNotFound)
		}
	}

	async fn set_settings(&self, user_id: AccountId, settings: UserSettings) -> Result<(), SettingsError> {
		let interval = PgInterval::try_from(settings.default_eta_alert).map_err(|e| SettingsError::Other(e))?;
		match sqlx::query_as::<_, (i32,)>("UPDATE accounts SET hospital=$2, pref_eta=$3 WHERE user_id=$1 RETURNING 1;")
			.bind(user_id.0)
			.bind(settings.hospital_location.map(|pt| wkb::Encode::<Geometry>(pt.into())))
			.bind(interval)
			.fetch_optional(&self.0)
			.await
			.map_err(|e| SettingsError::Other(e.into()))? {
			Some(_) => Ok(()),
			None => Err(SettingsError::UserNotFound)
		}
	}

	async fn get_phones(&self, user_id: AccountId) -> Result<Vec<PhoneNumber>, SettingsError> {
		// ensure user exists
		if sqlx::query_as::<_, (i32,)>("SELECT 1 FROM accounts WHERE user_id=$1")
			.bind(user_id.0).fetch_optional(&self.0).await.map_err(|e| SettingsError::Other(e.into()))?.is_none() {
			return Err(SettingsError::UserNotFound);
		}

		Ok(
			sqlx::query_as::<_, (Uuid, String, Option<String>)>("SELECT phone_id, phone, label FROM phone_numbers WHERE user_id=$1")
				.bind(user_id.0)
				.fetch_all(&self.0)
				.await
				.map_err(|e| SettingsError::Other(e.into()))?
				.into_iter()
				.map(|(phone_id, phone, label)| PhoneNumber {
					phone_id,
					label: label.unwrap_or_else(|| phone_pretty(&*phone)),
					number: phone,
				})
				.collect()
		)
	}

	async fn new_phone(&self, user_id: AccountId, phone: &str, label: &str) -> Result<PhoneNumber, SettingsError> {
		match sqlx::query_as::<_, (Uuid,)>("INSERT INTO phone_numbers(user_id, phone, label) VALUES ($1, $2, $3) RETURNING phone_id")
			.bind(user_id.0)
			.bind(phone)
			.bind(label)
			.fetch_one(&self.0)
			.await {
				Err(Error::Database(db)) if db.is_foreign_key_violation() => Err(SettingsError::UserNotFound),
				Err(e) => Err(SettingsError::Other(e.into())),
				Ok((phone_id, )) => Ok(PhoneNumber {
					phone_id,
					label: label.to_string(),
					number: phone.to_string()
				})
			}
	}

	async fn delete_phone(&self, user_id: AccountId, phone_id: Uuid) -> Result<(), DeletePhoneError> {
		match sqlx::query_as::<_, (i32,)>("DELETE FROM phone_numbers WHERE user_id=$1 AND phone_id=$2 RETURNING 1;")
			.bind(user_id.0)
			.bind(phone_id)
			.fetch_optional(&self.0)
			.await
			.map_err(|e| DeletePhoneError::Other(e.into()))? {
			Some(_) => Ok(()),
			None => Err(DeletePhoneError::PhoneNotFound)
		}
	}
}

impl SQLSettingsManager {
	/// Creates a new AmbulanceTracker using the specified connection as the backend.
	/// It is expected that the migrations file has been executed already.
	pub fn new(pool: PgPool) -> Self {
		Self(pool)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;
	use crate::data::{AccountManager, AccountRole};
	use crate::sql::sql_account_manager::SqlAccountManager;

	// Helper to setup the mock SettingsManager and AccountIds
	async fn get_settings_manager(pool: PgPool) -> Result<(impl SettingsManager, AccountId, AccountId, AccountId, AccountId), Box<dyn std::error::Error>> {
		let acc = SqlAccountManager::new(pool.clone());
		let (user1, _) = acc.create_site_admin("user1").await?;
		let (user2, _) = acc.create_account(&user1, AccountRole::Admin, "user2").await?;
		let (user3, _) = acc.create_account(&user2, AccountRole::User, "user3").await?;
		let (non_existent_user, _) = acc.create_account(&user2, AccountRole::User, "fake").await?;
		acc.delete_account(&user2, &non_existent_user).await?;

		Ok((SQLSettingsManager::new(pool), user1, user2, user3, non_existent_user))
	}

	#[sqlx::test]
	async fn test_get_settings_existing_user(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		// Assuming the mock returns some pre-set settings
		let result = settings_manager.get_settings(user1).await;
		assert!(result.is_ok());

		let settings = result.unwrap();
		assert_eq!(settings.default_eta_alert, Duration::from_secs(60 * 15));
	}

	#[sqlx::test]
	async fn test_get_settings_non_existent_user(pool: PgPool) {
		let (settings_manager, _, _, _, non_existent_user) = get_settings_manager(pool).await.unwrap();

		let result = settings_manager.get_settings(non_existent_user).await;
		assert!(result.is_err());
		match result {
			Err(SettingsError::UserNotFound) => (),
			_ => panic!("Expected UserNotFound error"),
		}
	}

	#[sqlx::test]
	async fn test_set_settings_existing_user(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		let new_settings = UserSettings {
			hospital_location: Some(geo_types::Point::new(40.7128, -74.0060)),
			default_eta_alert: Duration::new(7200, 0), // 2 hours
		};

		let result = settings_manager.set_settings(user1, new_settings.clone()).await;
		assert!(result.is_ok(), "failed: {:?}", result);

		// Retrieve the updated settings and check
		let retrieved_settings = settings_manager.get_settings(user1).await.unwrap();
		assert_eq!(retrieved_settings.default_eta_alert, new_settings.default_eta_alert);
		assert_eq!(retrieved_settings.hospital_location, new_settings.hospital_location); // Example check for lat
	}

	#[sqlx::test]
	async fn test_set_settings_non_existent_user(pool: PgPool) {
		let (settings_manager, _, _, _, non_existent_user) = get_settings_manager(pool).await.unwrap();

		let new_settings = UserSettings {
			hospital_location: Some(geo_types::Point::new(40.7128, -74.0060)),
			default_eta_alert: Duration::new(7200, 0),
		};

		let result = settings_manager.set_settings(non_existent_user, new_settings).await;
		assert!(result.is_err());
		match result {
			Err(SettingsError::UserNotFound) => (),
			result => panic!("Expected UserNotFound error, found {:?}", result),
		}
	}

	#[sqlx::test]
	async fn test_get_phones_existing_user(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		// Mock phone list for user1
		let result = settings_manager.get_phones(user1).await;
		assert!(result.is_ok());

		let phones = result.unwrap();
		assert_eq!(phones.len(), 0);
	}

	#[sqlx::test]
	async fn test_get_phones_non_existent_user(pool: PgPool) {
		let (settings_manager, _, _, _, non_existent_user) = get_settings_manager(pool).await.unwrap();

		let result = settings_manager.get_phones(non_existent_user).await;
		assert!(result.is_err());
		match result {
			Err(SettingsError::UserNotFound) => (),
			_ => panic!("Expected UserNotFound error"),
		}
	}

	#[sqlx::test]
	async fn test_new_phone_existing_user(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		let phone = "9876543210";
		let label = "Home";

		let result = settings_manager.new_phone(user1, phone, label).await;
		assert!(result.is_ok());

		// Check if the phone is added
		let phones = settings_manager.get_phones(user1).await.unwrap();
		assert_eq!(phones.len(), 1);
		assert_eq!(phones[0].label, label);
		assert_eq!(phones[0].number, phone)
	}

	#[sqlx::test]
	async fn test_new_phone_non_existent_user(pool: PgPool) {
		let (settings_manager, _, _, _, non_existent_user) = get_settings_manager(pool).await.unwrap();

		let phone = "9876543210";
		let label = "Home";

		let result = settings_manager.new_phone(non_existent_user, phone, label).await;
		assert!(result.is_err());
		match result {
			Err(SettingsError::UserNotFound) => (),
			_ => panic!("Expected UserNotFound error"),
		}
	}

	#[sqlx::test]
	async fn test_delete_phone_existing_user(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		let phone_id = settings_manager.new_phone(user1, "0123456789", "label").await.unwrap().phone_id;

		let result = settings_manager.delete_phone(user1, phone_id).await;
		assert!(result.is_ok());

		// Check that the phone is removed
		let phones = settings_manager.get_phones(user1).await.unwrap();
		assert!(phones.iter().all(|p| p.phone_id != phone_id));
	}

	#[sqlx::test]
	async fn test_delete_phone_non_existent_user(pool: PgPool) {
		let (settings_manager, user1, _, _, non_existent_user) = get_settings_manager(pool).await.unwrap();

		let phone_id = settings_manager.new_phone(user1, "0123456789", "label").await.unwrap().phone_id;
		let result = settings_manager.delete_phone(non_existent_user, phone_id).await;
		assert!(result.is_err());
		match result {
			Err(DeletePhoneError::PhoneNotFound) => (),
			result => panic!("Expected PhoneNotFound error, found {:?}", result),
		}
	}

	#[sqlx::test]
	async fn test_delete_non_existent_phone(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		let phone_id = settings_manager.new_phone(user1, "0123456789", "label").await.unwrap().phone_id;
		settings_manager.delete_phone(user1, phone_id).await.unwrap();

		let result = settings_manager.delete_phone(user1, phone_id).await;
		assert!(result.is_err());
		match result {
			Err(DeletePhoneError::PhoneNotFound) => (),
			_ => panic!("Expected PhoneNotFound error"),
		}
	}

	#[sqlx::test]
	async fn test_new_phone_duplicate_phone(pool: PgPool) {
		let (settings_manager, user1, _, _, _) = get_settings_manager(pool).await.unwrap();

		let phone = "1234567890";
		let label = "Mobile";

		// Adding duplicate phone
		let result = settings_manager.new_phone(user1, phone, label).await;
		assert!(result.is_ok());

		let result = settings_manager.new_phone(user1, phone, label).await;
		assert!(result.is_ok());

		// Check for duplicates
		let phones = settings_manager.get_phones(user1).await.unwrap();
		assert_eq!(phones.len(), 2); // Both phones should be there (duplicate allowed)
	}
}