use kalshi_sdk_rs::{Client, Credentials, Environment, Result};

pub fn authenticated_demo() -> Result<Client> {
    Client::builder(Environment::Demo)
        .credentials(Credentials::from_env()?)
        .build()
}
