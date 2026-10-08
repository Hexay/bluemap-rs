//! The served maps (`RoutingRequestHandler`'s `maps/<id>/(.*)` routes), shared between the running app and its
//! owner, so a map loaded after startup (a `-u` retry once its world appears) gets its live routes. Requests take a
//! snapshot (an uncontended read lock and an `Arc` clone); changes copy the list, which only happens on map loads.

use std::sync::{Arc, PoisonError, RwLock};

use crate::WebError;
use crate::live::LiveMap;
use crate::map_handler::MapRoute;

type Routes = Arc<Vec<(String, MapRoute)>>;

#[derive(Clone, Default)]
pub struct MapRegistry(Arc<RwLock<Routes>>);

impl MapRegistry {
    /// Serves `maps/<id>/…` from `route`; replaces an earlier route of the same id.
    pub fn insert(&self, id: impl Into<String>, route: MapRoute) -> Result<(), WebError> {
        let id = id.into();
        if id.is_empty() || id.contains('/') {
            return Err(WebError::MapId(id));
        }
        self.update(|routes| {
            routes.retain(|(i, _)| *i != id);
            routes.push((id, route));
        });
        Ok(())
    }

    /// Gives the served map `id` live routes (`live/markers.json`, `live/players.json`, `live/sse` as `live`
    /// enables them). False if `id` is not served.
    pub fn set_live(&self, id: &str, live: Arc<LiveMap>) -> bool {
        let mut found = false;
        self.update(|routes| {
            if let Some((_, route)) = routes.iter_mut().find(|(i, _)| i == id) {
                route.live = Some(live);
                found = true;
            }
        });
        found
    }

    pub(crate) fn snapshot(&self) -> Routes {
        self.0.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn update(&self, change: impl FnOnce(&mut Vec<(String, MapRoute)>)) {
        let mut routes = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let mut next = routes.as_ref().clone();
        change(&mut next);
        *routes = Arc::new(next);
    }
}
