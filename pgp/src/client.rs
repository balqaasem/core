use hyper::{client::HttpConnector, Client};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use std::sync::Arc;

pub(crate) fn build() -> Arc<Client<HttpsConnector<HttpConnector>>> {
    Arc::new(
        Client::builder().build::<_, hyper::Body>(
            HttpsConnectorBuilder::new()
                .with_native_roots()
                .https_or_http()
                .enable_http1()
                .build(),
        ),
    )
}
