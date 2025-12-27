use std::error::Error;
use std::time::Duration;
use geo_types::Point;
use sqlx::types::Uuid;
use crate::eta::eta_finder::EtaFinder;

pub struct MapboxEta(String, reqwest::Client);

#[inline(always)]
fn build_request_url(from: Point, to: Point, api_key: &str) -> String {
	format!("https://api.mapbox.com/directions/v5/mapbox/driving-traffic/{},{};{},{}?include=hov2,hov3,hot&overview=false&access_token={}",
			from.x(),
			from.y(),
			to.x(),
			to.y(),
			api_key
	)
}

#[derive(Debug, thiserror::Error)]
enum MapboxError {
	#[error("No routes returned")]
	NoRoutes
}

#[derive(serde::Deserialize, Debug)]
struct Route {
	duration: f64
}
#[derive(serde::Deserialize, Debug)]
struct MapboxResponse {
	routes: Vec<Route>
}

#[async_trait::async_trait]
impl EtaFinder for MapboxEta {
	async fn calculate_eta(&self, _ambulance_id: Uuid, from: Point, to: Point) -> Result<Duration, Box<dyn Error>> {
		let resp: MapboxResponse = serde_json::from_slice(&*self.1.get(
			build_request_url(from, to, &*self.0)
		).send().await?.bytes().await?)?;

		Ok(Duration::from_secs_f64(resp.routes.first().ok_or(MapboxError::NoRoutes)?.duration))
	}
}
impl MapboxEta {
	pub fn new(api_key: String) -> Self { Self(api_key, reqwest::Client::new()) }
}