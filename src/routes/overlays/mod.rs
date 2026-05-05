use crate::SharedAppState;
use crate::routes::overlays;
use axum::Router;
use axum::routing::get;
use tower_http::services::ServeDir;

pub mod teamoverlay;
pub fn router() -> Router<SharedAppState> {
    let assets_dir = "assets";
    let static_files_service = ServeDir::new(assets_dir).append_index_html_on_directories(true);
    Router::new().fallback_service(static_files_service)
}
