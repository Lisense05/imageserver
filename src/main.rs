use actix_web::{error, web, App, HttpResponse, HttpServer};

use std::{collections::HashMap, fs, io::Read, sync::Mutex, time::Instant};

use serde::Deserialize;

mod version1;

#[derive(Deserialize, Clone)]
pub struct Config {
    ip: String,
    protocol: String,
    domain: String,
    port: u16,
    max_image_size: usize,
    max_audio_size: usize,
    api_key: String,
    rate_limit_max_requests: u32,
    rate_limit_window_seconds: u64,
}

pub struct RateLimitState {
    pub request_count: u32,
    pub window_started_at: Instant,
}

pub struct AppState {
    pub upload_rate_limit_map: Mutex<HashMap<String, RateLimitState>>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    match fs::create_dir("./images") {
        Ok(ok) => ok,
        Err(err) => {
            // We don't want to completely error out just because the file already exists, this would be expected behaviour.
            if err.kind() != std::io::ErrorKind::AlreadyExists {
                panic!(
                    "Failed to create image directory with the following error: {}",
                    err
                )
            }
        }
    };

    match fs::create_dir("./audio") {
        Ok(ok) => ok,
        Err(err) => {
            // We don't want to completely error out just because the file already exists, this would be expected behaviour.
            if err.kind() != std::io::ErrorKind::AlreadyExists {
                panic!(
                    "Failed to create audio directory with the following error: {}",
                    err
                )
            }
        }
    };

    let mut config_file = String::new();
    match fs::File::open("./config.toml") {
        Ok(mut file) => {
            file.read_to_string(&mut config_file).unwrap();
        }
        Err(err) => panic!(
            "Failed to read the config.toml with the following error: {}",
            err
        ),
    }

    let config: Config = toml::from_str(config_file.as_str()).unwrap();
    let movable_config: Config = config.clone();

    HttpServer::new(move || {
        let config_clone = web::Data::new(movable_config.clone());

        let image_config = web::JsonConfig::default()
            .limit(movable_config.max_image_size / 1024)
            .error_handler(|err, _req| {
                // create custom error response
                error::InternalError::from_response(err, HttpResponse::Conflict().finish()).into()
            });

        let audio_config = web::JsonConfig::default()
            .limit(movable_config.max_audio_size / 1024)
            .error_handler(|err, _req| {
                // create custom error response
                error::InternalError::from_response(err, HttpResponse::Conflict().finish()).into()
            });

        App::new()
            .app_data(config_clone)
            .app_data(web::Data::new(AppState {
                upload_rate_limit_map: Mutex::new(HashMap::new()),
            }))
            .service(
                web::scope("/v1")
                    .service(version1::embed_external)
                    .service(
                        web::scope("/image")
                            .app_data(image_config)
                            .route("", web::post().to(version1::upload_image))
                            .route("/{image_name}", web::get().to(version1::fetch_image)),
                    )
                    .service(
                        web::scope("/audio")
                            .app_data(audio_config)
                            .route("", web::post().to(version1::upload_audio))
                            .route("/{audio_name}", web::get().to(version1::fetch_audio)),
                    ),
            )
    })
    .bind((config.ip, config.port))?
    .run()
    .await
}
