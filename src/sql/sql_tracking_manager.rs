use crate::data::{AccountId, Ambulance, AmbulanceLookupError, PhoneNumber, TrackedAmbulance, TrackingManager, UserLookupError};
use crate::sql::interval_conversion::convert_interval;
use geo_types::Geometry;
use geozero::wkb;
use sqlx::postgres::types::PgInterval;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use sqlx::{Error, PgPool};
use std::time::Duration;

pub struct SQLTrackingManager(PgPool);

#[inline(always)]
fn phone_pretty(phone: &str) -> String {
	format!("({}) {}-{}", &phone[0..3], &phone[3..6], &phone[6..10])
}

#[async_trait::async_trait]
impl TrackingManager for SQLTrackingManager {
	async fn get_user_tracking(&self, id: AccountId) -> Result<Vec<TrackedAmbulance>, UserLookupError> {
		// ensure user exists
		if sqlx::query_as::<_, (i32,)>("SELECT 1 FROM accounts WHERE user_id=$1")
			.bind(id.0).fetch_optional(&self.0).await.map_err(|e| UserLookupError::OtherError(e.into()))?.is_none() {
			return Err(UserLookupError::UserNotFound);
		}

		let ambulances =
			sqlx::query_as::<_, (Uuid, wkb::Decode<Geometry>, DateTime<Utc>, Option<String>, Option<String>, Option<String>, Option<DateTime<Utc>>, Option<PgInterval>, Vec<(PgInterval, String, Option<String>, Uuid)>)>
				("SELECT amb.ambulance_id, amb.location, amb.last_update, amb.ambulance_name, lts.user_description, lts.urgency, lts.eta, lts.notify_self_at, 
       ARRAY(SELECT ROW(etas.notify_at_eta, ph.phone, ph.label, ph.phone_id) FROM eta_notifications as etas INNER JOIN phone_numbers as ph ON etas.phone_id=ph.phone_id WHERE etas.tracking_id=lts.tracking_id)
		FROM live_tracking_sessions AS lts INNER JOIN ambulances AS amb ON amb.ambulance_id=lts.ambulance_id WHERE lts.user_id=$1;")
				.bind(id.0)
				.fetch_all(&self.0)
				.await
				.map_err(|e| UserLookupError::OtherError(e.into()))?;
		
		Ok(
			ambulances
				.into_iter()
				.map(|(ambulance_id, ambulance_location, ambulance_updated, ambulance_name, user_desc, urgency, eta, notify_self_at, phones)| 
				TrackedAmbulance {
					ambulance: Ambulance {
						id: ambulance_id,
						name: ambulance_name.unwrap_or_else(|| ambulance_id.to_string()),
						location: ambulance_location.geometry.expect("field should be not null").try_into().expect("field should be point"),
						last_updated: ambulance_updated,
					},
					user_label: user_desc.unwrap_or("Tracked Ambulance".to_string()),
					urgency: urgency.unwrap_or("Unspecified".to_string()),
					phones_tracking: phones.into_iter().map(|(notify_at_eta, phone, label, phone_id)| (PhoneNumber {
						phone_id,
						label: label.unwrap_or_else(|| phone_pretty(&*phone)),
						number: phone,
					}, convert_interval(notify_at_eta))).collect(),
					eta,
					user_eta_notify: notify_self_at.map(convert_interval),
				})
				.collect()
		)
	}

	async fn track_ambulance(&self, id: AccountId, ambulance_id: Uuid, user_label: &str, urgency: &str, phones: &[(Uuid, Duration)], notify_self_at: Option<Duration>) -> Result<(), AmbulanceLookupError> {
		let notify_self_at = match notify_self_at {
			Some(duration) => Some(PgInterval::try_from(duration).map_err(|e: Box<dyn std::error::Error + Send + Sync + 'static>| AmbulanceLookupError::OtherError(e))?),
			None => None
		};

		let mut tx = self.0.begin().await.map_err(|e| AmbulanceLookupError::OtherError(e.into()))?;
		
		let (tracking_id, ) =
			match sqlx::query_as::<_, (Uuid,)>("INSERT INTO live_tracking_sessions(user_id, ambulance_id, user_description, urgency, notify_self_at) VALUES ($1, $2, $3, $4, $5) RETURNING tracking_id;")
			.bind(id.0)
			.bind(ambulance_id)
			.bind(user_label)
			.bind(urgency)
			.bind(notify_self_at)
			.fetch_one(&mut *tx)
			.await {
			Err(Error::Database(db)) if db.is_foreign_key_violation() => Err(AmbulanceLookupError::AmbulanceNotTracked),
			Err(e) => Err(AmbulanceLookupError::OtherError(e.into())),
			Ok(e) => Ok(e)
		}?;

		let phone_notifies = phones.iter().map(|(_, notify_at)| PgInterval::try_from(*notify_at)).collect::<Result<Vec<_>, _>>().map_err(|e: Box<dyn std::error::Error + Send + Sync + 'static>| AmbulanceLookupError::OtherError(e))?;

		sqlx::query("INSERT INTO eta_notifications(tracking_id, notify_at_eta, phone_id) SELECT * FROM UNNEST($1::UUID[], $2::INTERVAL[], $3::UUID[]);")
			.bind(phones.iter().map(|_| tracking_id).collect::<Vec<_>>())
			.bind(phone_notifies)
			.bind(phones.iter().map(|(phone_id, _)| *phone_id).collect::<Vec<_>>())
			.execute(&mut *tx)
			.await
			.map_err(|e| AmbulanceLookupError::OtherError(e.into()))?;

		tx.commit().await.map_err(|e| AmbulanceLookupError::OtherError(e.into()))?;

		Ok(())
	}

	async fn dismiss_eta_alert(&self, id: AccountId, ambulance_id: Uuid) -> Result<(), AmbulanceLookupError> {
		if sqlx::query("UPDATE live_tracking_sessions SET notify_self_at=NULL WHERE user_id=$1 AND ambulance_id=$2;")
			.bind(id.0)
			.bind(ambulance_id)
			.execute(&self.0)
			.await
			.map_err(|e| AmbulanceLookupError::OtherError(e.into()))?
			.rows_affected() > 0 {
			Ok(())
		} else {
			Err(AmbulanceLookupError::AmbulanceNotTracked)
		}
	}

	async fn stop_tracking_ambulance(&self, id: AccountId, ambulance_id: Uuid) -> Result<(), AmbulanceLookupError> {
		if sqlx::query("DELETE FROM live_tracking_sessions WHERE user_id=$1 AND ambulance_id=$2;")
			.bind(id.0)
			.bind(ambulance_id)
			.execute(&self.0)
			.await
			.map_err(|e| AmbulanceLookupError::OtherError(e.into()))?
			.rows_affected() > 0 {
			Ok(())
		} else {
			Err(AmbulanceLookupError::AmbulanceNotTracked)
		}
	}
}

impl SQLTrackingManager {
	pub fn new(pool: PgPool) -> Self { Self(pool) }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::data::{AccountManager, AccountRole, AmbulanceTracker, SettingsManager};
	use crate::sql::sql_account_manager::SqlAccountManager;
	use crate::sql::sql_ambulance_tracker::SQLAmbulanceTracker;
	use crate::sql::sql_settings_manager::SQLSettingsManager;
	use geo_types::Point;
	use std::str::FromStr;

	#[derive(Debug, Clone)]
	struct CloseEnoughDateTime(DateTime<Utc>);
	impl PartialEq for CloseEnoughDateTime {
		fn eq(&self, other: &Self) -> bool {
			(self.0 - other.0).num_milliseconds() < 10
		}
	}

	async fn get_tracking_manager(pool: PgPool) -> Result<(impl TrackingManager,
														   AccountId, AccountId, AccountId, AccountId,
														   PhoneNumber, PhoneNumber, PhoneNumber, PhoneNumber,
														   Ambulance, Ambulance, Uuid), Box<dyn std::error::Error>> {
		let acc = SqlAccountManager::new(pool.clone());
		let (user1, _) = acc.create_site_admin("user1").await?;
		let (user2, _) = acc.create_account(&user1, AccountRole::Admin, "user2").await?;
		let (user3, _) = acc.create_account(&user2, AccountRole::User, "user3").await?;
		let (non_existent_user, _) = acc.create_account(&user2, AccountRole::User, "fake").await?;
		acc.delete_account(&user2, &non_existent_user).await?;

		let settings = SQLSettingsManager::new(pool.clone());
		let account_1_phone_1 = settings.new_phone(user1, "5045550101", "acc1ph1").await?;
		let account_1_phone_2 = settings.new_phone(user1, "5045550102", "acc1ph2").await?;
		let account_2_phone_1 = settings.new_phone(user2, "5045550103", "acc2ph1").await?;
		let account_2_phone_2 = settings.new_phone(user2, "5045550104", "acc2ph2").await?;

		let amb = SQLAmbulanceTracker::new(pool.clone());
		let amb1 = amb.add_ambulance("amb1", Point::new(-90., 30.), Utc::now()).await?;
		let amb2 = amb.add_ambulance("amb2", Point::new(-30., 30.), Utc::now()).await?;

		let tracking = SQLTrackingManager::new(pool.clone());

		Ok((
			tracking,
			user1, user2, user3, non_existent_user,
			account_1_phone_1, account_1_phone_2, account_2_phone_1, account_2_phone_2,
			amb1, amb2, Uuid::from_str("00000000-0000-0000-0000-000000000001").unwrap()
		))
	}

	#[sqlx::test]
	async fn test_errors(pool: PgPool) {
		let (
			tracking_manager,
			account_1_id,
			account_2_id,
			_account_3_id,
			non_existent_account_id,
			_account_1_phone_1,
			_account_1_phone_2,
			_account_2_phone_1,
			_account_2_phone_2,
			ambulance_1,
			_ambulance_2,
			non_existent_ambulance_id,
		) = get_tracking_manager(pool).await.unwrap();

		assert!(matches!(tracking_manager.get_user_tracking(non_existent_account_id).await, Err(UserLookupError::UserNotFound)));
		assert!(matches!(tracking_manager.track_ambulance(non_existent_account_id, non_existent_ambulance_id, "fake", "fake", &[], None).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.track_ambulance(account_1_id, non_existent_ambulance_id, "fake", "fake", &[], None).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.track_ambulance(non_existent_account_id, ambulance_1.id, "fake", "fake", &[], None).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));

		assert!(matches!(tracking_manager.dismiss_eta_alert(non_existent_account_id, non_existent_ambulance_id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.dismiss_eta_alert(account_1_id, non_existent_ambulance_id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.dismiss_eta_alert(non_existent_account_id, ambulance_1.id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));

		assert!(matches!(tracking_manager.stop_tracking_ambulance(non_existent_account_id, non_existent_ambulance_id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.stop_tracking_ambulance(account_1_id, non_existent_ambulance_id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.stop_tracking_ambulance(non_existent_account_id, ambulance_1.id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));

		tracking_manager.track_ambulance(account_1_id, ambulance_1.id, "a", "a", &[], None).await.unwrap();
		assert!(matches!(tracking_manager.dismiss_eta_alert(account_2_id, ambulance_1.id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(matches!(tracking_manager.dismiss_eta_alert(non_existent_account_id, ambulance_1.id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
		assert!(tracking_manager.dismiss_eta_alert(account_1_id, ambulance_1.id).await.is_ok());

		assert!(tracking_manager.stop_tracking_ambulance(account_1_id, ambulance_1.id).await.is_ok());
		assert!(matches!(tracking_manager.stop_tracking_ambulance(account_1_id, ambulance_1.id).await, Err(AmbulanceLookupError::AmbulanceNotTracked)));
	}

	#[sqlx::test]
	async fn test_track_ambulance_visible_in_user_tracking(pool: PgPool) {
		let (
			tracking_manager,
			account_1_id,
			_account_2_id,
			_account_3_id,
			_non_existent_account_id,
			account_1_phone_1,
			_account_1_phone_2,
			_account_2_phone_1,
			_account_2_phone_2,
			ambulance_1,
			_ambulance_2,
			_non_existent_ambulance_id,
		) = get_tracking_manager(pool).await.unwrap();

		tracking_manager
			.track_ambulance(
				account_1_id,
				ambulance_1.id,
				"My Ambulance",
				"high",
				&vec![(account_1_phone_1.phone_id, Duration::from_secs(30))],
				Some(Duration::from_secs(300)),
			)
			.await
			.unwrap();

		let tracking = tracking_manager
			.get_user_tracking(account_1_id)
			.await
			.unwrap();

		assert_eq!(tracking.len(), 1);
		assert_eq!(tracking[0].ambulance.id, ambulance_1.id);
		assert_eq!(tracking[0].ambulance.location, ambulance_1.location);
		assert_eq!(tracking[0].ambulance.name, ambulance_1.name);
		assert_eq!(CloseEnoughDateTime(tracking[0].ambulance.last_updated), CloseEnoughDateTime(ambulance_1.last_updated));
		assert_eq!(tracking[0].user_label, "My Ambulance");
		assert_eq!(tracking[0].urgency, "high");
		assert_eq!(tracking[0].phones_tracking.len(), 1);
		assert_eq!(tracking[0].phones_tracking[0].0.phone_id, account_1_phone_1.phone_id);
		assert_eq!(tracking[0].phones_tracking[0].0.label, account_1_phone_1.label);
		assert_eq!(tracking[0].phones_tracking[0].0.number, account_1_phone_1.number);
		assert_eq!(tracking[0].phones_tracking[0].1, Duration::from_secs(30));
		assert_eq!(tracking[0].user_eta_notify, Some(Duration::from_secs(300)));
	}

	#[sqlx::test]
	async fn test_stop_tracking_removes_ambulance(pool: PgPool) {
		let (
			tracking_manager,
			account_1_id,
			_account_2_id,
			_account_3_id,
			_non_existent_account_id,
			account_1_phone_1,
			_account_1_phone_2,
			_account_2_phone_1,
			_account_2_phone_2,
			ambulance_1,
			_ambulance_2,
			_non_existent_ambulance_id,
		) = get_tracking_manager(pool).await.unwrap();

		tracking_manager
			.track_ambulance(
				account_1_id,
				ambulance_1.id,
				"Temp",
				"medium",
				&vec![(account_1_phone_1.phone_id, Duration::from_secs(10))],
				None,
			)
			.await
			.unwrap();

		tracking_manager
			.stop_tracking_ambulance(account_1_id, ambulance_1.id)
			.await
			.unwrap();

		let tracking = tracking_manager
			.get_user_tracking(account_1_id)
			.await
			.unwrap();

		assert!(tracking.is_empty());
	}

	#[sqlx::test]
	async fn test_dismiss_eta_alert_clears_notification(pool: PgPool) {
		let (
			tracking_manager,
			account_1_id,
			_account_2_id,
			_account_3_id,
			_non_existent_account_id,
			account_1_phone_1,
			_account_1_phone_2,
			_account_2_phone_1,
			_account_2_phone_2,
			ambulance_1,
			_ambulance_2,
			_non_existent_ambulance_id,
		) = get_tracking_manager(pool).await.unwrap();

		tracking_manager
			.track_ambulance(
				account_1_id,
				ambulance_1.id,
				"Alert Test",
				"low",
				&vec![(account_1_phone_1.phone_id, Duration::from_secs(15))],
				Some(Duration::from_secs(120)),
			)
			.await
			.unwrap();

		let tracking = tracking_manager
			.get_user_tracking(account_1_id)
			.await
			.unwrap();

		assert_eq!(tracking.len(), 1);
		assert_eq!(tracking[0].ambulance.id, ambulance_1.id);
		assert_eq!(tracking[0].ambulance.location, ambulance_1.location);
		assert_eq!(tracking[0].ambulance.name, ambulance_1.name);
		assert_eq!(CloseEnoughDateTime(tracking[0].ambulance.last_updated), CloseEnoughDateTime(ambulance_1.last_updated));
		assert_eq!(tracking[0].user_label, "Alert Test");
		assert_eq!(tracking[0].urgency, "low");
		assert_eq!(tracking[0].phones_tracking.len(), 1);
		assert_eq!(tracking[0].phones_tracking[0].0.phone_id, account_1_phone_1.phone_id);
		assert_eq!(tracking[0].phones_tracking[0].0.label, account_1_phone_1.label);
		assert_eq!(tracking[0].phones_tracking[0].0.number, account_1_phone_1.number);
		assert_eq!(tracking[0].phones_tracking[0].1, Duration::from_secs(15));
		assert_eq!(tracking[0].user_eta_notify, Some(Duration::from_secs(120)));

		tracking_manager
			.dismiss_eta_alert(account_1_id, ambulance_1.id)
			.await
			.unwrap();

		let tracking = tracking_manager
			.get_user_tracking(account_1_id)
			.await
			.unwrap();

		assert_eq!(tracking.len(), 1);
		assert_eq!(tracking[0].ambulance.id, ambulance_1.id);
		assert_eq!(tracking[0].ambulance.location, ambulance_1.location);
		assert_eq!(tracking[0].ambulance.name, ambulance_1.name);
		assert_eq!(CloseEnoughDateTime(tracking[0].ambulance.last_updated), CloseEnoughDateTime(ambulance_1.last_updated));
		assert_eq!(tracking[0].user_label, "Alert Test");
		assert_eq!(tracking[0].urgency, "low");
		assert_eq!(tracking[0].phones_tracking.len(), 1);
		assert_eq!(tracking[0].phones_tracking[0].0.phone_id, account_1_phone_1.phone_id);
		assert_eq!(tracking[0].phones_tracking[0].0.label, account_1_phone_1.label);
		assert_eq!(tracking[0].phones_tracking[0].0.number, account_1_phone_1.number);
		assert_eq!(tracking[0].phones_tracking[0].1, Duration::from_secs(15));
		assert_eq!(tracking[0].user_eta_notify, None);
	}

	#[sqlx::test]
	async fn test_multiple_users_ambulances_phones_with_followup_mutations(pool: PgPool) {
		let (
			tracking_manager,
			account_1_id,
			account_2_id,
			_account_3_id,
			_non_existent_account_id,
			account_1_phone_1,
			account_1_phone_2,
			account_2_phone_1,
			account_2_phone_2,
			ambulance_1,
			ambulance_2,
			_non_existent_ambulance_id,
		) = get_tracking_manager(pool).await.unwrap();

		// --- Initial state setup ---

		// Account 1 tracks Ambulance 1 with two phones
		tracking_manager
			.track_ambulance(
				account_1_id,
				ambulance_1.id,
				"A1-Ambulance-1",
				"high",
				&vec![
					(account_1_phone_1.phone_id, Duration::from_secs(30)),
					(account_1_phone_2.phone_id, Duration::from_secs(60)),
				],
				Some(Duration::from_secs(300)),
			)
			.await
			.unwrap();

		// Account 1 tracks Ambulance 2 with one phone
		tracking_manager
			.track_ambulance(
				account_1_id,
				ambulance_2.id,
				"A1-Ambulance-2",
				"medium",
				&vec![(account_1_phone_1.phone_id, Duration::from_secs(45))],
				Some(Duration::from_secs(200)),
			)
			.await
			.unwrap();

		// Account 2 tracks Ambulance 1 with two phones
		tracking_manager
			.track_ambulance(
				account_2_id,
				ambulance_1.id,
				"A2-Ambulance-1",
				"low",
				&vec![
					(account_2_phone_1.phone_id, Duration::from_secs(20)),
					(account_2_phone_2.phone_id, Duration::from_secs(40)),
				],
				None,
			)
			.await
			.unwrap();

		// --- Follow-up mutations ---

		// Account 1 dismisses ETA alert for Ambulance 1
		tracking_manager
			.dismiss_eta_alert(account_1_id, ambulance_1.id)
			.await
			.unwrap();

		let tracking = tracking_manager
			.get_user_tracking(account_1_id)
			.await
			.unwrap();

		assert_eq!(tracking.len(), 2);
		let amb1u1 = tracking.iter().find(|a| a.ambulance.id == ambulance_1.id).unwrap();
		assert_eq!(amb1u1.ambulance.id, ambulance_1.id);
		assert_eq!(amb1u1.ambulance.location, ambulance_1.location);
		assert_eq!(amb1u1.ambulance.name, ambulance_1.name);
		assert_eq!(CloseEnoughDateTime(amb1u1.ambulance.last_updated), CloseEnoughDateTime(ambulance_1.last_updated));
		assert_eq!(amb1u1.user_label, "A1-Ambulance-1");
		assert_eq!(amb1u1.urgency, "high");
		assert_eq!(amb1u1.phones_tracking.len(), 2);
		let u1p1 = amb1u1.phones_tracking.iter().find(|p| p.0.phone_id == account_1_phone_1.phone_id).unwrap();
		assert_eq!(u1p1.0.phone_id, account_1_phone_1.phone_id);
		assert_eq!(u1p1.0.label, account_1_phone_1.label);
		assert_eq!(u1p1.0.number, account_1_phone_1.number);
		assert_eq!(u1p1.1, Duration::from_secs(30));
		let u1p2 = amb1u1.phones_tracking.iter().find(|p| p.0.phone_id == account_1_phone_2.phone_id).unwrap();
		assert_eq!(u1p2.0.phone_id, account_1_phone_2.phone_id);
		assert_eq!(u1p2.0.label, account_1_phone_2.label);
		assert_eq!(u1p2.0.number, account_1_phone_2.number);
		assert_eq!(u1p2.1, Duration::from_secs(60));
		assert_eq!(amb1u1.user_eta_notify, None);

		let amb2u1 = tracking.iter().find(|a| a.ambulance.id == ambulance_2.id).unwrap();
		assert_eq!(amb2u1.ambulance.id, ambulance_2.id);
		assert_eq!(amb2u1.ambulance.location, ambulance_2.location);
		assert_eq!(amb2u1.ambulance.name, ambulance_2.name);
		assert_eq!(CloseEnoughDateTime(amb2u1.ambulance.last_updated), CloseEnoughDateTime(ambulance_2.last_updated));
		assert_eq!(amb2u1.user_label, "A1-Ambulance-2");
		assert_eq!(amb2u1.urgency, "medium");
		assert_eq!(amb2u1.phones_tracking.len(), 1);
		let u1p1 = &amb2u1.phones_tracking[0];
		assert_eq!(u1p1.0.phone_id, account_1_phone_1.phone_id);
		assert_eq!(u1p1.0.label, account_1_phone_1.label);
		assert_eq!(u1p1.0.number, account_1_phone_1.number);
		assert_eq!(u1p1.1, Duration::from_secs(45));
		assert_eq!(amb2u1.user_eta_notify, Some(Duration::from_secs(200)));

		let tracking = tracking_manager
			.get_user_tracking(account_2_id)
			.await
			.unwrap();
		assert_eq!(tracking.len(), 1);
		let amb1u2 = &tracking[0];
		assert_eq!(amb1u2.ambulance.id, ambulance_1.id);
		assert_eq!(amb1u2.ambulance.location, ambulance_1.location);
		assert_eq!(amb1u2.ambulance.name, ambulance_1.name);
		assert_eq!(CloseEnoughDateTime(amb1u2.ambulance.last_updated), CloseEnoughDateTime(ambulance_1.last_updated));
		assert_eq!(amb1u2.user_label, "A2-Ambulance-1");
		assert_eq!(amb1u2.urgency, "low");
		assert_eq!(amb1u2.phones_tracking.len(), 2);
		let u1p1 = amb1u2.phones_tracking.iter().find(|p| p.0.phone_id == account_2_phone_1.phone_id).unwrap();
		assert_eq!(u1p1.0.phone_id, account_2_phone_1.phone_id);
		assert_eq!(u1p1.0.label, account_2_phone_1.label);
		assert_eq!(u1p1.0.number, account_2_phone_1.number);
		assert_eq!(u1p1.1, Duration::from_secs(20));
		let u1p2 = amb1u2.phones_tracking.iter().find(|p| p.0.phone_id == account_2_phone_2.phone_id).unwrap();
		assert_eq!(u1p2.0.phone_id, account_2_phone_2.phone_id);
		assert_eq!(u1p2.0.label, account_2_phone_2.label);
		assert_eq!(u1p2.0.number, account_2_phone_2.number);
		assert_eq!(u1p2.1, Duration::from_secs(40));
		assert_eq!(amb1u2.user_eta_notify, None);

		// --- Verification via get_user_tracking ---

		tracking_manager.stop_tracking_ambulance(account_1_id, ambulance_1.id).await.unwrap();
		tracking_manager.stop_tracking_ambulance(account_2_id, ambulance_1.id).await.unwrap();

		let tracking = tracking_manager
			.get_user_tracking(account_2_id)
			.await
			.unwrap();
		assert!(tracking.is_empty());

		let tracking = tracking_manager
			.get_user_tracking(account_1_id)
			.await
			.unwrap();
		assert_eq!(tracking.len(), 1);
		let amb2u1 = &tracking[0];
		assert_eq!(amb2u1.ambulance.id, ambulance_2.id);
		assert_eq!(amb2u1.ambulance.location, ambulance_2.location);
		assert_eq!(amb2u1.ambulance.name, ambulance_2.name);
		assert_eq!(CloseEnoughDateTime(amb2u1.ambulance.last_updated), CloseEnoughDateTime(ambulance_2.last_updated));
		assert_eq!(amb2u1.user_label, "A1-Ambulance-2");
		assert_eq!(amb2u1.urgency, "medium");
		assert_eq!(amb2u1.phones_tracking.len(), 1);
		let u1p1 = &amb2u1.phones_tracking[0];
		assert_eq!(u1p1.0.phone_id, account_1_phone_1.phone_id);
		assert_eq!(u1p1.0.label, account_1_phone_1.label);
		assert_eq!(u1p1.0.number, account_1_phone_1.number);
		assert_eq!(u1p1.1, Duration::from_secs(45));
		assert_eq!(amb2u1.user_eta_notify, Some(Duration::from_secs(200)));
	}
}