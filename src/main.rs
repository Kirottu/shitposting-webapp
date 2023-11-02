use std::fs;

use actix::Actor;
use actix_files::Files;
use actix_web::{body::BoxBody, web::Data, App, HttpResponse, HttpServer, Responder};
use serde::{de::Visitor, Deserialize};
use session::SessionManager;

mod player;
mod session;

const VALID_FILETYPES: &[&str] = &["mkv", "mp4", "MP4", "webm"];

#[derive(Deserialize)]
struct Config {
    host: String,
    shitposts: String,
    bind: String,
}

#[derive(Clone)]
pub struct Shitpost {
    title: String,
    url: String,
}

#[derive(Deserialize)]
struct SessionConfig {
    amount: usize,
    session: String,
}

struct RouletteFolders(Vec<String>);

impl<'de> Deserialize<'de> for RouletteFolders {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = RouletteFolders;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("folders")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut folders = Vec::new();

                while let Some(key) = map.next_key::<&str>()? {
                    if key == "folders" {
                        folders.push(map.next_value::<String>()?);
                    }
                }
                Ok(RouletteFolders(folders))
            }
        }
        deserializer.deserialize_identifier(FieldVisitor)
    }
}

struct Html(String);

impl Responder for Html {
    type Body = BoxBody;

    fn respond_to(self, _req: &actix_web::HttpRequest) -> actix_web::HttpResponse<Self::Body> {
        HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(self.0)
    }
}

#[actix_web::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config: Config = ron::de::from_bytes(&fs::read("config.ron").unwrap()).unwrap();

    let config = Data::new(config);
    let bind = config.bind.clone();

    let manager = Data::new(SessionManager::new().start());

    HttpServer::new(move || {
        App::new()
            .service(player::host)
            .service(player::host_submit)
            .service(player::join)
            .service(player::join_submit)
            .service(player::index)
            .service(player::socket)
            .service(Files::new("/static", "./static"))
            .service(Files::new("/shitposts", &config.shitposts))
            .app_data(manager.clone())
            .app_data(config.clone())
    })
    .bind(bind)
    .unwrap()
    .run()
    .await
    .unwrap();
}
