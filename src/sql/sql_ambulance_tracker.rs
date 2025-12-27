use crate::data::{Ambulance, AmbulanceTracker, AmbulanceTrackerError};
use geo_types::{Geometry, Point};
use geozero::wkb;
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::types::Uuid;
use sqlx::PgPool;
use std::error::Error;
use std::time::Duration;

pub struct SQLAmbulanceTracker(PgPool);

#[async_trait::async_trait]
impl AmbulanceTracker for SQLAmbulanceTracker {
	async fn add_ambulance(&self, name: &str, location: Point, fetched: DateTime<Utc>) -> Result<Ambulance, Box<dyn Error>> {
		let (id,): (Uuid,) =
			sqlx::query_as("INSERT INTO ambulances(ambulance_name, location, last_update) VALUES ($1, $2, $3) RETURNING ambulance_id;")
				.bind(name)
				.bind(wkb::Encode::<Geometry>(location.clone().into()))
				.bind(fetched)
				.fetch_one(&self.0)
				.await?;

		Ok(Ambulance {
			id,
			name: name.to_string(),
			location,
			last_updated: fetched
		})
	}

	async fn update_ambulance(&self, id: Uuid, location: Point, fetched: DateTime<Utc>) -> Result<(), AmbulanceTrackerError> {
		match
			sqlx::query_as::<_, (i32,)>("WITH updated AS (UPDATE ambulances SET location=$2, last_update=$3 WHERE ambulance_id=$1 AND last_update<$3 RETURNING 1) SELECT CASE WHEN EXISTS (SELECT 1 FROM ambulances WHERE ambulance_id=$1) THEN 1 ELSE 0 END;")
				.bind(id)
				.bind(wkb::Encode::<Geometry>(location.into()))
				.bind(fetched)
				.fetch_one(&self.0)
				.await
				.map_err(|e| AmbulanceTrackerError::Other(e.into()))?
				.0 {
			1 => Ok(()),
			0 => Err(AmbulanceTrackerError::AmbulanceNotFound),
			_ => panic!("invalid sql")
		}
	}

	async fn get_recently_updated(&self, last_updated: Duration) -> Result<Vec<Ambulance>, Box<dyn Error>> {
		let ambulances: Vec<(Uuid, Option<String>, wkb::Decode<Geometry>, DateTime<Utc>)> =
			sqlx::query_as("SELECT ambulance_id, ambulance_name, location, last_update FROM ambulances WHERE last_update>$1;")
				.bind(Utc::now() - last_updated)
				.fetch_all(&self.0)
				.await?;

		Ok(ambulances.into_iter().map(|(id, name, location, last_updated)| Ambulance {
			id,
			name: name.unwrap_or(id.to_string()),
			// not null column
			location: location.geometry.unwrap().try_into().unwrap(),
			last_updated
		}).collect())
	}

	async fn get_ambulance(&self, id: Uuid) -> Result<Option<Ambulance>, Box<dyn Error>> {
		let ambulance: Option<(Uuid, Option<String>, wkb::Decode<Geometry>, DateTime<Utc>)> =
			sqlx::query_as("SELECT ambulance_id, ambulance_name, location, last_update FROM ambulances WHERE ambulance_id=$1")
				.bind(id)
				.fetch_optional(&self.0)
				.await?;

		Ok(ambulance.map(|(id, name, location, last_updated)| Ambulance {
			id,
			name: name.unwrap_or(id.to_string()),
			// not null column
			location: location.geometry.unwrap().try_into().unwrap(),
			last_updated
		}))
	}
}

impl SQLAmbulanceTracker {
	/// Creates a new AmbulanceTracker using the specified connection as the backend.
	/// It is expected that the migrations file has been executed already.
	pub fn new(pool: PgPool) -> Self {
		Self(pool)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use geo_types::Point;
	use sqlx::types::chrono::Utc;
	use std::str::FromStr;

	fn get_tracker(pool: PgPool) -> SQLAmbulanceTracker {
		SQLAmbulanceTracker::new(pool)
	}

	#[sqlx::test]
	async fn test_add_ambulance(pg_pool: PgPool) {
		let tracker = get_tracker(pg_pool);

		// Test Case 1: Add a new ambulance
		let name = "Ambulance 1";
		let location: Point = Point::new(0.0, 0.0).into();
		let fetched = Utc::now();
		let ambulance = tracker.add_ambulance(name, location.clone(), fetched).await.unwrap();
		assert_eq!(ambulance.name, name);
		assert_eq!(ambulance.location, location);

		// Test Case 2: Add multiple ambulances and ensure correct assignment of IDs
		let name1 = "Ambulance 2";
		let location1 = Point::new(1.0, 1.0).into();
		let fetched1 = Utc::now();
		let ambulance1 = tracker.add_ambulance(name1, location1, fetched1).await.unwrap();

		let name2 = "Ambulance 3";
		let location2 = Point::new(2.0, 2.0).into();
		let fetched2 = Utc::now();
		let ambulance2 = tracker.add_ambulance(name2, location2, fetched2).await.unwrap();

		assert_ne!(ambulance1.id, ambulance2.id);

		// Test Case 3: Add an ambulance with the same name as an existing ambulance
		let name = "Ambulance 1";
		let location = Point::new(3.0, 3.0).into();
		let fetched = Utc::now();
		let ambulance = tracker.add_ambulance(name, location, fetched).await.unwrap();
		assert_eq!(ambulance.name, name);
	}

	#[sqlx::test]
	async fn test_update_ambulance(pg_pool: PgPool) {
		let tracker = get_tracker(pg_pool);

		// Add an ambulance to test updates
		let name = "Ambulance 1";
		let location = Point::new(0.0, 0.0).into();
		let fetched = Utc::now();
		let ambulance = tracker.add_ambulance(name, location, fetched).await.unwrap();

		// Test Case 4: Update ambulance location with valid fetched time (after initial)
		let new_location: Point = Point::new(2.0, 2.0).into();
		let new_fetched = Utc::now() + Duration::from_secs(10);
		tracker.update_ambulance(ambulance.id, new_location.clone(), new_fetched).await.unwrap();

		// Verify the update
		let updated_ambulance = tracker.get_ambulance(ambulance.id).await.unwrap().unwrap();
		assert_eq!(updated_ambulance.location, new_location);

		// Test Case 5: Update with same location but valid fetched time (ensure it's updated)
		let same_location: Point = Point::new(2.0, 2.0).into();
		let same_fetched = new_fetched + Duration::from_secs(5);
		tracker.update_ambulance(ambulance.id, same_location.clone(), same_fetched).await.unwrap();

		let updated_ambulance = tracker.get_ambulance(ambulance.id).await.unwrap().unwrap();
		assert_eq!(updated_ambulance.location, same_location);

		// Test Case 6: Attempt to update ambulance location with invalid fetched time (before previous update time)
		let new_location = Point::new(3.0, 3.0).into();
		let old_fetched = Utc::now() - Duration::from_secs(10);
		let result = tracker.update_ambulance(ambulance.id, new_location, old_fetched).await;
		assert!(result.is_ok());  // No error, no update, but should not actually update

		let updated_ambulance = tracker.get_ambulance(ambulance.id).await.unwrap().unwrap();
		assert_eq!(updated_ambulance.location, same_location);

		// Test Case 7: Update a non-existing ambulance
		let invalid_id = Uuid::from_str("22200000-0000-0000-0000-000000000001").unwrap();
		let new_location = Point::new(4.0, 4.0).into();
		let result = tracker.update_ambulance(invalid_id, new_location, Utc::now()).await;
		assert!(matches!(result, Err(AmbulanceTrackerError::AmbulanceNotFound)));
	}

	#[derive(PartialEq, Debug, Clone)]
	struct SortAmb(Uuid, String, f64, f64, CloseEnoughDateTime);
	#[derive(Debug, Clone)]
	struct CloseEnoughDateTime(DateTime<Utc>);
	impl PartialEq for CloseEnoughDateTime {
		fn eq(&self, other: &Self) -> bool {
			(self.0 - other.0).num_milliseconds() < 10
		}
	}
	impl From<Ambulance> for SortAmb {
		fn from(value: Ambulance) -> Self {
			let pt: Point = value.location.try_into().unwrap();
			Self(value.id, value.name, pt.0.x, pt.0.y, CloseEnoughDateTime(value.last_updated))
		}
	}

	#[sqlx::test]
	async fn test_get_recently_updated(pg_pool: PgPool) {
		let tracker = get_tracker(pg_pool);

		// Add some ambulances for testing
		let name = "Ambulance 1";
		let location = Point::new(0.0, 0.0).into();
		let fetched = Utc::now() - Duration::from_secs(65);
		let a1 = SortAmb::from(tracker.add_ambulance(name, location, fetched).await.unwrap());

		let name = "Ambulance 2";
		let location = Point::new(1.0, 1.0).into();
		let fetched = Utc::now();
		let a2 = SortAmb::from(tracker.add_ambulance(name, location, fetched).await.unwrap());

		let mut inserted_ambulances = vec![a1, a2.clone()];
		inserted_ambulances.sort_by_key(|a| a.0);

		let last_updated = Duration::from_secs(120);
		let mut ambulances: Vec<_> = tracker.get_recently_updated(last_updated).await.unwrap().into_iter().map(SortAmb::from).collect();
		ambulances.sort_by_key(|a| a.0);
		assert_eq!(ambulances, inserted_ambulances);

		let last_updated = Duration::from_secs(0);
		let ambulances = tracker.get_recently_updated(last_updated).await.unwrap();
		assert!(ambulances.is_empty());

		let last_updated = Duration::from_secs(60);
		let ambulances: Vec<_> = tracker.get_recently_updated(last_updated).await.unwrap().into_iter().map(SortAmb::from).collect();
		assert_eq!(ambulances, vec![a2]);
	}

	#[sqlx::test]
	async fn test_get_ambulance(pg_pool: PgPool) {
		let tracker = get_tracker(pg_pool);

		// Add ambulance to test
		let name = "Ambulance 1";
		let location = Point::new(0.0, 0.0).into();
		let fetched = Utc::now();
		let ambulance = tracker.add_ambulance(name, location, fetched).await.unwrap();

		// Test Case 11: Get an existing ambulance
		let retrieved = tracker.get_ambulance(ambulance.id).await.unwrap().unwrap();
		assert_eq!(retrieved.id, ambulance.id);
		assert_eq!(retrieved.name, ambulance.name);

		// Test Case 12: Get a non-existing ambulance
		let invalid_id = Uuid::from_str("20000000-0000-0000-0000-000000000001").unwrap();
		let retrieved = tracker.get_ambulance(invalid_id).await.unwrap();
		assert!(retrieved.is_none());

		// Test Case 13: Ensure consistency after multiple updates
		let new_location: Point = Point::new(2.0, 2.0).into();
		let fetched = Utc::now() + Duration::from_secs(10);
		tracker.update_ambulance(ambulance.id, new_location.clone(), fetched).await.unwrap();

		let updated_ambulance = tracker.get_ambulance(ambulance.id).await.unwrap().unwrap();
		assert_eq!(updated_ambulance.location, new_location);
	}
}