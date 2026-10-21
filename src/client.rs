//! The calls ready made over [`reqwest`], behind the `reqwest` feature.
//!
//! [`Client`] is [`Api`] with an HTTP client attached: each method renders the request, sends it
//! and decodes the answer. A merchant that already runs its own HTTP client and its own transport
//! rules should use [`Api`] directly and keep this feature off, because this module is the only
//! place the SDK opens a connection.
//!
//! The default [`reqwest::Client`] ([`default_http`]) has a connect timeout of [`CONNECT_TIMEOUT`],
//! a total timeout of [`TOTAL_TIMEOUT`] and follows no redirect. The gateway never redirects a
//! `/v1` call, and a redirect would carry the bearer key to wherever it points. A merchant that
//! passes its own client to [`Client::with_http`] owns these settings.
//!
//! ```no_run
//! # async fn run() -> Result<(), kenari_pay_sdk::client::ClientError> {
//! use kenari_pay_sdk::client::Client;
//! use kenari_pay_sdk::wire::Family;
//!
//! let client = Client::new("https://pay-sandbox.kenari.id", "kp_test_example")?;
//! let address = client.create_address("usr_1", Family::Evm).await?;
//! // Run `kenari_pay_sdk::address::verify` on it before you show it.
//! # let _ = address;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use crate::api::{Api, ApiError, HttpRequest, Method};
use crate::wire::{
    Address, CustomerDeposits, Deposit, DepositsPage, Family, Key, Keys, Networks, PoolFile,
    PoolImported, SweepsPage,
};

/// How long the default client waits to connect.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the default client waits for a whole call, connecting and reading the answer included.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

/// Why a call of [`Client`] failed. The `Display` text is fixed, so nothing of the gateway or of
/// the transport reaches a merchant's customer through `to_string()`. The detail is in `Debug`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// The gateway answered, and the answer is not what was asked for.
    Api(ApiError),
    /// The request did not complete: connecting, TLS, a timeout, a reset, or a request that could
    /// not be built. The text is the transport's, with the URL taken out, for a log.
    Transport(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Api(error) => error.fmt(f),
            ClientError::Transport(_) => f.write_str("request to the gateway failed"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Api(error) => Some(error),
            ClientError::Transport(_) => None,
        }
    }
}

impl From<ApiError> for ClientError {
    fn from(error: ApiError) -> ClientError {
        ClientError::Api(error)
    }
}

fn transport(error: reqwest::Error) -> ClientError {
    ClientError::Transport(error.without_url().to_string())
}

/// The `reqwest::Client` that [`Client::new`] uses: [`CONNECT_TIMEOUT`], [`TOTAL_TIMEOUT`], no
/// redirects and a user agent that names the SDK. Build the client of a test the same way, so the
/// test runs the code the merchant runs.
pub fn default_http() -> Result<reqwest::Client, ClientError> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("kenari-pay-sdk/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(transport)
}

/// The merchant API with an HTTP client attached.
///
/// It is cheap to clone, because [`reqwest::Client`] shares its connection pool between clones.
/// The `Debug` form hides the API key.
#[derive(Clone, Debug)]
pub struct Client {
    api: Api,
    http: reqwest::Client,
}

impl Client {
    /// A client for `base_url` and the merchant's API key over [`default_http`].
    pub fn new(base_url: &str, api_key: &str) -> Result<Client, ClientError> {
        Ok(Client::with_http(base_url, api_key, default_http()?))
    }

    /// A client over an HTTP client the merchant built, with its own timeouts, proxy and TLS
    /// settings. Set a timeout on it, because without one a call can wait forever. Turn redirects
    /// off too, so the bearer key is only ever sent to the base URL.
    pub fn with_http(base_url: &str, api_key: &str, http: reqwest::Client) -> Client {
        Client {
            api: Api::new(base_url, api_key),
            http,
        }
    }

    /// The request builder underneath, for the calls this client has no method for.
    pub fn api(&self) -> &Api {
        &self.api
    }

    /// Send a request that [`Api`] rendered and return the status and the body, whatever the
    /// status is.
    pub async fn send(&self, request: HttpRequest) -> Result<(u16, Vec<u8>), ClientError> {
        let mut builder = match request.method {
            Method::Get => self.http.get(&request.url),
            Method::Post => self.http.post(&request.url),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(transport)?;
        let status = response.status().as_u16();
        let body = response.bytes().await.map_err(transport)?;
        Ok((status, body.to_vec()))
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        request: HttpRequest,
    ) -> Result<T, ClientError> {
        let (status, body) = self.send(request).await?;
        Ok(Api::decode(status, &body)?)
    }

    /// `GET /v1/networks`.
    pub async fn networks(&self) -> Result<Networks, ClientError> {
        self.call(self.api.networks()).await
    }

    /// `POST /v1/addresses`. Run [`crate::address::verify`] on the answer before showing it.
    pub async fn create_address(
        &self,
        customer_ref: &str,
        family: Family,
    ) -> Result<Address, ClientError> {
        self.call(self.api.create_address(customer_ref, family))
            .await
    }

    /// `POST /v1/addresses/{id}/reissue`. Run [`crate::address::verify`] on the answer. An empty,
    /// `.` or `..` id is refused before anything is sent, as [`ApiError::Argument`] in
    /// [`ClientError::Api`].
    pub async fn reissue(&self, address_id: &str) -> Result<Address, ClientError> {
        self.call(self.api.reissue(address_id)?).await
    }

    /// `GET /v1/deposits/{id_or_key}`, with a 404 as `Ok(None)`. An empty, `.` or `..` value is
    /// refused before anything is sent, as [`ApiError::Argument`] in [`ClientError::Api`].
    pub async fn deposit(&self, id_or_key: &str) -> Result<Option<Deposit>, ClientError> {
        let (status, body) = self.send(self.api.deposit(id_or_key)?).await?;
        Ok(Api::decode_optional(status, &body)?)
    }

    /// `GET /v1/deposits?after_seq=&limit=`. See [`crate::feed`] for the cursor rule.
    pub async fn deposits(&self, after_seq: i64, limit: u32) -> Result<DepositsPage, ClientError> {
        self.call(self.api.deposits(after_seq, limit)).await
    }

    /// `GET /v1/customers/{customer_ref}/deposits?limit=`. An empty, `.` or `..` customer ref is
    /// refused before anything is sent, as [`ApiError::Argument`] in [`ClientError::Api`].
    pub async fn customer_deposits(
        &self,
        customer_ref: &str,
        limit: u32,
    ) -> Result<CustomerDeposits, ClientError> {
        self.call(self.api.customer_deposits(customer_ref, limit)?)
            .await
    }

    /// `GET /v1/sweeps?after_seq=&limit=`. See [`crate::feed`] for the cursor rule.
    pub async fn sweeps(&self, after_seq: i64, limit: u32) -> Result<SweepsPage, ClientError> {
        self.call(self.api.sweeps(after_seq, limit)).await
    }

    /// `GET /v1/keys`, for a merchant on pool addresses.
    pub async fn keys(&self) -> Result<Keys, ClientError> {
        self.call(self.api.keys()).await
    }

    /// `POST /v1/keys`, for a merchant on pool addresses.
    pub async fn register_key(
        &self,
        key_epoch: u32,
        attest_pubkey_hex: &str,
    ) -> Result<Key, ClientError> {
        self.call(self.api.register_key(key_epoch, attest_pubkey_hex))
            .await
    }

    /// `POST /v1/pool`, for a merchant on pool addresses.
    pub async fn import_pool(&self, file: &PoolFile) -> Result<PoolImported, ClientError> {
        self.call(self.api.import_pool(file)).await
    }
}
