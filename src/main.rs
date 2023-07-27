mod env;
use axum::{
    extract::{Path, State},
    headers::{authorization::Bearer, Authorization},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Router,
    TypedHeader,
};
use env::Env;
use evmap::{ReadHandleFactory, WriteHandle};
use parking_lot::Mutex;
use rate_limiter_lib::{InternalValue, KeyType, LimitType, Store};
use std::{error::Error, net::SocketAddr, sync::Arc};

pub struct AppState {
    pub store_reader: ReadHandleFactory<KeyType, InternalValue>,
    pub store_writer: Arc<Mutex<WriteHandle<KeyType, InternalValue>>>,
    pub ttl: i64,
}

pub fn routes(app_state: Arc<AppState>) -> Router {
    Router::new()
        .route("/vault", post(add_vault_item))
        .route("/vault/items", get(get_vault_items))
        .route("/vault/:id", put(put_vault_items))
        .with_state(app_state)
}
const POST_RATE_LIMIT: LimitType = 3;
const PUT_RATE_LIMIT: LimitType = 60;
const GET_RATE_LIMIT: LimitType = 1200;

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn Error>> {
    let _ = dotenv::dotenv().ok();
    let env = envy::from_env::<Env>()?;
    env_logger::init();
    let (read_handle, write_handle, timer_handler) = Store::init().await;
    let app_state = Arc::new(AppState {
        store_reader: read_handle,
        store_writer: write_handle,
        ttl: env.ttl,
    });

    let app = routes(app_state);
    let addr = SocketAddr::from(([127, 0, 0, 1], env.server_port as u16));
    log::info!("listening on {}", addr);
    let server_future = axum::Server::bind(&addr).serve(app.into_make_service());
    let _ = tokio::join!(server_future, timer_handler);
    Ok(())
}

async fn get_vault_items(
    TypedHeader(key): TypedHeader<Authorization<Bearer>>,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    if let Err(e) = Store::inc_below_limit(
        &app_state.store_writer,
        &app_state.store_reader.handle(),
        format!("get_vault_items_{}",key.token()),
        GET_RATE_LIMIT,
        app_state.ttl,
    ) {
        return (StatusCode::TOO_MANY_REQUESTS, e.to_string()).into_response();
    }
    (StatusCode::OK, "Returned vault items").into_response()
}

pub async fn add_vault_item(
    TypedHeader(key): TypedHeader<Authorization<Bearer>>,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    if let Err(e) = Store::inc_below_limit(
        &app_state.store_writer,
        &app_state.store_reader.handle(),
        format!("add_vault_item_{}",key.token()),
        POST_RATE_LIMIT,
        app_state.ttl,
    ) {
        return (StatusCode::TOO_MANY_REQUESTS, e.to_string()).into_response();
    }
    (StatusCode::OK, "Vault key added").into_response()
}

pub async fn put_vault_items(
    TypedHeader(key): TypedHeader<Authorization<Bearer>>,
    Path(_id): Path<String>,
    State(app_state): State<Arc<AppState>>,
) -> Response {
    if let Err(e) = Store::inc_below_limit(
        &app_state.store_writer,
        &app_state.store_reader.handle(),
        format!("put_vault_items_{}",key.token()),
        PUT_RATE_LIMIT,
        app_state.ttl,
    ) {
        return (StatusCode::TOO_MANY_REQUESTS, e.to_string()).into_response();
    }
    (StatusCode::OK, "Added vault items").into_response()
}
