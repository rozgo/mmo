
#[derive(Serialize, Deserialize)]
pub struct Config {
    pub aws: Aws,
}

#[derive(Serialize, Deserialize)]
pub struct Aws {
    pub access_key_id: String,
    pub secret_access_key: String,
}
