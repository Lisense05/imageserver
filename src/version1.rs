use actix_web::error::{ErrorInternalServerError, ErrorTooManyRequests, ErrorUnauthorized};
use actix_web::http::header::{CacheControl, CacheDirective, ContentType};
use actix_web::{error::ErrorBadRequest, get, web, Error, HttpRequest, HttpResponse};

use actix_multipart::Multipart;
use futures_util::stream::StreamExt as _;

use std::fs;
use std::io::Write;
use std::time::{Duration, Instant};

use uuid::Uuid;

use serde::{Deserialize, Serialize};

use crate::{AppState, Config, RateLimitState};

#[derive(Deserialize)]
struct Url {
    url: String,
}

#[get("/embed")]
// We fetch the image ourself so that we don't risk accidentally revealing our users IP
async fn embed_external(
    url: web::Query<Url>,
    config: web::Data<Config>,
) -> Result<HttpResponse, Error> {
    let url = &url.url;
    if url.contains(&config.domain) {
        return Err(ErrorBadRequest(
            "Can't try to use local images as external.",
        ));
    }

    let res = reqwest::get(url).await;
    if res.is_err() {
        println!("Failed to use url: {}", url);
        return Err(ErrorBadRequest("The specified website failed to respond."));
    }
    let res = res.unwrap();
    // Early return if the status isn't a success, usually means that the target website doesn't exist
    if !res.status().is_success() {
        return Err(ErrorBadRequest(format!(
            "Target website returned status code {}.",
            res.status()
        )));
    }

    let data = res.bytes().await.unwrap().to_vec();

    if !infer::is_image(&data) {
        return Err(ErrorBadRequest(
            "The target website didn't return an image.",
        ));
    }

    let mut builder = HttpResponse::Ok();
    builder.insert_header(CacheControl(vec![CacheDirective::MaxAge(86400u32)]));
    builder.content_type(ContentType::png());

    Ok(builder.body(data))
}

#[derive(Serialize)]
struct ImageStruct {
    link: String,
}

#[derive(Serialize)]
struct ReturnData {
    data: ImageStruct,
}

#[derive(Serialize)]
struct AudioReturnData {
    url: String,
}

fn check_rate_limit(
    request: &HttpRequest,
    config: &Config,
    app_state: &AppState,
) -> Result<(), Error> {
    let rate_limit_key = request
        .connection_info()
        .realip_remote_addr()
        .map(str::to_string)
        .or_else(|| request.peer_addr().map(|addr| addr.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let now = Instant::now();
    let window = Duration::from_secs(config.rate_limit_window_seconds);

    let mut limiter_map = app_state
        .upload_rate_limit_map
        .lock()
        .map_err(|_| ErrorInternalServerError("Failed to access rate limiter."))?;

    let limiter_entry = limiter_map
        .entry(rate_limit_key)
        .or_insert_with(|| RateLimitState {
            request_count: 0,
            window_started_at: now,
        });

    if now.duration_since(limiter_entry.window_started_at) >= window {
        limiter_entry.request_count = 0;
        limiter_entry.window_started_at = now;
    }

    if limiter_entry.request_count >= config.rate_limit_max_requests {
        return Err(ErrorTooManyRequests("Too many upload requests."));
    }

    limiter_entry.request_count += 1;

    Ok(())
}

fn check_authorization_header(request: &HttpRequest, config: &Config) -> Result<(), Error> {
    let authorization_header = request
        .headers()
        .get("Authorization")
        .ok_or_else(|| ErrorUnauthorized("Missing Authorization header."))?
        .to_str()
        .map_err(|_| ErrorUnauthorized("Invalid Authorization header."))?;

    if authorization_header.trim() != config.api_key {
        return Err(ErrorUnauthorized("Invalid Authorization header."));
    }

    Ok(())
}

async fn read_multipart_binary(mut payload: Multipart) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();

    while let Some(item) = payload.next().await {
        let mut field = item?;
        while let Some(chunk) = field.next().await {
            data.extend_from_slice(&chunk?);
        }
    }

    Ok(data)
}

pub async fn upload_image(
    request: HttpRequest,
    payload: Multipart,
    config: web::Data<Config>,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    check_rate_limit(&request, &config, &app_state)?;
    check_authorization_header(&request, &config)?;

    let data = read_multipart_binary(payload).await?;

    if !infer::is_image(&data) {
        return Err(ErrorBadRequest("The provided data wasn't an image."));
    }

    let unique_signature = Uuid::new_v4();
    let kind = infer::get(&data).unwrap();
    let image_url = format!("{}.{}", unique_signature, kind.extension());

    // This shouldn't ever error, but if it does it will unwrap into the handler
    match fs::File::create(format!("./images/{}", image_url)) {
        Ok(mut file) => {
            file.write_all(&data).unwrap();
            let return_data = serde_json::to_string(&ReturnData {
                data: ImageStruct {
                    link: format!(
                        "{}://{}/v1/image/{}",
                        config.protocol, config.domain, image_url
                    ),
                },
            })
            .unwrap();

            Ok(HttpResponse::Ok()
                .content_type(ContentType::png())
                .body(return_data))
        }
        Err(_) => Err(ErrorInternalServerError("Server failed to make file")),
    }
}

pub async fn fetch_image(image_name: web::Path<String>) -> Result<HttpResponse, Error> {
    let image = fs::read(format!("./images/{}", image_name))?;
    Ok(HttpResponse::Ok()
        .content_type(ContentType::png())
        .body(image))
}

// TODO: Turn this stuff into a trait to de-duplicate
pub async fn upload_audio(
    request: HttpRequest,
    payload: Multipart,
    config: web::Data<Config>,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    check_rate_limit(&request, &config, &app_state)?;
    check_authorization_header(&request, &config)?;

    let data = read_multipart_binary(payload).await?;

    let kind = infer::get(&data).unwrap();
    if kind.mime_type() != "video/webm" {
        return Err(ErrorBadRequest("The provided data wasn't an audio format."));
    }

    let unique_signature = Uuid::new_v4();
    let audio_url = format!("{}.{}", unique_signature, "ogg");

    // This shouldn't ever error, but if it does it will unwrap into the handler
    match fs::File::create(format!("./audio/{}", audio_url)) {
        Ok(mut file) => {
            file.write_all(&data).unwrap();
            let return_data = serde_json::to_string(&AudioReturnData {
                url: format!(
                    "{}://{}/v1/audio/{}",
                    config.protocol, config.domain, audio_url
                ),
            })
            .unwrap();

            Ok(HttpResponse::Ok()
                .content_type(ContentType::json())
                .body(return_data))
        }
        Err(_) => Err(ErrorInternalServerError("Server failed to make file")),
    }
}

pub async fn fetch_audio(audio_name: web::Path<String>) -> Result<HttpResponse, Error> {
    let audio_blob = fs::read(format!("./audio/{}", audio_name))?;
    Ok(HttpResponse::Ok()
        .content_type("audio/ogg")
        .body(audio_blob))
}
