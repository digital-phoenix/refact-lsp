use std::future::Future;
use std::pin::Pin;
use tracing::{info, error};
use axum::Extension;
use axum::http::{Method, Uri};
use hyper::{Body, Response};
use crate::custom_error::ScratchError;
use crate::global_context::SharedGlobalContext;
use crate::telemetry::telemetry_structs;

pub async fn telemetry_wrapper(func: impl Fn(Extension<SharedGlobalContext>, hyper::body::Bytes)
                                        -> Pin<Box<dyn Future<Output=Result<Response<Body>, ScratchError>> + Send>> ,
                               path: Uri,
                               method: Method,
                               ex: Extension<SharedGlobalContext>,
                               body_bytes: hyper::body::Bytes) -> Result<Response<Body>, ScratchError> {
    let t0 = std::time::Instant::now();
    let result = Box::pin(func(ex.clone(), body_bytes)).await;
    if let Err(e) = result {
        if !e.telemetry_skip {
            let tele_storage = &ex.read().await.telemetry;
            let mut tele_storage_locked = tele_storage.write().unwrap();
            tele_storage_locked.tele_net.push(telemetry_structs::TelemetryNetwork::new(
                path.path().to_string(),
                format!("{}", method),
                false,
                format!("{}", e.message),
            ));
        }
        error!("{} returning \"{}\"", path, e.status_code);
        return Ok(e.to_response());
    }
    info!("{} completed in {:?}", path, t0.elapsed());
    return Ok(result.unwrap());
}

#[macro_export]
macro_rules! telemetry_post {
    (
    $name:ident
     ) => {
           post(|path, method, ex, body_bytes| async {
               let tmp = |ex: Extension<SharedGlobalContext>,
                          body_bytes: hyper::body::Bytes|
               -> Pin<Box<dyn Future<Output=Result<Response<Body>, ScratchError>> + Send>> {
                    Box::pin($name(ex, body_bytes))
                };
               telemetry_wrapper(tmp, path, method, ex, body_bytes).await
           })
        };
    }

#[macro_export]
macro_rules! telemetry_get {
    (
    $name:ident
     ) => {
           get(|path, method, ex, body_bytes| async {
               let tmp = |ex: Extension<SharedGlobalContext>,
                          body_bytes: hyper::body::Bytes|
               -> Pin<Box<dyn Future<Output=Result<Response<Body>, ScratchError>> + Send>> {
                    Box::pin($name(ex, body_bytes))
                };
               telemetry_wrapper(tmp, path, method, ex, body_bytes).await
           })
        };
    }