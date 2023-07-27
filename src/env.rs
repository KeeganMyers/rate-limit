use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Env {
    pub server_port: usize,
    pub ttl: i64,
}
